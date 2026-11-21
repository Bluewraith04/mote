use mote_base::{Id, Span};
use mote_semantics::{BinaryOp, Block, Expr, ExprKind, Stmt, StmtKind, TyKind};

use super::Lowering;
use crate::ir::{Const, Op, Operand, Terminator};

impl Lowering<'_> {
    pub(super) fn block(&mut self, block: &Block) {
        for &stmt in &block.stmts {
            self.stmt(stmt);
        }
    }

    /// Starts a fresh block when the current one already ends, so code after a `return` has a place.
    fn open(&mut self) {
        if self.b.is_terminated() {
            let block = self.b.block();
            self.b.switch_to(block);
        }
    }

    pub(super) fn stmt(&mut self, id: Id<Stmt>) {
        self.open();
        let body = self.body;
        let stmt = &body.stmts[id];
        match &stmt.kind {
            StmtKind::Let { pattern, init } => self.let_binding(*pattern, *init, stmt.span),
            StmtKind::Const { local, init } => {
                let reg = self.local_reg(*local);
                let value = self.expr(*init);
                self.b.emit(Some(reg), Op::Copy(value), stmt.span);
            }
            StmtKind::Assign { target, op, value } => self.assign(*target, *op, *value, stmt.span),
            StmtKind::Return(value) => self.return_value(*value),
            StmtKind::Expr(expr) => {
                self.expr(*expr);
            }
            StmtKind::Error => {}
            StmtKind::If(item) => self.if_stmt(item),
            StmtKind::While { condition, body } => self.while_stmt(condition, body),
            StmtKind::For(item) => self.for_stmt(item, stmt.span),
            StmtKind::ComptimeIf(_) | StmtKind::ComptimeFor { .. } | StmtKind::ComptimeBlock(_) => {
                self.unsupported(stmt.span, "unlowered comptime statement");
            }
            StmtKind::Break => self.break_stmt(),
            StmtKind::Continue => self.continue_stmt(),
            StmtKind::Match(item) => drop(self.match_arms(item, mote_semantics::TyId::UNIT, stmt.span)),
            StmtKind::Scope(block) => self.scope_stmt(block, stmt.span),
            StmtKind::With { local, init, body, close } => self.with_stmt(*local, *init, body, close, stmt.span),
            StmtKind::Yield(value) => {
                let value = self.expr(*value);
                self.b.emit(None, Op::Yield(value), stmt.span);
            }
        }
    }

    fn let_binding(&mut self, pattern: Id<mote_semantics::Pattern>, init: Id<Expr>, span: Span) {
        if let Some(local) = super::self_capture(self.body, pattern, init) {
            let cell = self.local_reg(local);
            self.b.emit(Some(cell), Op::NewCell(Operand::Const(Const::Null)), span);
            let value = self.expr(init);
            self.b.emit(None, Op::StoreCell { cell: Operand::Reg(cell), value }, span);
            return;
        }
        let value = self.expr(init);
        self.bind(pattern, value, span);
    }

    fn assign(&mut self, target: Id<Expr>, op: Option<BinaryOp>, value: Id<Expr>, span: Span) {
        let body = self.body;
        if matches!(body.exprs[target].kind, ExprKind::Slice { .. }) {
            if op.is_some() {
                self.unsupported(span, "compound slice assignment");
            } else {
                self.assign_slice(target, value, span);
            }
            return;
        }
        let Some(place) = self.place(target) else { return };
        match op {
            None => {
                let value = self.expr(value);
                self.write(&place, value, span);
            }
            Some(BinaryOp::Coalesce) => self.coalesce_assign(&place, value, span),
            Some(op) => {
                let current = self.read(&place, span);
                let rhs_ty = self.body.exprs[value].ty;
                let rhs = self.expr(value);
                let result = if matches!(self.program.types.kind(place.ty), TyKind::FixedArray(..)) {
                    self.fixed_array_binary(place.ty, op, current, place.ty, rhs, rhs_ty, span)
                } else {
                    self.b.value(place.ty, Op::Binary { op, left: current, right: rhs }, span)
                };
                self.write(&place, result, span);
            }
        }
    }

    fn return_value(&mut self, value: Option<Id<Expr>>) {
        let value = value.map(|value| self.expr(value));
        self.emit_return(value);
    }

    /// Ends the current block by returning from the function.
    pub(super) fn emit_return(&mut self, value: Option<Operand>) {
        self.return_from(value, self.cleanups.len());
    }

    /// Returns after undoing the cleanups below `top`.
    pub(super) fn return_from(&mut self, value: Option<Operand>, top: usize) {
        let value = if self.ret == mote_semantics::TyId::UNIT { None } else { value };
        let value = match value {
            Some(value) if self.detach_result => Some(self.detach(value, self.ret, Span::default())),
            other => other,
        };
        self.run_cleanups_between(0, top, Span::default());
        self.emit_cell_params(Span::default());
        self.b.terminate(Terminator::Return(value));
    }
}
