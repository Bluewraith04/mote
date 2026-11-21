use mote_base::{Id, Span};
use mote_syntax as ast;
use mote_syntax::BindingKind;

use super::pattern::PatternCx;
use super::FnCx;
use crate::cx::Cx;
use crate::defs::LangItem;
use crate::tree::{Block, Else, Expr, ExprKind, ForKind, ForLoop, IfStmt, LocalKind, Stmt, StmtKind};
use crate::types::{TyId, TyKind};

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
        for &stmt in &block.stmts {
            let stmt = self.stmt(f, stmt);
            for (local, ty) in self.after_facts(f, stmt) {
                f.narrow(local, ty);
            }
            stmts.push(stmt);
        }
        Block { stmts, span: block.span }
    }

    pub(crate) fn stmt(&mut self, f: &mut FnCx, id: Id<ast::Stmt>) -> Id<Stmt> {
        let ast = self.ast(f.module);
        let node = &ast[id];
        let span = node.span;
        let kind = match &node.kind {
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
            ast::StmtKind::Error => StmtKind::Error,
        };
        f.body.stmt(kind, span)
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
        let iterable = self.expr(f, iterable, None);
        let found = self.iteration(f.body.exprs[iterable].ty, iterable_span);
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
    fn assign_slice(&mut self, f: &mut FnCx, target: Id<Expr>, op: ast::AssignOp, value: Id<ast::Expr>, target_span: Span) -> StmtKind {
        let ExprKind::Slice { object, step, .. } = f.body.exprs[target].kind else { return StmtKind::Error };
        let ty = f.body.exprs[target].ty;
        let element = match self.lang_adt(ty) {
            Some((LangItem::List, args)) if op == ast::AssignOp::Assign && step.is_none() => args[0],
            _ => {
                let message = if op != ast::AssignOp::Assign { "a slice is only assigned with `=`" } else { "only a `List` slice without a step is assigned" };
                self.error(target_span, message);
                return StmtKind::Error;
            }
        };
        if !self.writable(f, object) {
            self.error(target_span, "cannot assign: no `var` path to this place");
        }
        let whole = matches!(self.ast(f.module)[value].kind, ast::ExprKind::List(_));
        let node = self.expr(f, value, Some(if whole { ty } else { element }));
        let node_span = f.body.exprs[node].span;
        let value = if f.body.exprs[node].ty == ty { node } else { self.coerce(f, node, element, node_span) };
        StmtKind::Assign { target, op: None, value }
    }

    fn assign(&mut self, f: &mut FnCx, target: Id<ast::Expr>, op: ast::AssignOp, value: Id<ast::Expr>, span: Span) -> StmtKind {
        let target_span = self.ast(f.module)[target].span;
        let target = self.expr(f, target, None);
        if matches!(f.body.exprs[target].kind, ExprKind::Slice { .. }) {
            return self.assign_slice(f, target, op, value, target_span);
        }
        let ty = f.body.exprs[target].ty;
        if !Self::is_place(f, target) {
            if ty != TyId::ERROR {
                self.error(target_span, "cannot assign to this expression");
            }
            return StmtKind::Error;
        }
        if !self.writable(f, target) {
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
                let rhs = self.expr(f, value, Some(ty));
                let rhs_ty = f.body.exprs[rhs].ty;
                if ty != TyId::ERROR && rhs_ty != TyId::ERROR {
                    self.binary_type(tree_op, ty, rhs_ty, span);
                }
                (Some(tree_op), rhs)
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
