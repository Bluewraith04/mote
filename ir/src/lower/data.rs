use mote_base::{Id, Span};
use mote_semantics::{Conversion, DefId, DefKind, Expr, FieldValue, Payload, TyId, TyKind};

use super::Lowering;
use crate::ir::{Const, Op, Operand, Terminator};

impl Lowering<'_> {
    fn operands(&mut self, items: &[Id<Expr>]) -> Vec<Operand> {
        items.iter().map(|&item| self.expr(item)).collect()
    }

    pub(super) fn tuple(&mut self, items: &[Id<Expr>], ty: TyId, span: Span) -> Operand {
        let items = self.operands(items);
        self.b.value(ty, Op::Tuple(items), span)
    }

    pub(super) fn list(&mut self, items: &[Id<Expr>], ty: TyId, span: Span) -> Operand {
        let items = self.operands(items);
        self.b.value(ty, Op::List(items), span)
    }

    pub(super) fn array(&mut self, items: &[Id<Expr>], ty: TyId, span: Span) -> Operand {
        let items = self.operands(items);
        self.b.value(ty, Op::Array(items), span)
    }

    pub(super) fn array_repeat(&mut self, value: Id<Expr>, len: Id<Expr>, ty: TyId, span: Span) -> Operand {
        let value = self.expr(value);
        let len = self.expr(len);
        self.b.value(ty, Op::ArrayRepeat { value, len }, span)
    }

    pub(super) fn map(&mut self, pairs: &[(Id<Expr>, Id<Expr>)], ty: TyId, span: Span) -> Operand {
        let pairs: Vec<(Operand, Operand)> = pairs
            .iter()
            .map(|&(key, value)| {
                let key = self.expr(key);
                (key, self.expr(value))
            })
            .collect();
        let Some(map) = self.collection_call(ty, "new", Vec::new(), ty, span) else { return self.unsupported(span, "a Map literal") };
        for (key, value) in pairs {
            self.collection_call(ty, "set", vec![map, key, value], TyId::UNIT, span);
        }
        map
    }

    pub(super) fn range(&mut self, start: Id<Expr>, end: Id<Expr>, inclusive: bool, ty: TyId, span: Span) -> Operand {
        let start = self.expr(start);
        let end = self.expr(end);
        self.b.value(ty, Op::Range { start, end, inclusive }, span)
    }

    /// A record or class literal: `..base` first, then the written fields in order, the rest read from the base.
    pub(super) fn record(&mut self, fields: &[FieldValue], base: Option<Id<Expr>>, ty: TyId, span: Span) -> Operand {
        let base = base.map(|base| self.expr(base));
        let types = self.field_types(ty);
        let mut slots: Vec<Option<Operand>> = vec![None; types.len()];
        for field in fields {
            slots[field.index as usize] = Some(self.expr(field.value));
        }
        let values = slots
            .into_iter()
            .enumerate()
            .map(|(index, slot)| match (slot, base) {
                (Some(value), _) => value,
                (None, Some(object)) => self.b.value(types[index], Op::Field { object, index: index as u32 }, span),
                (None, None) => Operand::Const(Const::Null),
            })
            .collect();
        self.b.value(ty, Op::Record(values), span)
    }

    /// An enum variant with its payload in declaration order.
    pub(super) fn variant(&mut self, variant: u32, fields: &[FieldValue], ty: TyId, span: Span) -> Operand {
        let count = match self.program.types.kind(ty) {
            TyKind::Adt { def, .. } => match &self.program.defs[*def].kind {
                DefKind::Enum(item) => match &item.variants[variant as usize].payload {
                    Payload::Unit => 0,
                    Payload::Tuple(types) => types.len(),
                    Payload::Struct(fields) => fields.len(),
                },
                _ => 0,
            },
            _ => 0,
        };
        let mut slots: Vec<Operand> = vec![Operand::Const(Const::Null); count];
        for field in fields {
            slots[field.index as usize] = self.expr(field.value);
        }
        self.b.value(ty, Op::Variant { variant, fields: slots }, span)
    }

    /// The field types of a record or class type, with its type arguments applied.
    fn field_types(&mut self, ty: TyId) -> Vec<TyId> {
        let TyKind::Adt { def, args } = self.program.types.kind(ty).clone() else { return Vec::new() };
        let declared: Vec<TyId> = self.program.defs[def].record().map(|record| record.fields.iter().map(|field| field.ty).collect()).unwrap_or_default();
        declared.into_iter().map(|field| self.program.types.subst(field, &args)).collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn slice(&mut self, object: Id<Expr>, start: Option<Id<Expr>>, end: Option<Id<Expr>>, inclusive: bool, step: Option<Id<Expr>>, ty: TyId, span: Span) -> Operand {
        let object = self.expr(object);
        let start = start.map(|bound| self.expr(bound));
        let end = end.map(|bound| self.expr(bound));
        let step = step.map(|bound| self.expr(bound));
        self.b.value(ty, Op::Slice { object, start, end, inclusive, step }, span)
    }

    /// `xs[a..b] = v` on a list.
    pub(super) fn assign_slice(&mut self, target: Id<Expr>, value: Id<Expr>, span: Span) {
        let body = self.body;
        let mote_semantics::ExprKind::Slice { object, start, end, inclusive, .. } = &body.exprs[target].kind else { return };
        let object = self.expr(*object);
        let start = start.map(|bound| self.expr(bound));
        let end = end.map(|bound| self.expr(bound));
        let value = self.expr(value);
        self.b.emit(None, Op::SetSlice { object, start, end, inclusive: *inclusive, value }, span);
    }

    pub(super) fn convert(&mut self, conversion: Conversion, inner: Id<Expr>, ty: TyId, span: Span) -> Operand {
        let operand = self.expr(inner);
        match conversion {
            Conversion::Narrow => self.narrow(operand, self.body.exprs[inner].ty, ty, span),
            Conversion::Adapt => self.adapt(operand, self.body.exprs[inner].ty, ty, span),
            Conversion::ToUnion => {
                let operand = match operand {
                    Operand::Const(_) => self.b.value(self.body.exprs[inner].ty, Op::Copy(operand), span),
                    reg => reg,
                };
                self.b.value(ty, Op::Convert { conversion, operand }, span)
            }
            conversion => self.b.value(ty, Op::Convert { conversion, operand }, span),
        }
    }

    /// A value read as the narrower type a test proved: the payload of an optional or the member of a union.
    pub(super) fn narrow(&mut self, operand: Operand, from: TyId, to: TyId, span: Span) -> Operand {
        if from == to {
            return operand;
        }
        match self.program.types.kind(from).clone() {
            TyKind::Optional(inner) => {
                let payload = self.b.value(inner, Op::SomePayload(operand), span);
                self.narrow(payload, inner, to, span)
            }
            TyKind::Union(_) => self.b.value(to, Op::UnionPayload { operand, ty: to }, span),
            _ => operand,
        }
    }

    /// Whether the type is an optional union, which a test of one member takes apart in two steps.
    pub(super) fn optional_union(&self, ty: TyId, target: TyId) -> Option<TyId> {
        match self.program.types.kind(ty) {
            TyKind::Optional(held) if *held != target && matches!(self.program.types.kind(*held), TyKind::Union(_)) => Some(*held),
            _ => None,
        }
    }

    /// `is` on an optional union: it holds a value, and the union holds a member of the type.
    fn is_type_in_optional(&mut self, operand: Operand, held: TyId, target: TyId, span: Span) -> Operand {
        let result = self.b.reg(TyId::BOOL, None);
        let (some, none, join) = (self.b.block(), self.b.block(), self.b.block());
        let has = self.b.value(TyId::BOOL, Op::IsSome(operand), span);
        self.b.terminate(Terminator::Branch { condition: has, then: some, otherwise: none });
        self.b.switch_to(some);
        let payload = self.b.value(held, Op::SomePayload(operand), span);
        let is = self.b.value(TyId::BOOL, Op::IsType { operand: payload, ty: target }, span);
        self.b.emit(Some(result), Op::Copy(is), span);
        self.b.terminate(Terminator::Jump(join));
        self.b.switch_to(none);
        self.b.emit(Some(result), Op::Copy(Operand::Const(Const::Bool(false))), span);
        self.b.terminate(Terminator::Jump(join));
        self.b.switch_to(join);
        Operand::Reg(result)
    }

    pub(super) fn is_type(&mut self, inner: Id<Expr>, target: TyId, span: Span) -> Operand {
        let operand = self.expr(inner);
        if let Some(held) = self.optional_union(self.body.exprs[inner].ty, target) {
            return self.is_type_in_optional(operand, held, target, span);
        }
        let op = match self.program.types.kind(self.body.exprs[inner].ty) {
            _ if self.body.exprs[inner].ty == target => Op::Copy(Operand::Const(Const::Bool(true))),
            TyKind::Optional(payload) if *payload == target => Op::IsSome(operand),
            _ => Op::IsType { operand, ty: target },
        };
        self.b.value(TyId::BOOL, op, span)
    }

    pub(super) fn unwrap(&mut self, inner: Id<Expr>, ty: TyId, span: Span) -> Operand {
        let operand = self.expr(inner);
        self.b.value(ty, Op::Unwrap(operand), span)
    }

    /// `e?`: returns `None` or the converted `Err` from the function, else continues with the payload.
    pub(super) fn propagate(&mut self, inner: Id<Expr>, span: Span) -> Operand {
        let operand = self.expr(inner);
        let source = self.body.exprs[inner].ty;
        let (ok, fail) = (self.b.block(), self.b.block());
        match self.program.types.kind(source).clone() {
            TyKind::Optional(payload) => {
                let has = self.b.value(TyId::BOOL, Op::IsSome(operand), span);
                self.b.terminate(Terminator::Branch { condition: has, then: ok, otherwise: fail });
                self.b.switch_to(fail);
                self.emit_return(Some(Operand::Const(Const::Null)));
                self.b.switch_to(ok);
                self.b.value(payload, Op::SomePayload(operand), span)
            }
            TyKind::Adt { def, args } => {
                let (ok_index, err_index) = (self.variant_index(def, "Ok"), self.variant_index(def, "Err"));
                let succeeded = self.b.value(TyId::BOOL, Op::IsVariant { object: operand, variant: ok_index }, span);
                self.b.terminate(Terminator::Branch { condition: succeeded, then: ok, otherwise: fail });
                self.b.switch_to(fail);
                let error = self.b.value(args[1], Op::VariantField { object: operand, variant: err_index, index: 0 }, span);
                let target = match self.program.types.kind(self.ret).clone() {
                    TyKind::Adt { args: wanted, .. } => wanted.get(1).copied().unwrap_or(args[1]),
                    _ => args[1],
                };
                let error = self.fit_value(error, args[1], target, span);
                let failed = self.b.value(self.ret, Op::Variant { variant: err_index, fields: vec![error] }, span);
                self.emit_return(Some(failed));
                self.b.switch_to(ok);
                self.b.value(args[0], Op::VariantField { object: operand, variant: ok_index, index: 0 }, span)
            }
            _ => self.unsupported(span, "propagate"),
        }
    }

    /// The position of a named variant in an enum.
    fn variant_index(&self, def: DefId, name: &str) -> u32 {
        match &self.program.defs[def].kind {
            DefKind::Enum(item) => item.variants.iter().position(|variant| self.interner.resolve(variant.name) == name).unwrap_or(0) as u32,
            _ => 0,
        }
    }
}
