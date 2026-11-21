use std::collections::HashSet;

use mote_syntax::ItemKind;

use crate::cx::Cx;
use crate::defs::DefId;
use crate::resolve::KNOWN;

impl Cx<'_> {
    /// Checks where `@stable` goes and that no variant carries an attribute.
    pub(crate) fn check_attribute_uses(&mut self) {
        let mut seen = HashSet::new();
        let items: Vec<DefId> = self
            .program
            .defs
            .iter()
            .filter(|(_, def)| def.origin.is_some_and(|origin| origin.member.is_none() && seen.insert((def.module, origin.item))))
            .map(|(id, _)| id)
            .collect();
        for def in items {
            let Some((item, _)) = self.item_of(def) else { continue };
            for attr in &item.attrs {
                if self.text(attr.name.name) == "stable" && self.program.modules[self.program.defs[def].module].is_std && !item.is_pub {
                    self.error(attr.span, "`@stable` goes on a `pub` item");
                }
            }
            let ItemKind::Enum(en) = &item.kind else { continue };
            for attr in en.variants.iter().flat_map(|variant| &variant.attrs) {
                let name = self.text(attr.name.name);
                if KNOWN.contains(&name.as_str()) {
                    self.error(attr.span, format!("`@{name}` does not go on a variant"));
                } else {
                    self.error(attr.span, format!("unknown attribute `@{name}`"));
                }
            }
        }
    }
}
