//! Checks function bodies against their signatures and builds the typed tree.

mod attributes;
mod bounds;
mod call;
mod closure;
mod coerce;
mod collection;
mod constants;
mod exhaust;
mod exports;
mod expr;
mod global;
mod infer;
mod key;
mod matching;
mod narrow;
mod ops;
mod optional;
mod pattern;
mod record;
mod sendable;
mod stmt;
mod task;
mod traits;
mod value_types;
mod variant;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

pub use coerce::{plan, view_coercion_fn};

use mote_base::{Diagnostic, DiagnosticSink, Id, Interner, Span, Symbol};
use mote_syntax as ast;
use mote_syntax::{ItemKind, ParamKind};

use crate::cx::Cx;
use crate::defs::{Bound, DefId, DefKind, GenericParam, ModuleId, Receiver};
use crate::program::Program;
use crate::resolve::Scope;
use crate::tree::{ArmBody, Block, Body, Condition, Else, ExprKind, Local, LocalKind, StmtKind};
use crate::tree::Stmt as TreeStmt;
use crate::types::{BuiltinTy, ConstArg, ConstVal, GenericArg, TyId, TyKind};

/// Checks every function, test and global of a resolved program.
pub fn check(program: &mut Program, interner: &mut Interner, sink: &mut DiagnosticSink) {
    let ids: Vec<ModuleId> = program.modules.iter().map(|(id, _)| id).collect();
    let asts: Vec<ast::Ast> = ids.iter().map(|&id| std::mem::take(&mut program.modules[id].ast)).collect();
    {
        let mut cx = Cx::new(&asts, program, interner, sink);
        cx.check_all();
    }
    for (id, ast) in ids.into_iter().zip(asts) {
        program.modules[id].ast = ast;
    }
}

/// The state of the body being checked.
pub(crate) struct FnCx {
    pub module: ModuleId,
    pub scope: Scope,
    pub body: Body,
    pub ret: TyId,
    scopes: Vec<HashMap<Symbol, Id<Local>>>,
    /// The narrower types tests have proved for names, one map per scope.
    narrowed: Vec<HashMap<Id<Local>, TyId>>,
    /// Compile-time constant bindings visible per lexical scope (`const x = ...`, `comptime for`).
    pub consts: Vec<HashMap<Symbol, (ConstVal, TyId)>>,
    pub loops: u32,
    /// The type each variable of a call stands for, once known.
    infer: Vec<Option<TyId>>,
    /// The constant each const-generic inference variable stands for, once known.
    pub infer_consts: Vec<Option<ConstArg>>,
    /// The traits each type parameter in scope is bounded by, by position.
    bounds: Vec<Vec<Bound>>,
    /// The lambdas being checked, innermost last.
    frames: Vec<Frame>,
    /// Whether `ret` is still to be taken from the first `return` of a lambda.
    ret_open: bool,
    /// The element type of the `Stream` a generator yields.
    yields: Option<TyId>,
    /// Module-level code, which may assign its module's `var` globals.
    pub top_level: bool,
    /// How many `scope` blocks of this function or `spawn` the code is written inside.
    task_scopes: u32,
    /// How many `scope` and `with` blocks the code being checked is written inside.
    open_cleanups: u32,
}

/// A lambda being checked: where its own scopes begin and the outer locals it uses.
struct Frame {
    base: usize,
    captures: Vec<Id<Local>>,
}

impl FnCx {
    pub fn new(module: ModuleId, scope: Scope, ret: TyId) -> Self {
        Self {
            module,
            scope,
            body: Body::new(),
            ret,
            scopes: vec![HashMap::new()],
            narrowed: vec![HashMap::new()],
            consts: vec![HashMap::new()],
            loops: 0,
            infer: Vec::new(),
            infer_consts: Vec::new(),
            bounds: Vec::new(),
            frames: Vec::new(),
            ret_open: false,
            yields: None,
            top_level: false,
            task_scopes: 0,
            open_cleanups: 0,
        }
    }

    /// Finds a local and notes it as captured by every lambda it lies outside of.
    pub fn lookup(&mut self, name: Symbol) -> Option<Id<Local>> {
        let (depth, local) = self.scopes.iter().enumerate().rev().find_map(|(depth, scope)| scope.get(&name).map(|&local| (depth, local)))?;
        for frame in &mut self.frames {
            if depth < frame.base && !frame.captures.contains(&local) {
                frame.captures.push(local);
            }
        }
        Some(local)
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
        self.narrowed.push(HashMap::new());
        self.consts.push(HashMap::new());
    }

