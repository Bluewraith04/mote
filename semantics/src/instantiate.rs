//! Gives each function a program needs, at concrete type arguments, a typed body of its own.

use std::collections::{HashSet, VecDeque};

use mote_base::{Diagnostic, DiagnosticSink, Id, Interner, Span};
use mote_syntax as ast;

use crate::comptime;
use crate::cx::Cx;
use crate::defs::{DefId, DefKind, ModuleId};
use crate::program::{Instance, Program};
use crate::tree::{BinaryOp, Body, Callee, Conversion, Expr, ExprKind, FnRef, PatternKind, StmtKind, UnaryOp};
use crate::types::{BuiltinTy, GenericArg, TyId, TyKind};
use crate::walk::reach;

/// Type arguments nest no deeper than this; a function that grows its own arguments stops here.
const MAX_DEPTH: usize = 32;

/// Adds a body to `program.bodies` for each entry and every instance reachable from it, from a program with no errors.
pub fn instantiate(program: &mut Program, interner: &mut Interner, sink: &mut DiagnosticSink, entries: &[Instance]) {
    let globals: Vec<DefId> = program.defs.iter().map(|(id, _)| id).filter(|def| program.global_inits.contains_key(def)).collect();
    for &def in &globals {
        if let Some(mut body) = program.global_inits.remove(&def) {
            comptime::lower_body(program, interner, sink, &mut body);
            program.global_inits.insert(def, body);
        }
    }
    let modules: Vec<_> = program.modules.iter().map(|(id, _)| id).filter(|module| program.top_level.contains_key(module)).collect();
    for &module in &modules {
        if let Some(mut body) = program.top_level.remove(&module) {
            comptime::lower_body(program, interner, sink, &mut body);
            program.top_level.insert(module, body);
        }
    }

    let module_ids: Vec<ModuleId> = program.modules.iter().map(|(id, _)| id).collect();
    let asts: Vec<ast::Ast> = module_ids.iter().map(|&id| std::mem::take(&mut program.modules[id].ast)).collect();
    {
        let mut work = Work { program, interner, sink, asts: &asts, queue: VecDeque::new(), asked: HashSet::new() };
        for entry in entries {
            let span = work.program.defs[entry.def].span;
            work.need(entry.clone(), span);
        }
        for def in globals {
            for (instance, span) in references(work.program, work.interner, &work.program.global_inits[&def]) {
                work.need(instance, span);
            }
        }
        for module in modules {
            for (instance, span) in references(work.program, work.interner, &work.program.top_level[&module]) {
                work.need(instance, span);
            }
        }
        work.listing_checks();
        while let Some(instance) = work.queue.pop_front() {
            work.build(instance);
        }
    }
    for (id, ast) in module_ids.into_iter().zip(asts) {
        program.modules[id].ast = ast;
    }
}

struct Work<'a> {
    program: &'a mut Program,
    interner: &'a mut Interner,
    sink: &'a mut DiagnosticSink,
    asts: &'a [ast::Ast],
    queue: VecDeque<Instance>,
    asked: HashSet<Instance>,
}

