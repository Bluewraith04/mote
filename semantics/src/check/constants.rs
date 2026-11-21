use std::collections::HashSet;

use crate::cx::Cx;
use crate::defs::DefKind;
use crate::types::{TyId, TyKind};

impl Cx<'_> {
    /// Reports a constant two unit variants of an enum share.
    pub(crate) fn check_enum_constants(&mut self) {
        let enums: Vec<_> = self.program.defs.iter().filter(|(_, def)| matches!(def.kind, DefKind::Enum(_))).map(|(id, _)| id).collect();
        for def in enums {
            let mut used = HashSet::new();
            let variants: Vec<_> = self.program.defs[def].enum_def().expect("an enum").variants.iter().filter_map(|variant| variant.discriminant.map(|value| (value, variant.span))).collect();
            for (value, span) in variants {
                if !used.insert(value) {
                    self.error(span, format!("constant {value} is used twice"));
                }
            }
        }
    }

    /// The constants of an enum whose every variant is a unit variant with one.
    pub(crate) fn enum_constants(&self, ty: TyId) -> Option<Vec<i128>> {
        let TyKind::Adt { def, .. } = self.program.types.kind(ty) else { return None };
        let item = self.program.defs[*def].enum_def()?;
        if item.variants.is_empty() {
            return None;
        }
        item.variants.iter().map(|variant| variant.discriminant).collect()
    }

    /// Whether a type is an enum, which `as` reads through its constants.
    pub(crate) fn is_enum(&self, ty: TyId) -> bool {
        matches!(self.program.types.kind(ty), TyKind::Adt { def, .. } if self.program.defs[*def].enum_def().is_some())
    }
}
