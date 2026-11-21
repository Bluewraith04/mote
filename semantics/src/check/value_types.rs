use crate::cx::Cx;
use crate::defs::{DefId, DefKind};
use crate::types::{TyId, TyKind};

impl Cx<'_> {
    /// Checks that every struct holds only values that cannot change under it and does not contain itself.
    pub(crate) fn check_structs(&mut self) {
        let structs: Vec<DefId> = self.program.defs.iter().filter(|(_, def)| matches!(&def.kind, DefKind::Record(record) if !record.is_class)).map(|(id, _)| id).collect();
        for def in structs {
            let fields = self.program.defs[def].record().expect("a record").fields.clone();
            for field in &fields {
                if self.changes_inside(field.ty) {
                    let message = format!("a struct cannot hold {}", self.show_ty(field.ty));
                    self.error(field.span, message);
                }
            }
            if fields.iter().any(|field| self.holds(field.ty, def, &mut vec![def])) {
                let message = format!("struct `{}` contains itself", self.text(self.program.defs[def].name));
                self.error(self.program.defs[def].span, message);
            }
        }
    }

    /// Whether a value of the type can change without a write to the value that holds it.
    pub(crate) fn changes_inside(&self, ty: TyId) -> bool {
        self.program.changes_inside(ty)
    }

    /// Whether a value of the type holds a `target` struct inline.
    fn holds(&self, ty: TyId, target: DefId, seen: &mut Vec<DefId>) -> bool {
        match self.program.types.kind(ty) {
            TyKind::Optional(inner) | TyKind::FixedArray(inner, _) => self.holds(*inner, target, seen),
            TyKind::Tuple(members) | TyKind::Union(members) => members.iter().any(|&member| self.holds(member, target, seen)),
            TyKind::Adt { def, args } => {
                if *def == target {
                    return true;
                }
                let DefKind::Record(record) = &self.program.defs[*def].kind else { return false };
                if record.is_class {
                    return false;
                }
                if args.iter().any(|&arg| self.holds(arg, target, seen)) {
                    return true;
                }
                if seen.contains(def) {
                    return false;
                }
                seen.push(*def);
                record.fields.iter().any(|field| self.holds(field.ty, target, seen))
            }
            _ => false,
        }
    }
}