    pub fn pop_scope(&mut self) {
        self.scopes.pop();
        self.narrowed.pop();
        self.consts.pop();
    }

    pub fn define_const(&mut self, name: Symbol, val: ConstVal, ty: TyId) {
        self.consts.last_mut().expect("a body has a scope").insert(name, (val, ty));
    }

    pub fn lookup_const(&self, name: Symbol) -> Option<(ConstVal, TyId)> {
        self.consts.iter().rev().find_map(|scope| scope.get(&name).cloned())
    }

    /// From here to the end of the scope the local has the narrower type.
    pub fn narrow(&mut self, local: Id<Local>, ty: TyId) {
        self.narrowed.last_mut().expect("a body has a scope").insert(local, ty);
    }

    pub fn narrowed_ty(&self, local: Id<Local>) -> Option<TyId> {
        self.narrowed.iter().rev().find_map(|scope| scope.get(&local).copied())
    }

    pub fn find(&self, name: Symbol) -> Option<Id<Local>> {
        self.scopes.iter().rev().find_map(|scope| scope.get(&name).copied())
    }

    /// Switches to another module's names with no locals, for a default argument.
    pub fn enter(&mut self, module: ModuleId) -> Entered {
        let saved = Entered {
            module: self.module,
            scope_module: self.scope.module,
            scopes: std::mem::replace(&mut self.scopes, vec![HashMap::new()]),
            narrowed: std::mem::replace(&mut self.narrowed, vec![HashMap::new()]),
            consts: std::mem::replace(&mut self.consts, vec![HashMap::new()]),
        };
        self.module = module;
        self.scope.module = module;
        saved
    }

    pub fn leave(&mut self, saved: Entered) {
        self.module = saved.module;
        self.scope.module = saved.scope_module;
        self.scopes = saved.scopes;
        self.narrowed = saved.narrowed;
        self.consts = saved.consts;
    }
}

/// Whether a block holds a `yield` of its own function, not of a lambda or nested function.
fn block_yields(ast: &ast::Ast, block: &ast::Block) -> bool {
    block.stmts.iter().any(|&stmt| stmt_yields(ast, stmt))
}

fn stmt_yields(ast: &ast::Ast, id: Id<ast::Stmt>) -> bool {
    match &ast[id].kind {
        ast::StmtKind::Yield(_) => true,
        ast::StmtKind::If(branch) | ast::StmtKind::ComptimeIf(branch) => {
            block_yields(ast, &branch.then)
                || match &branch.otherwise {
                    Some(ast::Else::Block(block)) => block_yields(ast, block),
                    Some(ast::Else::If(other)) => stmt_yields(ast, *other),
                    None => false,
                }
        }
        ast::StmtKind::While { body, .. }
        | ast::StmtKind::For { body, .. }
        | ast::StmtKind::ComptimeFor { body, .. }
        | ast::StmtKind::Scope(body)
        | ast::StmtKind::ComptimeBlock(body)
        | ast::StmtKind::With { body, .. } => block_yields(ast, body),
        ast::StmtKind::Match(matched) => matched.arms.iter().any(|arm| match &arm.body {
            ast::ArmBody::Block(block) => block_yields(ast, block),
            ast::ArmBody::Stmt(stmt) => stmt_yields(ast, *stmt),
        }),
        _ => false,
    }
}

/// Whether a block contains `comptime` control flow or reflection builtins that depend on concrete types.
pub(crate) fn ast_uses_comptime(ast: &ast::Ast, interner: &Interner, block: &ast::Block) -> bool {
    block.stmts.iter().any(|&stmt| stmt_uses_comptime(ast, interner, stmt))
}

