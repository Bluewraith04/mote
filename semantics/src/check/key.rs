use mote_base::Span;

use super::FnCx;
use crate::cx::Cx;
use crate::defs::{DefKind, Payload};
use crate::types::{BuiltinTy, TyId, TyKind};

impl Cx<'_> {
    pub(crate) fn check_key_type(&mut self, f: &FnCx, ty: TyId, span: Span) {
        if let Some(offender) = self.unkeyable(f, ty, &mut Vec::new()) {
            self.error(span, format!("{} cannot be a Map/Set key: it contains {}", self.show_ty(ty), self.show_ty(offender)));
        }
    }

    /// The part of the type that cannot be in a map key: a `Float`, or a collection that can change.
    pub(crate) fn unkeyable(&mut self, f: &FnCx, ty: TyId, seen: &mut Vec<TyId>) -> Option<TyId> {
        if seen.contains(&ty) {
            return None;
        }
        seen.push(ty);
        let inner: Vec<TyId> = match self.program.types.kind(ty).clone() {
            TyKind::Float(_) | TyKind::Array(_) | TyKind::Any => return Some(ty),
            TyKind::Builtin { kind: BuiltinTy::List, .. } => return Some(ty),
            TyKind::Param { index, .. } => {
                let keyed = f.bounds.get(index as usize).is_some_and(|bounds| bounds.iter().any(|bound| self.program.std.key == Some(bound.def)));
                return (!keyed).then_some(ty);
            }
            TyKind::Optional(element) | TyKind::FixedArray(element, _) => vec![element],
            TyKind::Tuple(items) | TyKind::Union(items) => items,
            TyKind::Adt { def, args } => {
                if self.program.std.map == Some(def) || self.program.std.set == Some(def) {
                    return Some(ty);
                }
                let declared: Vec<TyId> = match &self.program.defs[def].kind {
                    DefKind::Record(record) if !record.is_class => record.fields.iter().map(|field| field.ty).collect(),
                    DefKind::Enum(en) => en
                        .variants
                        .iter()
                        .flat_map(|variant| match &variant.payload {
                            Payload::Unit => Vec::new(),
                            Payload::Tuple(items) => items.clone(),
                            Payload::Struct(fields) => fields.iter().map(|field| field.ty).collect(),
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                declared.into_iter().map(|field| self.program.types.subst_generic(field, &args)).collect()
            }
            _ => return None,
        };
        inner.into_iter().find_map(|part| self.unkeyable(f, part, seen))
    }
}