impl Work<'_> {
    /// Queues an instance that has a checked or comptime-deferred body and none yet.
    fn need(&mut self, instance: Instance, span: Span) {
        if self.program.bodies.contains_key(&instance) || !self.asked.insert(instance.clone()) {
            return;
        }
        if instance.args.iter().any(|arg| matches!(arg, GenericArg::Type(ty) if self.program.types.depth(*ty) > MAX_DEPTH)) {
            self.sink.emit(Diagnostic::error("type arguments nest too deeply", span));
            return;
        }
        let has_body = self.program.checked.contains_key(&instance.def)
            || self.program.defs[instance.def].function().is_some_and(|f| f.has_body && f.link.is_none());
        if has_body {
            self.queue.push_back(instance);
        }
    }

    /// Queues each default method that a type without parameters got from a trait it lists and whose body calls `@compile_error`/`@compile_assert`, so the listing is checked even when the method is never called.
    fn listing_checks(&mut self) {
        let mut copies: Vec<DefId> = self.program.listed_defaults.keys().copied().collect();
        copies.sort_by_key(|copy| copy.index());
        for copy in copies {
            let (Some(owner), Some(function)) = (self.program.defs[copy].owner, self.program.defs[copy].function()) else { continue };
            let owner_generic = match &self.program.defs[owner].kind {
                DefKind::Record(record) => !record.generics.is_empty(),
                DefKind::Enum(item) => !item.generics.is_empty(),
                _ => true,
            };
            let trait_generic = !matches!(&self.program.defs[self.program.listed_defaults[&copy]].kind, DefKind::Trait(item) if item.generics.is_empty());
            if owner_generic || trait_generic || !function.generics.is_empty() {
                continue;
            }
            let Some(body) = self.program.checked.get(&copy) else { continue };
            if self.has_compile_check(body) || self.ast_has_compile_check(copy) {
                let span = self.program.defs[owner].span;
                self.need(Instance { def: copy, args: Vec::new() }, span);
            }
        }
    }

    fn ast_has_compile_check(&self, def: DefId) -> bool {
        let item = &self.program.defs[def];
        let Some(origin) = &item.origin else { return false };
        let Some(member_idx) = origin.member else { return false };
        let ast = &self.asts[item.module.index()];
        let body_span = match &ast[origin.item].kind {
            ast::ItemKind::Trait(tr) => tr.members.get(member_idx as usize).and_then(|m| m.body.as_ref()).map(|b| b.span),
            _ => None,
        };
        let Some(span) = body_span else { return false };
        ast.exprs.iter().any(|(_, expr)| {
            expr.span.start >= span.start
                && expr.span.end <= span.end
                && matches!(
                    &expr.kind,
                    ast::ExprKind::Builtin { name, .. }
                        if matches!(self.interner.resolve(name.name), "compile_error" | "compile_assert")
                )
        })
    }

    fn has_compile_check(&self, body: &Body) -> bool {
        reach(body).exprs.into_iter().any(|id| match &body.exprs[id].kind {
            ExprKind::Call { callee: Callee::Builtin { name, .. }, .. } => {
                matches!(self.interner.resolve(*name), "compile_error" | "compile_assert")
            }
            _ => false,
        })
    }

    fn build(&mut self, instance: Instance) {
        let mut body = if let Some(checked) = self.program.checked.get(&instance.def) {
            let mut body = checked.clone();
            self.rewrite(&mut body, &instance.args);
            body
        } else {
            let mut cx = Cx::new(self.asts, self.program, self.interner, self.sink);
            let Some(body) = cx.check_function_body(instance.def, Some(&instance.args)) else { return };
            body
        };
        {
            let mut cx = Cx::new(self.asts, self.program, self.interner, self.sink);
            comptime::normalize_body_consts(&mut cx, &mut body);
            if let Some(declared_ret) = cx.program.defs[instance.def].function().map(|function| function.ret) {
                let raw_ret = cx.program.types.subst_generic(declared_ret, &instance.args);
                let norm_ret = comptime::normalize_ty_consts(&mut cx, raw_ret);
                cx.program.instance_returns.insert(instance.clone(), norm_ret);
            }
        }
        self.resolve_post(&mut body);
        comptime::lower_body(self.program, self.interner, self.sink, &mut body);
        self.print_displays(&mut body);
        self.equality_dunders(&mut body);
        let found = references(self.program, self.interner, &body);
        self.program.add_body(instance, body);
        for (next, span) in found {
            self.need(next, span);
        }
    }

    /// Writes the type/const arguments into every type of the body and substitutes const params.
    fn rewrite(&mut self, body: &mut Body, args: &[GenericArg]) {
        let types = &mut self.program.types;
        let locals: Vec<_> = body.locals.iter().map(|(id, _)| id).collect();
        for id in locals {
            body.locals[id].ty = types.subst_generic(body.locals[id].ty, args);
        }
        let exprs: Vec<_> = body.exprs.iter().map(|(id, _)| id).collect();
        for id in exprs {
            let expr = &mut body.exprs[id];
            expr.ty = types.subst_generic(expr.ty, args);
            match &mut expr.kind {
                ExprKind::Is { ty, .. } => *ty = types.subst_generic(*ty, args),
                ExprKind::ConstArg(arg) => *arg = types.subst_const(arg, args),
                ExprKind::Function(function) | ExprKind::Call { callee: Callee::Function(function), .. } => {
                    function.args = function.args.iter().map(|arg| subst_generic_arg(types, arg, args)).collect();
                }
                ExprKind::Call { callee: Callee::TraitMethod { self_ty, args: own, .. }, .. } => {
                    *self_ty = types.subst_generic(*self_ty, args);
                    *own = own.iter().map(|arg| subst_generic_arg(types, arg, args)).collect();
                }
                ExprKind::Call { callee: Callee::Builtin { generic_args, .. }, .. } => {
                    *generic_args = generic_args.iter().map(|arg| subst_generic_arg(types, arg, args)).collect();
                }
                _ => {}
            }
        }
        let patterns: Vec<_> = body.patterns.iter().map(|(id, _)| id).collect();
        for id in patterns {
            let pattern = &mut body.patterns[id];
            pattern.ty = types.subst_generic(pattern.ty, args);
            if let PatternKind::Type { ty, .. } = &mut pattern.kind {
                *ty = types.subst_generic(*ty, args);
            }
        }
    }

    /// Resolves deferred `with` close methods and trait calls after concrete types are known.
    fn resolve_post(&mut self, body: &mut Body) {
        let stmts: Vec<_> = body.stmts.iter().map(|(id, _)| id).collect();
        for id in stmts {
            let StmtKind::With { local, close: None, .. } = body.stmts[id].kind else { continue };
            let ty = body.locals[local].ty;
            match self.close_method(ty) {
                Some(function) => {
                    if let StmtKind::With { close, .. } = &mut body.stmts[id].kind {
                        *close = Some(function);
                    }
                }
                None => {
                    let message = format!("{} has no `close(self)` method", self.program.render(ty, self.interner));
                    self.sink.emit(Diagnostic::error(message, body.stmts[id].span));
                }
            }
        }
        let exprs: Vec<_> = body.exprs.iter().map(|(id, _)| id).collect();
        for id in exprs {
            let ExprKind::Call { callee: Callee::TraitMethod { method, self_ty, args: own }, .. } = &body.exprs[id].kind else {
                continue;
            };
            let (method, self_ty, own) = (*method, *self_ty, own.clone());
            match self.concrete_method(method, self_ty, own) {
                Some(function) => {
                    if let ExprKind::Call { callee, .. } = &mut body.exprs[id].kind {
                        *callee = Callee::Function(function);
                    }
                }
                None => {
                    let message = format!("{} has no method `{}`", self.program.render(self_ty, self.interner), self.interner.resolve(self.program.defs[method].name));
                    self.sink.emit(Diagnostic::error(message, body.exprs[id].span));
                }
            }
        }
    }

    /// `@print(x)` and `@println(x)` of a record or enum with its own `to_string` print `x.to_string()`.
    fn print_displays(&mut self, body: &mut Body) {
        let Some(name) = self.interner.get("to_string") else { return };
        let prints: Vec<_> = body
            .exprs
            .iter()
            .filter_map(|(id, expr)| match &expr.kind {
                ExprKind::Call { callee: Callee::Builtin { name: bname, .. }, args } if args.len() == 1 && self.is_print_builtin(*bname) => Some((id, args[0])),
                _ => None,
            })
            .collect();
        for (call, arg) in prints {
            let Some(method) = self.display_method(body.exprs[arg].ty, name) else { continue };
            let span = body.exprs[arg].span;
            let text = body.expr(ExprKind::Call { callee: Callee::Function(method), args: vec![arg] }, TyId::STRING, span);
            if let ExprKind::Call { callee: Callee::Builtin { generic_args, .. }, args } = &mut body.exprs[call].kind {
                *generic_args = vec![GenericArg::Type(TyId::STRING)];
                args[0] = text;
            }
        }
    }

    fn is_print_builtin(&self, name: mote_base::Symbol) -> bool {
        matches!(self.interner.resolve(name), "print" | "println" | "eprint" | "eprintln")
    }

    /// The `to_string(self) -> String` a record or enum of the program writes, if it has one.
    fn display_method(&self, ty: TyId, name: mote_base::Symbol) -> Option<FnRef> {
        let (owner, _) = self.program.method_owner(ty)?;
        let item = &self.program.defs[owner];
        if !matches!(item.kind, DefKind::Record(_) | DefKind::Enum(_)) {
            return None;
        }
        let (found, args) = self.program.resolve_method(ty, name)?;
        let DefKind::Function(function) = &self.program.defs[found].kind else { return None };
        (function.receiver == crate::defs::Receiver::Read && function.params.is_empty() && function.generics.is_empty() && function.ret == TyId::STRING).then_some(FnRef { def: found, args })
    }

    /// Rewrites remaining `==`/`!=` on concrete types that define `__eq__` or `__ne__` (e.g. monomorphized `T: Eq`).
    fn equality_dunders(&mut self, body: &mut Body) {
        let rewrites: Vec<_> = body
            .exprs
            .iter()
            .filter_map(|(id, expr)| match expr.kind {
                ExprKind::Binary { op: op @ (BinaryOp::Eq | BinaryOp::NotEq), left, right }
                    if expr.ty == TyId::BOOL && body.exprs[left].ty == body.exprs[right].ty =>
                {
                    Some((id, op, left, right, body.exprs[left].ty, expr.span))
                }
                _ => None,
            })
            .collect();
        for (id, op, left, right, ty, span) in rewrites {
            if op == BinaryOp::Eq {
                if let Some(method) = self.equality_method(ty, "__eq__") {
                    body.exprs[id].kind = ExprKind::Call { callee: Callee::Function(method), args: vec![left, right] };
                }
            } else if let Some(method) = self.equality_method(ty, "__ne__") {
                body.exprs[id].kind = ExprKind::Call { callee: Callee::Function(method), args: vec![left, right] };
            } else if let Some(method) = self.equality_method(ty, "__eq__") {
                let eq = body.expr(ExprKind::Call { callee: Callee::Function(method), args: vec![left, right] }, TyId::BOOL, span);
                body.exprs[id].kind = ExprKind::Unary { op: UnaryOp::Not, operand: eq };
            }
        }
    }

    /// The `__eq__` or `__ne__` dunder matching `fn(self, other: Self) -> Bool` on `ty`.
    fn equality_method(&mut self, ty: TyId, name: &str) -> Option<FnRef> {
        let sym = self.interner.get(name)?;
        let (found, args) = self.program.resolve_method(ty, sym)?;
        let DefKind::Function(function) = &self.program.defs[found].kind else { return None };
        if function.receiver != crate::defs::Receiver::Read || !function.generics.is_empty() || function.params.len() != 1 || function.params[0].is_var {
            return None;
        }
        let (param, ret) = (function.params[0].ty, function.ret);
        let param_ty = self.program.types.subst_generic(param, &args);
        let ret_ty = self.program.types.subst_generic(ret, &args);
        (param_ty == ty && ret_ty == TyId::BOOL).then_some(FnRef { def: found, args })
    }

    /// The `close` method of a resource type.
    fn close_method(&self, ty: TyId) -> Option<FnRef> {
        let closeable = self.program.std.closeable?;
        let DefKind::Trait(item) = &self.program.defs[closeable].kind else { return None };
        self.concrete_method(*item.methods.first()?, ty, Vec::new())
    }

    /// The method of the type that a trait's method stands for.
    fn concrete_method(&self, method: DefId, self_ty: TyId, own: Vec<GenericArg>) -> Option<FnRef> {
        let (found, mut args) = self.program.resolve_method(self_ty, self.program.defs[method].name)?;
        let trait_args = match self.program.defs[method].owner.map(|owner| &self.program.defs[owner].kind) {
            Some(crate::defs::DefKind::Trait(item)) => item.generics.len(),
            _ => 0,
        };
        args.extend(own.into_iter().skip(trait_args));
        Some(FnRef { def: found, args })
    }
}