fn stmt_uses_comptime(ast: &ast::Ast, interner: &Interner, id: Id<ast::Stmt>) -> bool {
    match &ast[id].kind {
        ast::StmtKind::ComptimeIf(_)
        | ast::StmtKind::ComptimeFor { .. }
        | ast::StmtKind::ComptimeBlock(_)
        | ast::StmtKind::Const(_) => true,
        ast::StmtKind::Binding(b) => {
            b.ty.is_some_and(|t| ty_uses_comptime(ast, interner, t)) || expr_uses_comptime(ast, interner, b.init)
        }
        ast::StmtKind::Assign { target, value, .. } => expr_uses_comptime(ast, interner, *target) || expr_uses_comptime(ast, interner, *value),
        ast::StmtKind::If(branch) => {
            expr_uses_comptime_cond(ast, interner, &branch.condition)
                || ast_uses_comptime(ast, interner, &branch.then)
                || match &branch.otherwise {
                    Some(ast::Else::Block(b)) => ast_uses_comptime(ast, interner, b),
                    Some(ast::Else::If(s)) => stmt_uses_comptime(ast, interner, *s),
                    None => false,
                }
        }
        ast::StmtKind::While { condition, body } => expr_uses_comptime_cond(ast, interner, condition) || ast_uses_comptime(ast, interner, body),
        ast::StmtKind::For { iterable, body, .. } | ast::StmtKind::With { init: iterable, body, .. } => {
            expr_uses_comptime(ast, interner, *iterable) || ast_uses_comptime(ast, interner, body)
        }
        ast::StmtKind::Scope(b) => ast_uses_comptime(ast, interner, b),
        ast::StmtKind::Match(m) => {
            expr_uses_comptime(ast, interner, m.subject)
                || m.arms.iter().any(|arm| {
                    arm.guard.is_some_and(|g| expr_uses_comptime(ast, interner, g))
                        || match &arm.body {
                            ast::ArmBody::Block(b) => ast_uses_comptime(ast, interner, b),
                            ast::ArmBody::Stmt(s) => stmt_uses_comptime(ast, interner, *s),
                        }
                })
        }
        ast::StmtKind::Return(e) | ast::StmtKind::Break(e) => e.is_some_and(|e| expr_uses_comptime(ast, interner, e)),
        ast::StmtKind::Yield(e) | ast::StmtKind::Expr(e) => expr_uses_comptime(ast, interner, *e),
        _ => false,
    }
}

fn expr_uses_comptime_cond(ast: &ast::Ast, interner: &Interner, cond: &ast::Condition) -> bool {
    match cond {
        ast::Condition::Expr(e) => expr_uses_comptime(ast, interner, *e),
        ast::Condition::Let { value, .. } => expr_uses_comptime(ast, interner, *value),
    }
}

fn is_comptime_builtin(name: &str) -> bool {
    matches!(
        name,
        "type_info"
            | "typeof"
            | "type_name"
            | "size_of"
            | "align_of"
            | "is_flat"
            | "array_len"
            | "fields"
            | "variants"
            | "has_field"
            | "field"
            | "set_field"
            | "construct"
            | "compile_error"
            | "compile_assert"
            | "__default"
            | "__construct_variant"
            | "__with_variant_field"
            | "__with_field"
            | "__duplicate"
            | "__variant_fields"
            | "__variant_is"
            | "__variant_field"
    )
}

fn ty_uses_comptime(ast: &ast::Ast, interner: &Interner, id: Id<ast::Type>) -> bool {
    match &ast[id].kind {
        ast::TypeKind::Builtin { .. } => true,
        ast::TypeKind::Nullable(inner) => ty_uses_comptime(ast, interner, *inner),
        ast::TypeKind::Tuple(els) | ast::TypeKind::Union(els) => els.iter().any(|&t| ty_uses_comptime(ast, interner, t)),
        ast::TypeKind::Array { element, .. } => ty_uses_comptime(ast, interner, *element),
        ast::TypeKind::Generic { args, .. } => args.iter().any(|g| match g {
            ast::GenericArg::Type(t) => ty_uses_comptime(ast, interner, *t),
            ast::GenericArg::Expr(e) => expr_uses_comptime(ast, interner, *e),
        }),
        ast::TypeKind::Function { params, ret, .. } => {
            params.iter().any(|p| ty_uses_comptime(ast, interner, p.ty)) || ty_uses_comptime(ast, interner, *ret)
        }
        _ => false,
    }
}

