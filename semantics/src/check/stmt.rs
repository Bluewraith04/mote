use mote_base::{Id, Span};
use mote_syntax as ast;
use mote_syntax::BindingKind;

use super::call::Args;
use super::pattern::PatternCx;
use super::FnCx;
use crate::comptime;
use crate::cx::Cx;
use crate::defs::Receiver;
use crate::tree::{BinaryOp, Block, Else, Expr, ExprKind, ForKind, ForLoop, IfStmt, LocalKind, Stmt, StmtKind};
use crate::types::{BuiltinTy, ConstVal, TyId, TyKind};

impl Cx<'_> {
    /// Checks a block in a new scope.
    pub(crate) fn block(&mut self, f: &mut FnCx, block: &ast::Block) -> Block {
        f.push_scope();
        let checked = self.block_stmts(f, block);
        f.pop_scope();
        checked
    }

    /// Checks a block's statements in the current scope.
    pub(crate) fn block_stmts(&mut self, f: &mut FnCx, block: &ast::Block) -> Block {
        let mut stmts = Vec::new();
        for &stmt_id in &block.stmts {
            self.stmt_into(f, stmt_id, &mut stmts);
        }
        Block { stmts, span: block.span }
    }

    pub(crate) fn stmt(&mut self, f: &mut FnCx, id: Id<ast::Stmt>) -> Id<Stmt> {
        let span = self.ast(f.module)[id].span;
        let mut stmts = Vec::new();
        self.stmt_into(f, id, &mut stmts);
        match stmts.len() {
            0 => f.body.stmt(StmtKind::Scope(Block { stmts: Vec::new(), span }), span),
            1 => stmts[0],
            _ => f.body.stmt(StmtKind::Scope(Block { stmts, span }), span),
        }
    }

    fn stmt_into(&mut self, f: &mut FnCx, id: Id<ast::Stmt>, out: &mut Vec<Id<Stmt>>) {
        let ast = self.ast(f.module);
        let node = &ast[id];
        let span = node.span;
        match &node.kind.clone() {
            ast::StmtKind::Const(decl) => {
                self.const_stmt(f, decl, span);
                return;
            }
            ast::StmtKind::ComptimeBlock(block) => {
                if let Err(msg) = comptime::eval_comptime_block(self, f, block) {
                    self.error(span, msg);
                }
                return;
            }
            ast::StmtKind::ComptimeIf(branch) => {
                self.comptime_if_stmt(f, branch, span, out);
                return;
            }
            ast::StmtKind::ComptimeFor { pattern, iterable, body } => {
                self.comptime_for_stmt(f, *pattern, *iterable, body, span, out);
                return;
            }
            _ => {}
        }
        let kind = match &self.ast(f.module)[id].kind {
            ast::StmtKind::Binding(binding) => self.binding(f, binding),
            ast::StmtKind::Assign { target, op, value } => self.assign(f, *target, *op, *value, span),
            ast::StmtKind::If(branch) => self.if_stmt(f, branch),
            ast::StmtKind::While { condition, body } => {
                f.push_scope();
                let condition = self.condition(f, condition);
                self.assume(f, &condition, true);
                f.loops += 1;
                let body = self.block(f, body);
                f.loops -= 1;
                f.pop_scope();
                StmtKind::While { condition, body }
            }
            ast::StmtKind::Match(matched) => self.match_stmt(f, matched, span),
            ast::StmtKind::Return(value) => self.return_stmt(f, *value, span),
            ast::StmtKind::Break(None) | ast::StmtKind::Continue if f.loops == 0 => {
                self.error(span, "outside a loop");
                StmtKind::Error
            }
            ast::StmtKind::Break(None) => StmtKind::Break,
            ast::StmtKind::Break(Some(_)) => {
                self.error(span, "`break` takes no value");
                StmtKind::Error
            }
            ast::StmtKind::For { pattern, iterable, body } => self.for_stmt(f, *pattern, *iterable, body),
            ast::StmtKind::Function(function) => self.nested_fn(f, function, span),
            ast::StmtKind::Yield(value) => self.yield_stmt(f, *value, span),
            ast::StmtKind::Continue => StmtKind::Continue,
            ast::StmtKind::Expr(expr) => StmtKind::Expr(self.expr(f, *expr, None)),
            ast::StmtKind::Scope(block) => self.scope_stmt(f, block),
            ast::StmtKind::With { name, init, body } => self.with_stmt(f, *name, *init, body),
            ast::StmtKind::Spawn(body) => StmtKind::Expr(self.spawn_expr(f, body, span)),
            ast::StmtKind::Const(_)
            | ast::StmtKind::ComptimeBlock(_)
            | ast::StmtKind::ComptimeIf(_)
            | ast::StmtKind::ComptimeFor { .. } => unreachable!(),
            ast::StmtKind::Error => StmtKind::Error,
        };
        let stmt = f.body.stmt(kind, span);
        for (local, ty) in self.after_facts(f, stmt) {
            f.narrow(local, ty);
        }
        out.push(stmt);
    }

    fn const_stmt(&mut self, f: &mut FnCx, decl: &ast::ConstDecl, span: Span) {
        let declared_ty = decl.ty.map(|ty| self.written_type(f, ty));
        match comptime::eval_ast_const_in_fn(self, f, decl.value) {
            Ok(mut val) => {
                let val_ty = match (&mut val, declared_ty) {
                    (ConstVal::Int(n, int), Some(d_ty)) => {
                        if let TyKind::Int(target) = self.program.types.kind(d_ty) {
                            if !target.fits(*n) {
                                self.error(span, format!("constant value {n} does not fit in {}", self.show_ty(d_ty)));
                            } else {
                                *int = *target;
                            }
                        }
                        d_ty
                    }
                    (_, Some(d_ty)) => d_ty,
                    (v, None) => crate::comptime::infer_const_ty(&mut self.program.types, v),
                };
                f.define_const(decl.name.name, val, val_ty);
            }
            Err(msg) => self.error(span, msg),
        }
    }

    fn comptime_if_stmt(&mut self, f: &mut FnCx, branch: &ast::If, span: Span, out: &mut Vec<Id<Stmt>>) {
        let ast::Condition::Expr(cond_e) = branch.condition else {
            self.error(span, "`comptime if` does not support pattern conditions");
            return;
        };
        let take = match comptime::eval_ast_const_in_fn(self, f, cond_e) {
            Ok(ConstVal::Bool(b)) => b,
            Ok(_) => {
                self.error(span, "`comptime if` condition must be a Bool");
                return;
            }
            Err(msg) => {
                self.error(span, msg);
                return;
            }
        };
        if take {
            f.push_scope();
            for &s in &branch.then.stmts {
                self.stmt_into(f, s, out);
            }
            f.pop_scope();
        } else if let Some(otherwise) = &branch.otherwise {
            f.push_scope();
            match otherwise {
                ast::Else::If(s) => self.stmt_into(f, *s, out),
                ast::Else::Block(b) => {
                    for &s in &b.stmts {
                        self.stmt_into(f, s, out);
                    }
                }
            }
            f.pop_scope();
        }
    }

    fn comptime_for_stmt(&mut self, f: &mut FnCx, pattern: Id<ast::Pattern>, iterable: Id<ast::Expr>, body: &ast::Block, span: Span, out: &mut Vec<Id<Stmt>>) {
        let items = match comptime::eval_comptime_iterable(self, f, iterable) {
            Ok(items) => items,
            Err(msg) => {
                self.error(span, msg);
                return;
            }
        };
        for (val, elem_ty) in items {
            f.push_scope();
            if let Err(msg) = self.bind_comptime_pattern(f, pattern, &val, elem_ty) {
                self.error(self.ast(f.module)[pattern].span, msg);
                f.pop_scope();
                break;
            }
            for &s in &body.stmts {
                self.stmt_into(f, s, out);
            }
            f.pop_scope();
        }
    }

    fn bind_comptime_pattern(&self, f: &mut FnCx, pattern: Id<ast::Pattern>, val: &ConstVal, ty: TyId) -> Result<(), String> {
        let p = self.ast(f.module)[pattern].clone();
        match &p.kind {
            ast::PatternKind::Wildcard => Ok(()),
            ast::PatternKind::Binding(ident) => {
                f.define_const(ident.name, val.clone(), ty);
                Ok(())
            }
            ast::PatternKind::Tuple(sub) => {
                let ConstVal::Tuple(items) = val else {
                    return Err("cannot destructure non-tuple in `comptime for`".to_string());
                };
                if sub.len() != items.len() {
                    return Err("tuple pattern length mismatch in `comptime for`".to_string());
                }
                for (sp, item) in sub.iter().zip(items) {
                    self.bind_comptime_pattern(f, *sp, item, TyId::ERROR)?;
                }
                Ok(())
            }
            _ => Err("pattern not supported in `comptime for`".to_string()),
        }
    }

    fn binding(&mut self, f: &mut FnCx, binding: &ast::Binding) -> StmtKind {
        let annotation = binding.ty.map(|ty| self.written_type(f, ty));
        let init = match annotation {
            Some(ty) => self.expr_to(f, binding.init, ty),
            None => self.expr(f, binding.init, None),
        };
        let ty = annotation.unwrap_or(f.body.exprs[init].ty);
        let mut pcx = PatternCx::new(binding.kind == BindingKind::Var, LocalKind::Let);
        let pattern = self.pattern(f, &mut pcx, binding.pattern, ty);
        if !self.irrefutable(f, pattern) {
            let span = f.body.patterns[pattern].span;
            self.error(span, "this pattern can fail; use `if let` or `match`");
        }
        StmtKind::Let { pattern, init }
    }

    fn for_stmt(&mut self, f: &mut FnCx, pattern: Id<ast::Pattern>, iterable: Id<ast::Expr>, body: &ast::Block) -> StmtKind {
        let iterable_span = self.ast(f.module)[iterable].span;
        let mut iterable = self.expr(f, iterable, None);
        let found = self.iteration(f, &mut iterable, iterable_span);
        let (kind, element) = found.unwrap_or((ForKind::Indexed, TyId::ERROR));
        f.push_scope();
        let pattern = self.arm_pattern(f, pattern, element);
        if !self.irrefutable(f, pattern) {
            let span = f.body.patterns[pattern].span;
            self.error(span, "this pattern can fail; use `match` inside the loop");
        }
        f.loops += 1;
        let body = self.block(f, body);
        f.loops -= 1;
        f.pop_scope();
        StmtKind::For(ForLoop { pattern, iterable, kind, body })
    }

    /// `xs[a..b] = v`: one value for every slot, or a list as long as the slice.
    #[allow(clippy::too_many_arguments)]
    fn assign_slice(
        &mut self,
        f: &mut FnCx,
        object: Id<ast::Expr>,
        start: Option<Id<ast::Expr>>,
        end: Option<Id<ast::Expr>>,
        inclusive: bool,
        step: Option<Id<ast::Expr>>,
        op: ast::AssignOp,
        value: Id<ast::Expr>,
        target_span: Span,
    ) -> StmtKind {
        let Some((object, ty, start, end, step)) = self.check_slice_parts(f, object, start, end, step, target_span) else {
            return StmtKind::Error;
        };
        let element = match self.program.types.kind(ty) {
            TyKind::Builtin { kind: BuiltinTy::List, args } if op == ast::AssignOp::Assign && step.is_none() => args[0],
            _ => {
                let message = if op != ast::AssignOp::Assign { "a slice is only assigned with `=`" } else { "only a `List` slice without a step is assigned" };
                self.error(target_span, message);
                return StmtKind::Error;
            }
        };
        if !self.writable(f, object) {
            self.error(target_span, "cannot assign: no `var` path to this place");
        }
        let target = f.body.expr(ExprKind::Slice { object, start, end, inclusive, step }, ty, target_span);
        let whole = matches!(self.ast(f.module)[value].kind, ast::ExprKind::List(_));
        let node = self.expr(f, value, Some(if whole { ty } else { element }));
        let node_span = f.body.exprs[node].span;
        let value = if f.body.exprs[node].ty == ty { node } else { self.coerce(f, node, element, node_span) };
        StmtKind::Assign { target, op: None, value }
    }

    fn assign(&mut self, f: &mut FnCx, target: Id<ast::Expr>, op: ast::AssignOp, value: Id<ast::Expr>, span: Span) -> StmtKind {
        let target_span = self.ast(f.module)[target].span;
        if let ast::ExprKind::Index { object, index } = self.ast(f.module)[target].kind {
            return self.assign_index(f, object, index, op, value, target_span, span);
        }
        if let ast::ExprKind::Slice { object, start, end, inclusive, step } = self.ast(f.module)[target].kind {
            return self.assign_slice(f, object, start, end, inclusive, step, op, value, target_span);
        }
        let target = self.expr(f, target, None);
        self.assign_place(f, target, op, value, target_span, span)
    }

    /// Assigns into `ast_obj[ast_idx]`, dispatching built-in collections as places and custom types to `__setitem__`.
    #[allow(clippy::too_many_arguments)]
    fn assign_index(&mut self, f: &mut FnCx, ast_obj: Id<ast::Expr>, ast_idx: Id<ast::Expr>, op: ast::AssignOp, value: Id<ast::Expr>, target_span: Span, span: Span) -> StmtKind {
        let obj = self.expr(f, ast_obj, None);
        let ty = f.body.exprs[obj].ty;
        let builtin = matches!(
            self.program.types.kind(ty),
            TyKind::Error | TyKind::Array(_) | TyKind::FixedArray(..) | TyKind::String | TyKind::Builtin { kind: BuiltinTy::List, .. }
        ) || self.map_args(ty).is_some();
        if builtin {
            let target = self.index_on(f, obj, ast_idx, None, target_span);
            return self.assign_place(f, target, op, value, target_span, span);
        }
        if let Some(name) = self.interner.get("__setitem__")
            && let Some((method, owner_args)) = self.quiet_method(f, ty, name)
        {
            if op != ast::AssignOp::Assign {
                self.error(span, "custom index assignment only supports `=`");
                return StmtKind::Error;
            }
            if self.receiver_of(method) != Receiver::Write {
                self.error(span, "`__setitem__` must take `var self`");
                return StmtKind::Error;
            }
            let args = [ast::Arg { name: None, value: ast_idx }, ast::Arg { name: None, value }];
            let call = self.call_with(f, method, &owner_args, &[], Some(obj), Args::Written(&args), None, span);
            return StmtKind::Expr(call);
        }
        self.error(target_span, format!("{} cannot be indexed", self.show_ty(ty)));
        StmtKind::Error
    }

    fn assign_place(&mut self, f: &mut FnCx, target: Id<Expr>, op: ast::AssignOp, value: Id<ast::Expr>, target_span: Span, span: Span) -> StmtKind {
        let ty = f.body.exprs[target].ty;
        if !Self::is_place(f, target) {
            if ty != TyId::ERROR {
                self.error(target_span, "cannot assign to this expression");
            }
            return StmtKind::Error;
        }
        let writable = self.writable(f, target);
        if !writable {
            self.error(target_span, "cannot assign: no `var` path to this place");
        }
        if let ExprKind::Global(def) = f.body.exprs[target].kind
            && !(f.top_level && self.program.defs[def].module == f.module)
        {
            self.error(target_span, "only the module's own top-level code assigns a global");
        }
        let (tree_op, value) = match op {
            ast::AssignOp::Assign => (None, self.expr_to(f, value, ty)),
            ast::AssignOp::Coalesce => {
                if ty != TyId::ERROR && !matches!(self.program.types.kind(ty), TyKind::Optional(_)) {
                    let message = format!("`??=` needs an optional place, found {}", self.show_ty(ty));
                    self.error(target_span, message);
                    return StmtKind::Error;
                }
                (Some(crate::tree::BinaryOp::Coalesce), self.expr_to(f, value, ty))
            }
            other => {
                let Some(tree_op) = compound_op(other) else { return StmtKind::Error };
                let hint = if Self::is_untyped_literal(self.ast(f.module), value) { self.operand_hint(ty) } else { ty };
                let rhs = self.expr(f, value, Some(hint));
                let rhs_ty = f.body.exprs[rhs].ty;
                if ty == TyId::ERROR || rhs_ty == TyId::ERROR {
                    return StmtKind::Error;
                }
                if self.primitive_binary_type(tree_op, ty, rhs_ty).is_some()
                    || self.fixed_array_binary_type(tree_op, ty, rhs_ty) == Some(ty)
                {
                    return StmtKind::Assign { target, op: Some(tree_op), value: rhs };
                }
                if let Some(dunder) = in_place_dunder(tree_op)
                    && let Some(name) = self.interner.get(dunder)
                    && let Some((method, owner_args)) = self.quiet_method(f, ty, name)
                {
                    if self.receiver_of(method) != Receiver::Write {
                        self.error(span, format!("`{dunder}` must take `var self`"));
                        return StmtKind::Error;
                    }
                    if !writable {
                        return StmtKind::Error;
                    }
                    let call = self.call_with(f, method, &owner_args, &[], Some(target), Args::Checked(&[rhs]), None, span);
                    return StmtKind::Expr(call);
                }
                // Separate node keeps init analysis from treating the RHS read as the LHS write.
                let lhs = f.body.expr(f.body.exprs[target].kind.clone(), ty, target_span);
                let result = self.binary_checked(f, tree_op, lhs, rhs, Some(ty), span);
                let assigned = self.coerce(f, result, ty, span);
                (None, assigned)
            }
        };
        StmtKind::Assign { target, op: tree_op, value }
    }

    fn if_stmt(&mut self, f: &mut FnCx, branch: &ast::If) -> StmtKind {
        f.push_scope();
        let condition = self.condition(f, &branch.condition);
        self.assume(f, &condition, true);
        let then = self.block(f, &branch.then);
        f.pop_scope();
        let otherwise = match &branch.otherwise {
            None => None,
            Some(ast::Else::Block(block)) => {
                f.push_scope();
                self.assume(f, &condition, false);
                let block = self.block(f, block);
                f.pop_scope();
                Some(Else::Block(block))
            }
            Some(ast::Else::If(other)) => {
                f.push_scope();
                self.assume(f, &condition, false);
                let other = self.stmt(f, *other);
                f.pop_scope();
                Some(Else::If(other))
            }
        };
        StmtKind::If(IfStmt { condition, then, otherwise })
    }

    fn yield_stmt(&mut self, f: &mut FnCx, value: Id<ast::Expr>, span: Span) -> StmtKind {
        let Some(element) = f.yields else {
            self.error(span, "`yield` is only in a function that returns a Stream");
            return StmtKind::Error;
        };
        if f.open_cleanups > 0 {
            self.error(span, "a generator cannot yield inside `with` or `scope`");
            return StmtKind::Error;
        }
        StmtKind::Yield(self.expr_to(f, value, element))
    }

    fn return_stmt(&mut self, f: &mut FnCx, value: Option<Id<ast::Expr>>, span: Span) -> StmtKind {
        if f.top_level {
            self.error(span, "`return` is only in a function");
            return StmtKind::Error;
        }
        if f.yields.is_some() {
            if value.is_some() {
                self.error(span, "a generator returns no value");
                return StmtKind::Error;
            }
            return StmtKind::Return(None);
        }
        match value {
            Some(value) if f.ret_open => {
                let node = self.expr(f, value, None);
                f.ret = f.body.exprs[node].ty;
                f.ret_open = false;
                StmtKind::Return(Some(node))
            }
            Some(value) => {
                let ret = f.ret;
                StmtKind::Return(Some(self.expr_to(f, value, ret)))
            }
            None if f.ret_open => {
                f.ret = TyId::UNIT;
                f.ret_open = false;
                StmtKind::Return(None)
            }
            None => {
                if f.ret != TyId::UNIT && f.ret != TyId::ERROR {
                    let message = format!("return needs a {} value", self.show_ty(f.ret));
                    self.error(span, message);
                }
                StmtKind::Return(None)
            }
        }
    }
}