fn subst_generic_arg(types: &mut crate::types::Types, arg: &GenericArg, args: &[GenericArg]) -> GenericArg {
    match arg {
        GenericArg::Type(t) => GenericArg::Type(types.subst_generic(*t, args)),
        GenericArg::Const(c) => GenericArg::Const(types.subst_const(c, args)),
    }
}

/// Collects view coercion helpers reached through an `Adapt` conversion.
fn collect_adapt_view_fns(program: &Program, interner: &Interner, from: TyId, to: TyId, out: &mut Vec<FnRef>) {
    match (program.types.kind(from), program.types.kind(to)) {
        (TyKind::Function(source), TyKind::Function(target)) => {
            for (want, have) in target.params.iter().zip(&source.params) {
                collect_plan_view_fns(program, interner, want.ty, have.ty, out);
            }
            if target.ret != TyId::UNIT {
                collect_plan_view_fns(program, interner, source.ret, target.ret, out);
            }
        }
        (TyKind::Builtin { kind: sk, args: s_args }, TyKind::Builtin { kind: tk, args: t_args })
            if sk == tk && matches!(sk, BuiltinTy::Stream | BuiltinTy::Task) =>
        {
            if let (Some(&s), Some(&t)) = (s_args.first(), t_args.first()) {
                collect_plan_view_fns(program, interner, s, t, out);
            }
        }
        _ => {}
    }
}