fn expr_uses_comptime(ast: &ast::Ast, interner: &Interner, id: Id<ast::Expr>) -> bool {
    match &ast[id].kind {
        ast::ExprKind::Comptime(_) | ast::ExprKind::ComptimeIf { .. } => true,
        ast::ExprKind::Builtin { name, generic_args, args } => {
            is_comptime_builtin(interner.resolve(name.name))
                || generic_args.iter().any(|g| match g {
                    ast::GenericArg::Type(t) => ty_uses_comptime(ast, interner, *t),
                    ast::GenericArg::Expr(e) => expr_uses_comptime(ast, interner, *e),
                })
                || args.iter().any(|a| expr_uses_comptime(ast, interner, a.value))
        }
        ast::ExprKind::Paren(e) | ast::ExprKind::Unary { operand: e, .. } | ast::ExprKind::Unwrap(e) | ast::ExprKind::Propagate(e)
        | ast::ExprKind::Cast { expr: e, .. } => expr_uses_comptime(ast, interner, *e),
        ast::ExprKind::TypeTest { expr: e, ty } => {
            expr_uses_comptime(ast, interner, *e) || ty_uses_comptime(ast, interner, *ty)
        }
        ast::ExprKind::TypeApply { args, .. } => args.iter().any(|g| match g {
            ast::GenericArg::Type(t) => ty_uses_comptime(ast, interner, *t),
            ast::GenericArg::Expr(e) => expr_uses_comptime(ast, interner, *e),
        }),
        ast::ExprKind::Binary { left, right, .. } | ast::ExprKind::Range { start: left, end: right, .. }
        | ast::ExprKind::Index { object: left, index: right } | ast::ExprKind::ArrayRepeat { value: left, len: right } => {
            expr_uses_comptime(ast, interner, *left) || expr_uses_comptime(ast, interner, *right)
        }
        ast::ExprKind::Tuple(els) | ast::ExprKind::List(els) => els.iter().any(|&e| expr_uses_comptime(ast, interner, e)),
        ast::ExprKind::Call { callee, args } => {
            expr_uses_comptime(ast, interner, *callee) || args.iter().any(|a| expr_uses_comptime(ast, interner, a.value))
        }
        ast::ExprKind::Member { object, args, .. } => {
            expr_uses_comptime(ast, interner, *object)
                || args.iter().any(|g| match g {
                    ast::GenericArg::Type(t) => ty_uses_comptime(ast, interner, *t),
                    ast::GenericArg::Expr(e) => expr_uses_comptime(ast, interner, *e),
                })
        }
        ast::ExprKind::OptionalMember { object, .. } | ast::ExprKind::TupleIndex { object, .. } => {
            expr_uses_comptime(ast, interner, *object)
        }
        ast::ExprKind::If { condition, then, otherwise } => {
            expr_uses_comptime_cond(ast, interner, condition)
                || expr_uses_comptime(ast, interner, *then)
                || match &otherwise {
                    ast::ElseExpr::If(e) | ast::ElseExpr::Value(e) => expr_uses_comptime(ast, interner, *e),
                }
        }
        _ => false,
    }
}

/// What `enter` set aside.
pub(crate) struct Entered {
    module: ModuleId,
    scope_module: ModuleId,
    scopes: Vec<HashMap<Symbol, Id<Local>>>,
    narrowed: Vec<HashMap<Id<Local>, TyId>>,
    consts: Vec<HashMap<Symbol, (ConstVal, TyId)>>,
}

