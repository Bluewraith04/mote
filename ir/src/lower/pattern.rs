use mote_base::{Id, Span};
use mote_semantics::{ArmBody, BinaryOp, Body, Literal, Match, Pattern, PatternKind, TyId, TyKind};

use super::Lowering;
use crate::ir::{BlockId, Const, Op, Operand, RegId, Terminator};

fn is_wildcard(body: &Body, id: Id<Pattern>) -> bool {
    matches!(body.patterns[id].kind, PatternKind::Wildcard)
}

/// What an arm needs of the subject before its own tests start.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Head {
    Variant(u32),
    Any,
    Other,
}

impl Lowering<'_> {
    /// The register of a local, made the first time it is bound; the alternatives of an or-pattern share one.
    pub(super) fn local_reg(&mut self, local: Id<mote_semantics::Local>) -> RegId {
        if let Some(&reg) = self.regs.get(&local) {
            return reg;
        }
        let info = &self.body.locals[local];
        let reg = if self.cells.contains(&local) { self.b.cell(info.ty, Some(info.name)) } else { self.b.reg(info.ty, Some(info.name)) };
        self.regs.insert(local, reg);
        reg
    }

    /// Binds a local to a value; a local in a cell gets a fresh cell.
    pub(super) fn bind_local(&mut self, local: Id<mote_semantics::Local>, value: Operand, span: Span) {
        let reg = self.local_reg(local);
        let op = if self.cells.contains(&local) { Op::NewCell(value) } else { Op::Copy(value) };
        self.b.emit(Some(reg), op, span);
    }

    /// Continues in a new block when the condition holds, else jumps to `fail`.
    fn require(&mut self, condition: Operand, fail: BlockId) {
        let ok = self.b.block();
        self.b.terminate(Terminator::Branch { condition, then: ok, otherwise: fail });
        self.b.switch_to(ok);
    }

    /// Continues in a new block when the condition does not hold, else jumps to `fail`.
    fn forbid(&mut self, condition: Operand, fail: BlockId) {
        let ok = self.b.block();
        self.b.terminate(Terminator::Branch { condition, then: fail, otherwise: ok });
        self.b.switch_to(ok);
    }

    fn compare(&mut self, op: BinaryOp, value: Operand, constant: Const, span: Span) -> Operand {
        self.b.value(TyId::BOOL, Op::Binary { op, left: value, right: Operand::Const(constant) }, span)
    }

    /// Tests the pattern against the value and binds its names, jumping to `fail` on a mismatch; on a match the current block goes on.
    pub(super) fn matches(&mut self, id: Id<Pattern>, value: Operand, fail: BlockId) {
        let body = self.body;
        let pattern = &body.patterns[id];
        let span = pattern.span;
        match &pattern.kind {
            PatternKind::Wildcard | PatternKind::Error => {}
            PatternKind::Bind(local) => self.bind_local(*local, value, span),
            PatternKind::At { local, pattern } => {
                self.bind_local(*local, value, span);
                self.matches(*pattern, value, fail);
            }
            PatternKind::Literal(Literal::Null) => {
                let some = self.b.value(TyId::BOOL, Op::IsSome(value), span);
                self.forbid(some, fail);
            }
            PatternKind::Literal(literal) => {
                let constant = match literal {
                    Literal::Int(n) => Const::Int(*n),
                    Literal::Bool(b) => Const::Bool(*b),
                    Literal::String(s) => Const::String(*s),
                    Literal::Null => Const::Null,
                };
                let same = self.compare(BinaryOp::Eq, value, constant, span);
                self.require(same, fail);
            }
            PatternKind::Range { start, end, inclusive } => {
                let above = self.compare(BinaryOp::GtEq, value, Const::Int(*start), span);
                self.require(above, fail);
                let below = self.compare(if *inclusive { BinaryOp::LtEq } else { BinaryOp::Lt }, value, Const::Int(*end), span);
                self.require(below, fail);
            }
            PatternKind::Or(alternatives) => {
                let success = self.b.block();
                let (last, rest) = alternatives.split_last().expect("an or-pattern has alternatives");
                for &alternative in rest {
                    let next = self.b.block();
                    self.matches(alternative, value, next);
                    self.b.terminate(Terminator::Jump(success));
                    self.b.switch_to(next);
                }
                self.matches(*last, value, fail);
                self.b.terminate(Terminator::Jump(success));
                self.b.switch_to(success);
            }
            PatternKind::Tuple(items) => {
                for (index, &item) in items.iter().enumerate().filter(|&(_, &item)| !is_wildcard(body, item)) {
                    let part = self.b.value(body.patterns[item].ty, Op::TupleField { object: value, index: index as u32 }, span);
                    self.matches(item, part, fail);
                }
            }
            PatternKind::OptionalSome(inner) => {
                let some = self.b.value(TyId::BOOL, Op::IsSome(value), span);
                self.require(some, fail);
                let payload = self.b.value(body.patterns[*inner].ty, Op::SomePayload(value), span);
                self.matches(*inner, payload, fail);
            }
            PatternKind::OptionalNone => {
                let some = self.b.value(TyId::BOOL, Op::IsSome(value), span);
                self.forbid(some, fail);
            }
            PatternKind::Variant { variant, fields } => {
                let is = self.b.value(TyId::BOOL, Op::IsVariant { object: value, variant: *variant }, span);
                self.require(is, fail);
                for field in fields.iter().filter(|field| !is_wildcard(body, field.pattern)) {
                    let part = self.b.value(body.patterns[field.pattern].ty, Op::VariantField { object: value, variant: *variant, index: field.index }, span);
                    self.matches(field.pattern, part, fail);
                }
            }
            PatternKind::Record { fields } => {
                for field in fields.iter().filter(|field| !is_wildcard(body, field.pattern)) {
                    let part = self.b.value(body.patterns[field.pattern].ty, Op::Field { object: value, index: field.index }, span);
                    self.matches(field.pattern, part, fail);
                }
            }
            PatternKind::Type { local, ty: target } => {
                let from = pattern.ty;
                if let Some(held) = self.optional_union(from, *target) {
                    let some = self.b.value(TyId::BOOL, Op::IsSome(value), span);
                    self.require(some, fail);
                    let payload = self.b.value(held, Op::SomePayload(value), span);
                    let is = self.b.value(TyId::BOOL, Op::IsType { operand: payload, ty: *target }, span);
                    self.require(is, fail);
                } else if from != *target {
                    let op = match self.program.types.kind(from) {
                        TyKind::Optional(inner) if inner == target => Op::IsSome(value),
                        _ => Op::IsType { operand: value, ty: *target },
                    };
                    let is = self.b.value(TyId::BOOL, op, span);
                    self.require(is, fail);
                }
                if let Some(local) = local {
                    let narrowed = self.narrow(value, from, *target, span);
                    self.bind_local(*local, narrowed, span);
                }
            }
        }
    }

    /// Binds a pattern that cannot fail; a failing test would be a bug in the checker.
    pub(super) fn bind_irrefutable(&mut self, pattern: Id<Pattern>, value: Operand) {
        let here = self.b.current();
        let fail = self.b.block();
        self.b.switch_to(fail);
        self.b.terminate(Terminator::Unreachable);
        self.b.switch_to(here);
        self.matches(pattern, value, fail);
    }

    fn head(&self, id: Id<Pattern>) -> Head {
        match &self.body.patterns[id].kind {
            PatternKind::Variant { variant, .. } => Head::Variant(*variant),
            PatternKind::Wildcard | PatternKind::Bind(_) => Head::Any,
            PatternKind::At { pattern, .. } => self.head(*pattern),
            _ => Head::Other,
        }
    }

    /// `match`: each arm's pattern, then its guard, in order; an enum subject first switches on the tag to the first arm that can match.
    pub(super) fn match_arms(&mut self, item: &Match, ty: TyId, span: Span) -> Operand {
        let body = self.body;
        let subject_ty = body.exprs[item.subject].ty;
        let subject = self.expr(item.subject);
        let result = (ty != TyId::UNIT).then(|| self.b.reg(ty, None));
        let join = self.b.block();
        let starts: Vec<BlockId> = item.arms.iter().map(|_| self.b.block()).collect();
        let no_match = self.b.block();
        self.dispatch(item, subject, subject_ty, &starts, no_match, span);
        for (index, arm) in item.arms.iter().enumerate() {
            self.b.switch_to(starts[index]);
            let next = starts.get(index + 1).copied().unwrap_or(no_match);
            self.matches(arm.pattern, subject, next);
            if let Some(guard) = arm.guard {
                let inner = self.b.block();
                self.branch_on(guard, inner, next);
                self.b.switch_to(inner);
            }
            match &arm.body {
                ArmBody::Value(value) => {
                    let value = self.expr(*value);
                    if let Some(result) = result {
                        self.b.emit(Some(result), Op::Copy(value), arm.span);
                    }
                }
                ArmBody::Block(block) => self.block(block),
            }
            if !self.b.is_terminated() {
                self.b.terminate(Terminator::Jump(join));
            }
        }
        self.b.switch_to(no_match);
        self.b.terminate(Terminator::Unreachable);
        self.b.switch_to(join);
        result.map_or(Operand::Const(Const::Null), Operand::Reg)
    }

    /// Ends the current block by going to the first arm: through a switch on the tag when every arm starts with a variant or a catch-all.
    fn dispatch(&mut self, item: &Match, subject: Operand, subject_ty: TyId, starts: &[BlockId], no_match: BlockId, span: Span) {
        let heads: Vec<Head> = item.arms.iter().map(|arm| self.head(arm.pattern)).collect();
        let is_enum = matches!(self.program.types.kind(subject_ty), TyKind::Adt { def, .. } if self.program.defs[*def].enum_def().is_some());
        if !is_enum || item.arms.len() < 2 || heads.contains(&Head::Other) {
            self.b.terminate(Terminator::Jump(starts[0]));
            return;
        }
        let mut variants: Vec<u32> = heads.iter().filter_map(|head| if let Head::Variant(variant) = head { Some(*variant) } else { None }).collect();
        variants.sort_unstable();
        variants.dedup();
        let first_for = |wanted: Head| heads.iter().position(|head| *head == wanted || *head == Head::Any).map_or(no_match, |index| starts[index]);
        let cases = variants.iter().map(|&variant| (i128::from(variant), first_for(Head::Variant(variant)))).collect();
        let default = heads.iter().position(|head| *head == Head::Any).map_or(no_match, |index| starts[index]);
        let tag = self.b.value(TyId::INT, Op::VariantTag(subject), span);
        self.b.terminate(Terminator::Switch { value: tag, cases, default });
    }
}
