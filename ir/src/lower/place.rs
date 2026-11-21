use mote_base::{Id, Span};
use mote_semantics::{DefId, Expr, ExprKind, TyId, TyKind};

use super::Lowering;
use crate::ir::{Op, Operand, RegId};

/// A place a value can be read from and written to, with its operands already evaluated.
pub(super) struct Place {
    kind: Kind,
    pub ty: TyId,
}

enum Kind {
    Reg(RegId),
    /// A local that lives in a cell.
    Cell(RegId),
    Global(DefId),
    /// A field of a record or class, or an element of a tuple.
    Field { base: Box<Place>, index: u32 },
    Index { base: Box<Place>, index: Operand },
}

impl Lowering<'_> {
    pub(super) fn is_place(&self, id: Id<Expr>) -> bool {
        matches!(self.body.exprs[id].kind, ExprKind::Local(_) | ExprKind::Global(_) | ExprKind::Field { .. } | ExprKind::TupleField { .. } | ExprKind::Index { .. })
    }

    /// Evaluates the operands of a place once; `None` when the place is not lowered yet.
    pub(super) fn place(&mut self, id: Id<Expr>) -> Option<Place> {
        let body = self.body;
        let expr = &body.exprs[id];
        let kind = match &expr.kind {
            ExprKind::Local(local) => match self.regs.get(local) {
                Some(&reg) if self.cells.contains(local) => Kind::Cell(reg),
                Some(&reg) => Kind::Reg(reg),
                None => {
                    self.unsupported(expr.span, "local of an unsupported construct");
                    return None;
                }
            },
            ExprKind::Global(def) => Kind::Global(*def),
            ExprKind::Field { object, index } | ExprKind::TupleField { object, index } => Kind::Field { base: Box::new(self.base(*object)), index: *index },
            ExprKind::Index { object, index } => {
                let base = Box::new(self.base(*object));
                Kind::Index { base, index: self.expr(*index) }
            }
            _ => {
                self.unsupported(expr.span, "assignment target");
                return None;
            }
        };
        Some(Place { kind, ty: expr.ty })
    }

    /// The place an object is read from; a value that is not a place is held in a fresh register.
    fn base(&mut self, id: Id<Expr>) -> Place {
        if self.is_place(id)
            && let Some(place) = self.place(id)
        {
            return place;
        }
        let value = self.expr(id);
        let ty = self.body.exprs[id].ty;
        let span = self.body.exprs[id].span;
        let reg = self.b.reg(ty, None);
        self.b.emit(Some(reg), Op::Copy(value), span);
        Place { kind: Kind::Reg(reg), ty }
    }

    /// The value in the place, read where it stands.
    fn load(&mut self, place: &Place, span: Span) -> Operand {
        match &place.kind {
            Kind::Reg(reg) => Operand::Reg(*reg),
            Kind::Cell(cell) => self.b.value(place.ty, Op::LoadCell(Operand::Reg(*cell)), span),
            Kind::Global(def) => self.b.value(place.ty, Op::LoadGlobal(*def), span),
            Kind::Field { base, index } => {
                let object = self.load(base, span);
                let op = if matches!(self.program.types.kind(base.ty), TyKind::Tuple(_)) { Op::TupleField { object, index: *index } } else { Op::Field { object, index: *index } };
                self.b.value(place.ty, op, span)
            }
            Kind::Index { base, index } => {
                let object = self.load(base, span);
                if self.is_collection(base.ty) {
                    return self.collection_call(base.ty, "get", vec![object, *index], place.ty, span).unwrap_or_else(|| self.unsupported(span, "indexing a Map"));
                }
                self.b.value(place.ty, Op::Index { object, index: *index }, span)
            }
        }
    }

    /// The value in the place, copied out of a register so a later write does not change it.
    pub(super) fn read(&mut self, place: &Place, span: Span) -> Operand {
        let value = self.load(place, span);
        if matches!(place.kind, Kind::Reg(_)) { self.b.value(place.ty, Op::Copy(value), span) } else { value }
    }

    pub(super) fn write(&mut self, place: &Place, value: Operand, span: Span) {
        match &place.kind {
            Kind::Reg(reg) => self.b.emit(Some(*reg), Op::Copy(value), span),
            Kind::Cell(cell) => self.b.emit(None, Op::StoreCell { cell: Operand::Reg(*cell), value }, span),
            Kind::Global(def) => self.b.emit(None, Op::StoreGlobal { def: *def, value }, span),
            Kind::Field { base, index } => self.write_through(base, span, |object| Op::SetField { object, index: *index, value }),
            Kind::Index { base, index } if self.is_collection(base.ty) => {
                let object = self.load(base, span);
                if self.collection_call(base.ty, "set", vec![object, *index, value], TyId::UNIT, span).is_none() {
                    self.unsupported(span, "assigning into a Map");
                }
            }
            Kind::Index { base, index } => self.write_through(base, span, |object| Op::SetIndex { object, index: *index, value }),
        }
    }

    /// Applies a write to the object in `base`: in place for a register or a reference, else on a copy that is stored back.
    fn write_through(&mut self, base: &Place, span: Span, op: impl FnOnce(Operand) -> Op) {
        if let Kind::Reg(reg) = base.kind {
            self.b.emit(None, op(Operand::Reg(reg)), span);
        } else if self.is_reference(base.ty) {
            let object = self.load(base, span);
            self.b.emit(None, op(object), span);
        } else {
            let object = self.load(base, span);
            let copy = self.b.reg(base.ty, None);
            self.b.emit(Some(copy), Op::Copy(object), span);
            self.b.emit(None, op(Operand::Reg(copy)), span);
            self.write(base, Operand::Reg(copy), span);
        }
    }
}
