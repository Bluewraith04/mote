use mote_base::Id;
use mote_semantics::{BinaryOp, Expr, ExprKind, TyId, UnaryOp};

use super::Lowering;
use crate::ir::{Const, Op, Operand};

impl Lowering<'_> {
    pub(super) fn expr(&mut self, id: Id<Expr>) -> Operand {
        let body = self.body;
        let expr = &body.exprs[id];
        let (ty, span) = (expr.ty, expr.span);
        match &expr.kind {
            ExprKind::Int(n) => Operand::Const(Const::Int(*n)),
            ExprKind::Float(x) => Operand::Const(Const::Float(*x)),
            ExprKind::Bool(b) => Operand::Const(Const::Bool(*b)),
            ExprKind::Null => Operand::Const(Const::Null),
            ExprKind::String(s) => Operand::Const(Const::String(*s)),
            ExprKind::Concat(parts) => {
                let parts = parts.iter().map(|&part| self.expr(part)).collect();
                self.b.value(TyId::STRING, Op::Concat(parts), span)
            }
            ExprKind::Show(inner) => {
                let operand = self.expr(*inner);
                self.b.value(TyId::STRING, Op::Show(operand), span)
            }
            ExprKind::Local(local) => self.local(*local, span),
            ExprKind::Global(_) | ExprKind::Field { .. } | ExprKind::TupleField { .. } | ExprKind::Index { .. } => match self.place(id) {
                Some(place) => self.read(&place, span),
                None => Operand::Const(Const::Null),
            },
            ExprKind::Unary { op, operand } => {
                let operand = self.expr(*operand);
                self.b.value(ty, Op::Unary { op: *op, operand }, span)
            }
            ExprKind::Binary { op: BinaryOp::And | BinaryOp::Or, .. } => self.logic_value(id),
            ExprKind::Binary { op: op @ (BinaryOp::Eq | BinaryOp::NotEq), left, right } if body.exprs[*left].ty == body.exprs[*right].ty && self.is_collection(body.exprs[*left].ty) => {
                let operand_ty = body.exprs[*left].ty;
                let (left, right) = (self.expr(*left), self.expr(*right));
                let same = self.collection_call(operand_ty, "equals", vec![left, right], TyId::BOOL, span);
                match (same, op) {
                    (Some(same), BinaryOp::NotEq) => self.b.value(TyId::BOOL, Op::Unary { op: UnaryOp::Not, operand: same }, span),
                    (Some(same), _) => same,
                    (None, _) => self.unsupported(span, "== on a collection"),
                }
            }
            ExprKind::Binary { op, left, right } => {
                let left = self.expr(*left);
                let right = self.expr(*right);
                self.b.value(ty, Op::Binary { op: *op, left, right }, span)
            }
            ExprKind::Cast(inner) => {
                let operand = self.expr(*inner);
                self.b.value(ty, Op::Cast(operand), span)
            }
            ExprKind::Call { callee, args } => self.call(id, callee, args, ty, span),
            ExprKind::Function(function) => self.function_value(function, ty, span),
            ExprKind::Tuple(items) => self.tuple(items, ty, span),
            ExprKind::List(items) => self.list(items, ty, span),
            ExprKind::Array(items) => self.array(items, ty, span),
            ExprKind::ArrayRepeat { value, len } => self.array_repeat(*value, *len, ty, span),
            ExprKind::Map(pairs) => self.map(pairs, ty, span),
            ExprKind::Record { fields, base } => self.record(fields, *base, ty, span),
            ExprKind::Variant { variant, fields } => self.variant(*variant, fields, ty, span),
            ExprKind::Range { start, end, inclusive } => self.range(*start, *end, *inclusive, ty, span),
            ExprKind::Slice { object, start, end, inclusive, step } => self.slice(*object, *start, *end, *inclusive, *step, ty, span),
            ExprKind::Is { expr, ty: target } => self.is_type(*expr, *target, span),
            ExprKind::Convert { conversion, expr } => self.convert(*conversion, *expr, ty, span),
            ExprKind::Unwrap(inner) => self.unwrap(*inner, ty, span),
            ExprKind::Propagate(inner) => self.propagate(*inner, span),
            ExprKind::Closure(closure) => self.closure(closure, ty, span),
            ExprKind::Spawn { task, scoped } => self.spawn(task, *scoped, ty, span),
            ExprKind::If { condition, then, otherwise } => self.if_value(condition, *then, *otherwise, ty, span),
            ExprKind::Match(item) => self.match_arms(item, ty, span),
            ExprKind::Error => Operand::Const(Const::Null),
        }
    }

    /// A `let` is read where it stands; a `var` is copied so a later write does not change the value.
    fn local(&mut self, local: Id<mote_semantics::Local>, span: mote_base::Span) -> Operand {
        let Some(&reg) = self.regs.get(&local) else { return self.unsupported(span, "local of an unsupported construct") };
        let info = &self.body.locals[local];
        if self.cells.contains(&local) {
            self.b.value(info.ty, Op::LoadCell(Operand::Reg(reg)), span)
        } else if info.is_var { self.b.value(info.ty, Op::Copy(Operand::Reg(reg)), span) } else { Operand::Reg(reg) }
    }
}