fn collect_plan_view_fns(program: &Program, interner: &Interner, from: TyId, to: TyId, out: &mut Vec<FnRef>) {
    let Some(steps) = crate::check::plan(program, from, to) else { return };
    let mut cur = from;
    for (conv, step_ty) in steps {
        match conv {
            Conversion::ToView => {
                if let Some(fn_ref) = crate::check::view_coercion_fn(program, interner, cur, step_ty) {
                    out.push(fn_ref);
                }
            }
            Conversion::Adapt => collect_adapt_view_fns(program, interner, cur, step_ty, out),
            _ => {}
        }
        cur = step_ty;
    }
}

/// The std methods an expression runs for the language: indexing a `Map`, a `Map` literal, or adapted view coercions.
fn language_calls(program: &Program, interner: &Interner, body: &Body, id: Id<Expr>) -> Vec<FnRef> {
    let (names, ty): (&[&str], TyId) = match &body.exprs[id].kind {
        ExprKind::Index { object, .. } => (&["get", "set"], body.exprs[*object].ty),
        ExprKind::Map(_) => (&["new", "set"], body.exprs[id].ty),
        ExprKind::Convert { conversion: Conversion::Adapt, expr } => {
            let mut out = Vec::new();
            collect_adapt_view_fns(program, interner, body.exprs[*expr].ty, body.exprs[id].ty, &mut out);
            return out;
        }
        _ => return Vec::new(),
    };
    names.iter().filter_map(|name| program.collection_method(ty, interner.get(name)?)).collect()
}

