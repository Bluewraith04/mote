use mote_syntax as ast;
use mote_syntax::ItemKind;

use crate::cx::Cx;
use crate::defs::{DefId, ModuleId};

/// The attribute names; any other is an error.
pub const KNOWN: [&str; 3] = ["blocking", "stable", "ignore"];

impl Cx<'_> {
    /// Checks an item's attributes (`@stable`, `@ignore`).
    pub(super) fn attributes(&mut self, module: ModuleId, item: &ast::Item, _def: DefId) {
        let is_std = self.program.modules[module].is_std;
        for attr in &item.attrs {
            let name = self.text(attr.name.name);
            if !KNOWN.contains(&name.as_str()) {
                self.error(attr.span, format!("unknown attribute `@{name}`"));
                continue;
            }
            match name.as_str() {
                "blocking" => self.error(attr.span, "`@blocking` goes on a function in an `extern` block"),
                "stable" if !is_std => self.error(attr.span, "`@stable` is for std only"),
                "ignore" if !matches!(item.kind, ItemKind::Test(_)) => {
                    self.error(attr.span, "`@ignore` goes on a test");
                }
                _ => {}
            }
        }
    }

    /// Checks attributes on an `extern "lib"` function declaration; returns whether `@blocking` is set.
    pub(super) fn extern_fn_attributes(&mut self, decl: &ast::ExternFn) -> bool {
        let mut blocking = false;
        for attr in &decl.attrs {
            let name = self.text(attr.name.name);
            if name == "blocking" {
                if !attr.args.is_empty() {
                    self.error(attr.span, "`@blocking` takes no arguments");
                }
                blocking = true;
            } else {
                self.error(attr.span, format!("unexpected attribute `@{name}` in `extern` block"));
            }
        }
        blocking
    }
}