impl Cx<'_> {
    pub(crate) fn check_all(&mut self) {
        self.check_attribute_uses();
        self.check_listed_traits();
        self.check_enum_constants();
        self.check_public_signatures();
        self.check_declared_bounds();
        let defs: Vec<DefId> = self.program.defs.iter().map(|(id, _)| id).collect();
        self.check_structs();
        for def in defs {
            match &self.program.defs[def].kind {
                DefKind::Function(function) if function.has_body && function.link.is_none() => self.check_function(def),
                DefKind::Global(_) => {
                    self.global_type(def);
                }
                DefKind::Test => self.check_test(def),
                _ => {}
            }
        }
        self.check_top_level();
        self.check_handle_types();
    }

    pub(crate) fn show_ty(&self, ty: TyId) -> String {
        self.program.render(ty, &*self.interner)
    }

    /// Binds a local in the innermost scope; a second binding of a name there is an error.
    pub(crate) fn declare(&mut self, f: &mut FnCx, local: Id<Local>) {
        let name = f.body.locals[local].name;
        let span = f.body.locals[local].span;
        let first = f.scopes.last().and_then(|scope| scope.get(&name)).copied();
        if let Some(first) = first {
            let message = format!("`{}` is defined twice", self.text(name));
            let first = f.body.locals[first].span;
            self.sink.emit(Diagnostic::error(message, span).label(first, "first defined here"));
            return;
        }
        f.scopes.last_mut().expect("a body has a scope").insert(name, local);
    }

    pub(crate) fn def_has_generics(&self, def: DefId) -> bool {
        let item = &self.program.defs[def];
        let own = item.function().is_some_and(|f| !f.generics.is_empty());
        let owner = item.owner.is_some_and(|o| match &self.program.defs[o].kind {
            DefKind::Record(r) => !r.generics.is_empty(),
            DefKind::Enum(e) => !e.generics.is_empty(),
            DefKind::Trait(_) => true,
            _ => false,
        });
        let extend = item.extend.is_some_and(|e| !self.program.extends[e].generics.is_empty());
        own || owner || extend
    }

    fn check_function(&mut self, def: DefId) {
        let Some(signature) = self.fn_ast(def) else { return };
        let Some(body) = signature.body else { return };
        let module = self.program.defs[def].module;
        // Generic functions with comptime constructs (`comptime if`/`for`, `@fields`, etc.) are
        // typechecked per concrete instantiation so each unrolled branch gets concrete types.
        if self.def_has_generics(def) && ast_uses_comptime(self.ast(module), self.interner, body) {
            return;
        }
        let body_out = self.check_function_body(def, None);
        if let Some(body_out) = body_out {
            self.program.checked.insert(def, body_out);
        }
    }

    /// Checks a function either generically (`inst_args: None`) or instantiated at concrete `inst_args`.
    pub(crate) fn check_function_body(&mut self, def: DefId, inst_args: Option<&[GenericArg]>) -> Option<Body> {
        let signature = self.fn_ast(def)?;
        let body = signature.body?;
        let (module, receiver, raw_ret, raw_params, span) = {
            let item = &self.program.defs[def];
            let function = item.function().expect("a function");
            (item.module, function.receiver, function.ret, function.params.clone(), item.span)
        };
        let mut scope = self.scope_for(def);
        let subst_ty = |types: &mut crate::types::Types, ty: TyId| -> TyId {
            match inst_args {
                Some(args) => types.subst_generic(ty, args),
                None => ty,
            }
        };
        if let Some(args) = inst_args {
            if let Some(st) = scope.self_ty {
                scope.self_ty = Some(self.program.types.subst_generic(st, args));
            }
            for (idx, (&name, &is_const)) in scope.params.clone().iter().zip(scope.const_params.clone().iter()).enumerate() {
                if !is_const
                    && let Some(GenericArg::Type(t)) = args.get(idx)
                {
                    scope.bound.push((name, *t));
                }
            }
        }
        let self_ty = scope.self_ty.unwrap_or(TyId::ERROR);
        let ret = subst_ty(&mut self.program.types, raw_ret);
        let mut f = FnCx::new(module, scope, ret);
        f.bounds = self.param_bounds(def);
        if let Some(args) = inst_args {
            for param_bounds in &mut f.bounds {
                for bound in param_bounds {
                    bound.args = bound.args.iter().map(|&t| self.program.types.subst_generic(t, args)).collect();
                }
            }
        }
        if let Some(args) = inst_args {
            let gen_params = self.all_generic_params(def);
            for (idx, param) in gen_params.iter().enumerate() {
                if let GenericParam::Const { name, ty } = param
                    && let Some(GenericArg::Const(ConstArg::Val(v))) = args.get(idx)
                {
                    f.define_const(*name, v.clone(), *ty);
                }
            }
        }
        if receiver != Receiver::None {
            let name = self.interner.intern("self");
            let local = f.body.local(Local { name, ty: self_ty, is_var: receiver == Receiver::Write, kind: LocalKind::Receiver, span });
            f.body.params.push(local);
            self.declare(&mut f, local);
        }
        let spans = signature.params.iter().filter_map(|param| match &param.kind {
            ParamKind::Named { name, .. } | ParamKind::Variadic { name, .. } => Some(name.span),
            ParamKind::SelfValue { .. } => None,
        });
        for (param, span) in raw_params.iter().zip(spans) {
            let p_ty = subst_ty(&mut self.program.types, param.ty);
            let ty = if param.variadic { self.list_of(p_ty, span) } else { p_ty };
            let local = f.body.local(Local { name: param.name, ty, is_var: param.is_var, kind: LocalKind::Param, span });
            f.body.params.push(local);
            self.declare(&mut f, local);
        }
        let stream = match self.program.types.kind(ret) {
            TyKind::Builtin { kind: BuiltinTy::Stream, args } if block_yields(self.ast(module), body) => Some(args[0]),
            _ => None,
        };
        f.yields = stream;
        f.body.is_generator = stream.is_some();
        let block = self.block_stmts(&mut f, body);
        f.body.block = block;
        if stream.is_none() && ret != TyId::UNIT && ret != TyId::ERROR && !Self::block_diverges(&f.body, &f.body.block, false) {
            self.error(body.span, "missing return");
        }
        Some(f.body)
    }

    pub(crate) fn all_generic_params(&self, def: DefId) -> Vec<GenericParam> {
        let mut out = Vec::new();
        let item = &self.program.defs[def];
        if let Some(ext) = item.extend {
            out.extend(self.program.extends[ext].generics.clone());
        } else if let Some(owner) = item.owner {
            match &self.program.defs[owner].kind {
                DefKind::Trait(t) => {
                    out.push(GenericParam::Type { name: self.self_name, bounds: Vec::new() });
                    out.extend(t.generics.clone());
                }
                DefKind::Record(r) => out.extend(r.generics.clone()),
                DefKind::Enum(e) => out.extend(e.generics.clone()),
                _ => {}
            }
        }
        if let Some(f) = item.function() {
            out.extend(f.generics.clone());
        }
        out
    }

    fn check_test(&mut self, def: DefId) {
        let Some((item, None)) = self.item_of(def) else { return };
        let ItemKind::Test(test) = &item.kind else { return };
        let module = self.program.defs[def].module;
        let scope = Scope::empty(module);
        let mut f = FnCx::new(module, scope, TyId::UNIT);
        let block = self.block_stmts(&mut f, &test.body);
        f.body.block = block;
        self.program.checked.insert(def, f.body);
    }

    /// Whether every path through the block ends in a `return`, or with `leaves` in a `break` or `continue` too.
    pub(crate) fn block_diverges(body: &Body, block: &Block, leaves: bool) -> bool {
        block.stmts.iter().any(|&stmt| Self::stmt_diverges(body, stmt, leaves))
    }

    pub(crate) fn stmt_diverges(body: &Body, stmt: Id<TreeStmt>, leaves: bool) -> bool {
        match &body.stmts[stmt].kind {
            StmtKind::Return(_) => true,
            StmtKind::Break | StmtKind::Continue => leaves,
            StmtKind::If(branch) => match &branch.otherwise {
                Some(Else::Block(block)) => Self::block_diverges(body, &branch.then, leaves) && Self::block_diverges(body, block, leaves),
                Some(Else::If(other)) => Self::block_diverges(body, &branch.then, leaves) && Self::stmt_diverges(body, *other, leaves),
                None => false,
            },
            StmtKind::While { condition: Condition::Expr(condition), body: inner } => {
                matches!(body.exprs[*condition].kind, ExprKind::Bool(true)) && !Self::block_breaks(body, inner)
            }
            StmtKind::Scope(block) | StmtKind::With { body: block, .. } => Self::block_diverges(body, block, leaves),
            StmtKind::Match(matched) => !matched.arms.is_empty() && matched.arms.iter().all(|arm| matches!(&arm.body, ArmBody::Block(block) if Self::block_diverges(body, block, leaves))),
            _ => false,
        }
    }

    /// Whether a `break` in the block leaves the loop around it.
    fn block_breaks(body: &Body, block: &Block) -> bool {
        block.stmts.iter().any(|&stmt| match &body.stmts[stmt].kind {
            StmtKind::Break => true,
            StmtKind::If(branch) => {
                Self::block_breaks(body, &branch.then)
                    || match &branch.otherwise {
                        Some(Else::Block(block)) => Self::block_breaks(body, block),
                        Some(Else::If(other)) => Self::block_breaks(body, &Block { stmts: vec![*other], span: Span::default() }),
                        None => false,
                    }
            }
            StmtKind::Scope(block) | StmtKind::With { body: block, .. } => Self::block_breaks(body, block),
            StmtKind::Match(matched) => matched.arms.iter().any(|arm| matches!(&arm.body, ArmBody::Block(block) if Self::block_breaks(body, block))),
            _ => false,
        })
    }
}
