use mote_base::{Id, Span};
use mote_semantics::{BinaryOp, Block, Condition, Else, Expr, ExprKind, ForKind, ForLoop, IfStmt, Pattern, TyId, UnaryOp};

use super::place::Place;
use super::Lowering;
use crate::ir::{BlockId, Callee, Const, Op, Operand, Terminator};

impl Lowering<'_> {
    /// Ends the current block with a branch on the condition of an `if` or `while`.
    pub(super) fn condition(&mut self, condition: &Condition, then: BlockId, otherwise: BlockId) {
        match condition {
            Condition::Expr(expr) => self.branch_on(*expr, then, otherwise),
            Condition::Let { pattern, value } => {
                let value = self.expr(*value);
                self.matches(*pattern, value, otherwise);
                self.b.terminate(Terminator::Jump(then));
            }
        }
    }

    /// Ends the current block by jumping to `then` or `otherwise`; `&&`, `||` and `!` become branches.
    pub(super) fn branch_on(&mut self, id: Id<Expr>, then: BlockId, otherwise: BlockId) {
        let body = self.body;
        match &body.exprs[id].kind {
            ExprKind::Binary { op: BinaryOp::And, left, right } => {
                let middle = self.b.block();
                self.branch_on(*left, middle, otherwise);
                self.b.switch_to(middle);
                self.branch_on(*right, then, otherwise);
            }
            ExprKind::Binary { op: BinaryOp::Or, left, right } => {
                let middle = self.b.block();
                self.branch_on(*left, then, middle);
                self.b.switch_to(middle);
                self.branch_on(*right, then, otherwise);
            }
            ExprKind::Unary { op: UnaryOp::Not, operand } => self.branch_on(*operand, otherwise, then),
            _ => {
                let condition = self.expr(id);
                self.b.terminate(Terminator::Branch { condition, then, otherwise });
            }
        }
    }

    /// `a && b` and `a || b` as a value.
    pub(super) fn logic_value(&mut self, id: Id<Expr>) -> Operand {
        let span = self.body.exprs[id].span;
        let result = self.b.reg(TyId::BOOL, None);
        let (yes, no, join) = (self.b.block(), self.b.block(), self.b.block());
        self.branch_on(id, yes, no);
        for (block, value) in [(yes, true), (no, false)] {
            self.b.switch_to(block);
            self.b.emit(Some(result), Op::Copy(Operand::Const(Const::Bool(value))), span);
            self.b.terminate(Terminator::Jump(join));
        }
        self.b.switch_to(join);
        Operand::Reg(result)
    }

    pub(super) fn if_stmt(&mut self, item: &IfStmt) {
        let then = self.b.block();
        let join = self.b.block();
        let otherwise = if item.otherwise.is_some() { self.b.block() } else { join };
        self.condition(&item.condition, then, otherwise);
        self.b.switch_to(then);
        self.block(&item.then);
        self.jump_to(join);
        match &item.otherwise {
            Some(Else::If(stmt)) => {
                self.b.switch_to(otherwise);
                self.stmt(*stmt);
                self.jump_to(join);
            }
            Some(Else::Block(block)) => {
                self.b.switch_to(otherwise);
                self.block(block);
                self.jump_to(join);
            }
            None => {}
        }
        self.b.switch_to(join);
    }

    /// `if c { a } else { b }` as a value.
    pub(super) fn if_value(&mut self, condition: &Condition, then: Id<Expr>, otherwise: Id<Expr>, ty: TyId, span: Span) -> Operand {
        let result = (ty != TyId::UNIT).then(|| self.b.reg(ty, None));
        let (yes, no, join) = (self.b.block(), self.b.block(), self.b.block());
        self.condition(condition, yes, no);
        for (block, branch) in [(yes, then), (no, otherwise)] {
            self.b.switch_to(block);
            let value = self.expr(branch);
            if let Some(result) = result {
                self.b.emit(Some(result), Op::Copy(value), span);
            }
            self.jump_to(join);
        }
        self.b.switch_to(join);
        result.map_or(Operand::Const(Const::Null), Operand::Reg)
    }

    pub(super) fn while_stmt(&mut self, condition: &Condition, body: &Block) {
        let (header, inner, exit) = (self.b.block(), self.b.block(), self.b.block());
        self.b.terminate(Terminator::Jump(header));
        self.b.switch_to(header);
        self.condition(condition, inner, exit);
        self.b.switch_to(inner);
        self.open_loop(exit, header);
        self.block(body);
        self.loops.pop();
        self.jump_to(header);
        self.b.switch_to(exit);
    }

    pub(super) fn for_stmt(&mut self, item: &ForLoop, span: Span) {
        match item.kind {
            ForKind::Range => self.for_range(item, span),
            ForKind::Indexed => self.for_indexed(item, span),
            ForKind::Stream => self.for_next(item, "stream_next", true, span),
            ForKind::Receiver => self.for_next(item, "receiver_recv", false, span),
        }
    }

    /// Opens a loop; a `break` or `continue` ends the scopes and `with`s opened since.
    fn open_loop(&mut self, exit: BlockId, next: BlockId) {
        self.loops.push((exit, next, self.cleanups.len()));
    }

    pub(super) fn break_stmt(&mut self) {
        if let Some(&(exit, _, depth)) = self.loops.last() {
            self.run_cleanups(depth, Span::default());
            self.b.terminate(Terminator::Jump(exit));
        }
    }

    pub(super) fn continue_stmt(&mut self) {
        if let Some(&(_, next, depth)) = self.loops.last() {
            self.run_cleanups(depth, Span::default());
            self.b.terminate(Terminator::Jump(next));
        }
    }

    /// Jumps to the block unless the current one already ends.
    fn jump_to(&mut self, block: BlockId) {
        if !self.b.is_terminated() {
            self.b.terminate(Terminator::Jump(block));
        }
    }

    /// Counts a register from the start of a range, stopping at the end; the range may be written out or held in a value.
    fn for_range(&mut self, item: &ForLoop, span: Span) {
        let body = self.body;
        let element = body.patterns[item.pattern].ty;
        let (start, end, inclusive) = match &body.exprs[item.iterable].kind {
            ExprKind::Range { start, end, inclusive } => {
                let start = self.expr(*start);
                let end = self.expr(*end);
                (start, end, Operand::Const(Const::Bool(*inclusive)))
            }
            _ => {
                let range = self.expr(item.iterable);
                let start = self.b.value(element, Op::Field { object: range, index: 0 }, span);
                let end = self.b.value(element, Op::Field { object: range, index: 1 }, span);
                let inclusive = self.b.value(TyId::BOOL, Op::Field { object: range, index: 2 }, span);
                (start, end, inclusive)
            }
        };
        let counter = self.b.reg(element, None);
        self.b.emit(Some(counter), Op::Copy(start), span);
        let end_reg = self.b.reg(element, None);
        self.b.emit(Some(end_reg), Op::Copy(end), span);
        let (header, inner, step, exit) = (self.b.block(), self.b.block(), self.b.block(), self.b.block());
        self.b.terminate(Terminator::Jump(header));
        self.b.switch_to(header);
        let (counter_op, end_op) = (Operand::Reg(counter), Operand::Reg(end_reg));
        let go = match inclusive {
            Operand::Const(Const::Bool(false)) => self.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::Lt, left: counter_op, right: end_op }, span),
            Operand::Const(Const::Bool(true)) => self.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::LtEq, left: counter_op, right: end_op }, span),
            _ => {
                let less = self.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::Lt, left: counter_op, right: end_op }, span);
                let equal = self.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::Eq, left: counter_op, right: end_op }, span);
                let last = self.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::And, left: inclusive, right: equal }, span);
                self.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::Or, left: less, right: last }, span)
            }
        };
        self.b.terminate(Terminator::Branch { condition: go, then: inner, otherwise: exit });
        self.b.switch_to(inner);
        self.bind(item.pattern, counter_op, span);
        self.open_loop(exit, step);
        self.block(&item.body);
        self.loops.pop();
        self.jump_to(step);
        self.b.switch_to(step);
        if inclusive != Operand::Const(Const::Bool(false)) {
            let equal = self.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::Eq, left: counter_op, right: end_op }, span);
            let stop = match inclusive {
                Operand::Const(Const::Bool(true)) => equal,
                _ => self.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::And, left: inclusive, right: equal }, span),
            };
            let next = self.b.block();
            self.b.terminate(Terminator::Branch { condition: stop, then: exit, otherwise: next });
            self.b.switch_to(next);
        }
        self.b.emit(Some(counter), Op::Binary { op: BinaryOp::Add, left: counter_op, right: Operand::Const(Const::Int(1)) }, span);
        self.b.terminate(Terminator::Jump(header));
        self.b.switch_to(exit);
    }

    /// Walks a list or array by position, reading its length each time round.
    fn for_indexed(&mut self, item: &ForLoop, span: Span) {
        let body = self.body;
        let element = body.patterns[item.pattern].ty;
        let list = self.expr(item.iterable);
        let list_reg = self.b.reg(body.exprs[item.iterable].ty, None);
        self.b.emit(Some(list_reg), Op::Copy(list), span);
        let list = Operand::Reg(list_reg);
        let position = self.b.reg(TyId::INT, None);
        self.b.emit(Some(position), Op::Copy(Operand::Const(Const::Int(0))), span);
        let position_op = Operand::Reg(position);
        let (header, inner, step, exit) = (self.b.block(), self.b.block(), self.b.block(), self.b.block());
        self.b.terminate(Terminator::Jump(header));
        self.b.switch_to(header);
        let len = self.b.value(TyId::INT, Op::Len(list), span);
        let go = self.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::Lt, left: position_op, right: len }, span);
        self.b.terminate(Terminator::Branch { condition: go, then: inner, otherwise: exit });
        self.b.switch_to(inner);
        let value = self.b.value(element, Op::Index { object: list, index: position_op }, span);
        self.bind(item.pattern, value, span);
        self.open_loop(exit, step);
        self.block(&item.body);
        self.loops.pop();
        self.jump_to(step);
        self.b.switch_to(step);
        self.b.emit(Some(position), Op::Binary { op: BinaryOp::Add, left: position_op, right: Operand::Const(Const::Int(1)) }, span);
        self.b.terminate(Terminator::Jump(header));
        self.b.switch_to(exit);
    }

    /// Calls an intrinsic that returns the next item or `None`, until it returns `None`.
    fn for_next(&mut self, item: &ForLoop, intrinsic: &str, writes_back: bool, span: Span) {
        let body = self.body;
        let element = body.patterns[item.pattern].ty;
        let source = self.expr(item.iterable);
        let source_ty = body.exprs[item.iterable].ty;
        let source_reg = self.b.reg(source_ty, None);
        self.b.emit(Some(source_reg), Op::Copy(source), span);
        let optional = self.program.types.optional(element);
        let name = self.interner.intern(intrinsic);
        let (header, inner, exit) = (self.b.block(), self.b.block(), self.b.block());
        self.b.terminate(Terminator::Jump(header));
        self.b.switch_to(header);
        let outs = if writes_back { vec![self.b.reg(source_ty, None)] } else { Vec::new() };
        let next = self.b.value(optional, Op::Call { callee: Callee::Intrinsic(name), args: vec![Operand::Reg(source_reg)], outs }, span);
        let has = self.b.value(TyId::BOOL, Op::IsSome(next), span);
        self.b.terminate(Terminator::Branch { condition: has, then: inner, otherwise: exit });
        self.b.switch_to(inner);
        let value = self.b.value(element, Op::SomePayload(next), span);
        self.bind(item.pattern, value, span);
        self.open_loop(exit, header);
        self.block(&item.body);
        self.loops.pop();
        self.jump_to(header);
        self.b.switch_to(exit);
    }

    /// Binds an irrefutable pattern to a value.
    pub(super) fn bind(&mut self, pattern: Id<Pattern>, value: Operand, span: Span) {
        use mote_semantics::PatternKind;
        let body = self.body;
        let info = &body.patterns[pattern];
        match &info.kind {
            PatternKind::Bind(local) => self.bind_local(*local, value, span),
            PatternKind::Wildcard => {}
            PatternKind::Tuple(items) => {
                for (index, &item) in items.iter().enumerate() {
                    let part = self.b.value(body.patterns[item].ty, Op::TupleField { object: value, index: index as u32 }, span);
                    self.bind(item, part, span);
                }
            }
            _ => self.bind_irrefutable(pattern, value),
        }
    }

    /// `x ??= v`: writes `v` only when the place holds `None`.
    pub(super) fn coalesce_assign(&mut self, place: &Place, value: Id<Expr>, span: Span) {
        let current = self.read(place, span);
        let present = self.b.value(TyId::BOOL, Op::IsSome(current), span);
        let (set, join) = (self.b.block(), self.b.block());
        self.b.terminate(Terminator::Branch { condition: present, then: join, otherwise: set });
        self.b.switch_to(set);
        let value = self.expr(value);
        self.write(place, value, span);
        self.jump_to(join);
        self.b.switch_to(join);
    }
}