/// The functions a body calls or names, with where.
fn references(program: &Program, interner: &Interner, body: &Body) -> Vec<(Instance, Span)> {
    let reached = reach(body);
    let language: Vec<(Instance, Span)> = reached
        .exprs
        .iter()
        .flat_map(|&id| language_calls(program, interner, body, id).into_iter().map(move |function| (function, body.exprs[id].span)))
        .collect();
    let called = reached.exprs.into_iter().filter_map(|id| match &body.exprs[id].kind {
        ExprKind::Function(function) | ExprKind::Call { callee: Callee::Function(function), .. } => Some((function.clone(), body.exprs[id].span)),
        _ => None,
    });
    let closed = reached.stmts.into_iter().filter_map(|id| match &body.stmts[id].kind {
        StmtKind::With { close: Some(function), .. } => Some((function.clone(), body.stmts[id].span)),
        _ => None,
    });
    called.chain(closed).chain(language).collect()
}

#[cfg(test)]
mod tests {
    use mote_base::Severity;

    use super::*;
    use crate::check::check;
    use crate::defs::Resolved;
    use crate::resolve::resolve;
    use crate::testing::parse_sources;
    use crate::types::TyKind;

    struct Built {
        program: Program,
        interner: Interner,
        errors: Vec<String>,
    }

