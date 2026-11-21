use super::FnCx;
use crate::cx::Cx;
use crate::defs::{DefKind, LangItem, Payload};
use crate::types::{TyId, TyKind};

impl Cx<'_> {
    /// The part of the type that cannot be in a map key: a `Float`, or a collection that can change.
    pub(crate) fn unkeyable(&mut self, f: &FnCx, ty: TyId, seen: &mut Vec<TyId>) -> Option<TyId> {
        if seen.contains(&ty) {
            return None;
        }
        seen.push(ty);
        let inner: Vec<TyId> = match self.program.types.kind(ty).clone() {
            TyKind::Float(_) | TyKind::Array(_) => return Some(ty),
            TyKind::Param { index, .. } => {
                let keyed = f.bounds.get(index as usize).is_some_and(|bounds| bounds.iter().any(|bound| self.program.defs[bound.def].lang == Some(LangItem::Key)));
                return (!keyed).then_some(ty);
            }
            TyKind::Optional(element) | TyKind::FixedArray(element, _) => vec![element],
            TyKind::Tuple(items) | TyKind::Union(items) => items,
            TyKind::Adt { def, args } => {
                if matches!(self.program.defs[def].lang, Some(LangItem::List | LangItem::Map | LangItem::Set | LangItem::Array)) {
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
                declared.into_iter().map(|field| self.program.types.subst(field, &args)).collect()
            }
            _ => return None,
        };
        inner.into_iter().find_map(|part| self.unkeyable(f, part, seen))
    }
}