fn compound_op(op: ast::AssignOp) -> Option<crate::tree::BinaryOp> {
    use crate::tree::BinaryOp;
    Some(match op {
        ast::AssignOp::Add => BinaryOp::Add,
        ast::AssignOp::Sub => BinaryOp::Sub,
        ast::AssignOp::Mul => BinaryOp::Mul,
        ast::AssignOp::Div => BinaryOp::Div,
        ast::AssignOp::Rem => BinaryOp::Rem,
        ast::AssignOp::BitAnd => BinaryOp::BitAnd,
        ast::AssignOp::BitOr => BinaryOp::BitOr,
        ast::AssignOp::BitXor => BinaryOp::BitXor,
        ast::AssignOp::Shl => BinaryOp::Shl,
        ast::AssignOp::Shr => BinaryOp::Shr,
        ast::AssignOp::Assign | ast::AssignOp::Coalesce => return None,
    })
}

fn in_place_dunder(op: BinaryOp) -> Option<&'static str> {
    Some(match op {
        BinaryOp::Add => "__iadd__",
        BinaryOp::Sub => "__isub__",
        BinaryOp::Mul => "__imul__",
        BinaryOp::Div => "__idiv__",
        BinaryOp::Rem => "__irem__",
        BinaryOp::BitAnd => "__ibit_and__",
        BinaryOp::BitOr => "__ibit_or__",
        BinaryOp::BitXor => "__ibit_xor__",
        BinaryOp::Shl => "__ishl__",
        BinaryOp::Shr => "__ishr__",
        _ => return None,
    })
}