    /// Checks the sources, then instantiates `main`.
    fn build(text: &str) -> Built {
        let mut parsed = parse_sources(&[("main", text)]);
        let mut program = resolve(parsed.input, &mut parsed.interner, &mut parsed.sink);
        check(&mut program, &mut parsed.interner, &mut parsed.sink);
        let mut built = Built { program, interner: parsed.interner, errors: Vec::new() };
        let main = built.def("main");
        instantiate(&mut built.program, &mut built.interner, &mut parsed.sink, &[Instance { def: main, args: Vec::new() }]);
        built.errors = parsed.sink.iter().filter(|diagnostic| diagnostic.severity == Severity::Error).map(|diagnostic| diagnostic.message.clone()).collect();
        built
    }

    impl Built {
        fn def(&self, name: &str) -> DefId {
            let module = self.program.modules.iter().find(|(_, module)| module.path.len() == 1 && self.interner.resolve(module.path[0]) == "main").expect("a main module").1;
            let symbol = self.interner.get(name).expect("a known name");
            match module.scope.get(&symbol) {
                Some(Resolved::Def(def)) => *def,
                other => panic!("{name} is {other:?}"),
            }
        }

        /// The instances in order, as `name<args>`.
        fn order(&self) -> Vec<String> {
            self.program
                .order
                .iter()
                .map(|instance| {
                    let name = self.interner.resolve(self.program.defs[instance.def].name).to_string();
                    if instance.args.is_empty() {
                        name
                    } else {
                        let args: Vec<String> = instance
                            .args
                            .iter()
                            .map(|arg| match arg {
                                GenericArg::Type(ty) => self.program.render(*ty, &self.interner),
                                GenericArg::Const(c) => self.program.render_const(c, &self.interner),
                            })
                            .collect();
                        format!("{name}<{}>", args.join(","))
                    }
                })
                .collect()
        }

        /// Whether any node of any body still has a type parameter or variable.
        fn open(&self) -> bool {
            self.program.bodies.values().any(|body| {
                let types = &self.program.types;
                body.exprs.iter().any(|(_, expr)| types.has_param(expr.ty) || types.has_infer(expr.ty)) || body.locals.iter().any(|(_, local)| types.has_param(local.ty))
            })
        }
    }

    #[test]
    fn each_set_of_type_arguments_gets_one_body() {
        let built = build("fn id<T>(x: T) -> T { return x }\nfn main() {\n let a = id(1)\n let b = id(\"a\")\n let c = id(2)\n}");
        assert!(built.errors.is_empty(), "{:?}", built.errors);
        assert_eq!(built.order(), ["main", "id<Int>", "id<String>"]);
        assert!(!built.open());
        let instance = &built.program.order[1];
        assert_eq!(built.program.bodies[instance].exprs.iter().map(|(_, expr)| expr.ty).collect::<Vec<_>>(), [TyId::INT]);
    }

    #[test]
    fn a_function_nothing_reaches_has_no_body() {
        let built = build("fn unused<T>(x: T) {}\nfn dead() {}\nfn main() {}");
        assert_eq!(built.order(), ["main"]);
    }

    #[test]
    fn a_method_of_a_generic_type_takes_its_owners_arguments() {
        let built = build("struct Box<T> {\n v: T\n pub fn get(self) -> T { return self.v }\n pub fn map<U>(self, f: (T) -> U) -> U { return f(self.v) }\n}\nfn main() {\n let b: Box<Int> = Box { v: 1 }\n let x = b.get()\n let y = b.map(|n| n > 0)\n}");
        assert!(built.errors.is_empty(), "{:?}", built.errors);
        assert_eq!(built.order(), ["main", "get<Int>", "map<Int,Bool>"]);
        assert!(!built.open());
    }

