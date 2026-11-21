use std::cell::Cell;

use mote_base::Span;

use crate::cx::Cx;
use crate::defs::{DefId, DefKind, Payload};
use crate::types::{TyId, TyKind};

impl Cx<'_> {
    /// Reports a private type that a `pub` declaration names.
    pub(crate) fn check_public_signatures(&mut self) {
        let defs: Vec<DefId> = self.program.defs.iter().map(|(id, _)| id).collect();
        for def in defs {
            let item = &self.program.defs[def];
            if !item.is_pub || item.owner.is_some_and(|owner| !self.program.defs[owner].is_pub) {
                continue;
            }
            let mut named: Vec<(TyId, Span)> = Vec::new();
            match &item.kind {
                DefKind::Function(function) => {
                    named.extend(function.params.iter().map(|param| (param.ty, item.span)));
                    named.push((function.ret, item.span));
                }
                DefKind::Global(global) => named.push((global.ty, item.span)),
                DefKind::Alias(alias) => named.push((alias.target, item.span)),
                DefKind::Record(record) => named.extend(record.fields.iter().filter(|field| field.is_pub).map(|field| (field.ty, field.span))),
                DefKind::Enum(en) => {
                    for variant in &en.variants {
                        match &variant.payload {
                            Payload::Unit => {}
                            Payload::Tuple(types) => named.extend(types.iter().map(|&ty| (ty, variant.span))),
                            Payload::Struct(fields) => named.extend(fields.iter().map(|field| (field.ty, field.span))),
                        }
                    }
                }
                _ => {}
            }
            for (ty, span) in named {
                if let Some(private) = self.private_type(ty) {
                    let message = format!("private type `{}` in a public declaration", self.text(self.program.defs[private].name));
                    self.error(span, message);
                    break;
                }
            }
        }
    }

    /// A type declared in the program without `pub` that the type mentions.
    fn private_type(&self, ty: TyId) -> Option<DefId> {
        let found = Cell::new(None);
        self.program.types.any(ty, &|kind| match kind {
            TyKind::Adt { def, .. } if found.get().is_none() && !self.program.defs[*def].is_pub && self.program.defs[*def].origin.is_some() => {
                found.set(Some(*def));
                true
            }
            _ => false,
        });
        found.get()
    }
}