    #[test]
    fn a_call_through_a_bound_reaches_the_types_own_method() {
        let built = build("trait Named { fn name(self) -> String }\nstruct P: (Named) {\n pub fn name(self) -> String { return \"p\" }\n}\nstruct Q: (Named) {\n pub fn name(self) -> String { return \"q\" }\n}\nfn call<T: Named>(x: T) -> String { return x.name() }\nfn main() {\n let a = call(P {})\n let b = call(Q {})\n}");
        assert!(built.errors.is_empty(), "{:?}", built.errors);
        assert_eq!(built.order(), ["main", "call<P>", "call<Q>", "name", "name"]);
        let p = &built.program.order[1];
        let reaches: Vec<_> = built.program.bodies[p].exprs.iter().filter_map(|(_, expr)| if let ExprKind::Call { callee, .. } = &expr.kind { Some(callee.clone()) } else { None }).collect();
        assert!(matches!(reaches.as_slice(), [Callee::Function(function)] if built.program.defs[function.def].owner == Some(built.def("P"))));
    }

    #[test]
    fn a_static_call_on_a_type_parameter_reaches_the_types_own_function() {
        let built = build("trait Make { fn make() -> Self }\nstruct P: (Make) {\n pub fn make() -> P { return P {} }\n}\nstruct Q: (Make) {\n pub fn make() -> Q { return Q {} }\n}\nfn build<T: Make>() -> T { return T.make() }\nfn main() {\n let a: P = build()\n let b: Q = build()\n}");
        assert!(built.errors.is_empty(), "{:?}", built.errors);
        assert_eq!(built.order(), ["main", "build<P>", "build<Q>", "make", "make"]);
        assert!(!built.open());
    }

    #[test]
    fn a_call_through_a_generic_trait_bound_reaches_the_types_own_method() {
        let built = build("trait Convert<T> { fn convert(self) -> T }\nstruct P: (Convert<Int>) {\n pub fn convert(self) -> Int { return 1 }\n}\nstruct Q: (Convert<String>) {\n pub fn convert(self) -> String { return \"q\" }\n}\nfn run<T, U: Convert<T>>(x: U) -> T { return x.convert() }\nfn main() {\n let a: Int = run(P {})\n let b: String = run(Q {})\n}");
        assert!(built.errors.is_empty(), "{:?}", built.errors);
        assert_eq!(built.order(), ["main", "run<Int,P>", "run<String,Q>", "convert", "convert"]);
        assert!(!built.open());
    }

    #[test]
    fn a_recursive_function_has_one_body_per_arguments() {
        let built = build("fn count<T>(x: T, n: Int) {\n if n > 0 { count(x, n - 1) }\n}\nfn main() { count(1, 3) }");
        assert_eq!(built.order(), ["main", "count<Int>"]);
    }

    #[test]
    fn a_function_that_grows_its_own_arguments_is_stopped() {
        let built = build("fn grow<T>(x: T) { grow((x, x)) }\nfn main() { grow(1) }");
        assert_eq!(built.errors, ["type arguments nest too deeply"]);
        assert!(built.program.order.len() < 40);
    }

    #[test]
    fn globals_and_top_level_code_reach_their_callees() {
        let built = build("fn id<T>(x: T) -> T { return x }\nlet g = id(1)\nid(\"a\")\nfn main() {}");
        assert!(built.errors.is_empty(), "{:?}", built.errors);
        let mut order = built.order();
        order.sort();
        assert_eq!(order, ["id<Int>", "id<String>", "main"]);
    }

    #[test]
    fn an_instance_keeps_the_types_of_a_generic_closure_body() {
        let built = build("fn twice<T>(x: T, f: (T) -> T) -> T { return f(f(x)) }\nfn main() { let a = twice(1, |n| n + 1) }");
        assert!(built.errors.is_empty(), "{:?}", built.errors);
        assert!(!built.open());
        let instance = &built.program.order[1];
        let closure_types: Vec<_> = built.program.bodies[instance].exprs.iter().filter(|(_, expr)| matches!(built.program.types.kind(expr.ty), TyKind::Function(_))).collect();
        assert!(!closure_types.is_empty());
    }
}
