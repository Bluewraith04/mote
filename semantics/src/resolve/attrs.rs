use mote_base::Symbol;
use mote_syntax as ast;
use mote_syntax::{AttrValue, ItemKind};

use crate::cx::Cx;
use crate::defs::{DefId, DefKind, Link, LangItem, ModuleId};

/// The attribute names; any other is an error.
pub const KNOWN: [&str; 6] = ["link", "blocking", "intrinsic", "lang", "stable", "ignore"];

/// The `@link` arguments: a library and an optional symbol.
struct LinkArgs {
    library: Symbol,
    symbol: Option<Symbol>,
}

impl Cx<'_> {
    /// Checks an item's attributes and records `@link`, `@blocking` and `@lang`.
    pub(super) fn attributes(&mut self, module: ModuleId, item: &ast::Item, def: DefId) {
        let is_std = self.program.modules[module].is_std;
        let native = matches!(item.kind, ItemKind::NativeFunction(_));
        let mut link: Option<LinkArgs> = None;
        let mut blocking = false;
        let mut intrinsic: Option<Symbol> = None;
        let mut named_intrinsic = false;
        for attr in &item.attrs {
            let name = self.text(attr.name.name);
            if !KNOWN.contains(&name.as_str()) {
                self.error(attr.span, format!("unknown attribute `@{name}`"));
                continue;
            }
            match name.as_str() {
                "link" | "blocking" if !native => self.error(attr.span, format!("`@{name}` goes on a native fn")),
                "link" => link = self.link_args(attr),
                "blocking" => blocking = true,
                "intrinsic" => {
                    named_intrinsic = true;
                    intrinsic = self.intrinsic_attr(attr, native, is_std);
                }
                "lang" => self.lang_attr(item, def, attr, is_std),
                "stable" if !is_std => self.error(attr.span, "`@stable` is for std only"),
                "ignore" if !matches!(item.kind, ItemKind::Test(_)) => {
                    self.error(attr.span, "`@ignore` goes on a test");
                }
                _ => {}
            }
        }
        if !native {
            return;
        }
        let span = self.program.defs[def].span;
        match (link, intrinsic) {
            (Some(_), Some(_)) => self.error(span, "a native fn takes `@link` or `@intrinsic`, not both"),
            (Some(args), None) => {
                let symbol = args.symbol.unwrap_or(self.program.defs[def].name);
                if let DefKind::Function(function) = &mut self.program.defs[def].kind {
                    function.link = Some(Link { library: args.library, symbol, blocking });
                }
            }
            (None, Some(name)) => {
                if let DefKind::Function(function) = &mut self.program.defs[def].kind {
                    function.intrinsic = Some(name);
                }
            }
            (None, None) if !named_intrinsic => self.error(span, "a native fn needs `@link`"),
            (None, None) => {}
        }
    }

    fn intrinsic_attr(&mut self, attr: &ast::Attribute, native: bool, is_std: bool) -> Option<Symbol> {
        if !is_std {
            self.error(attr.span, "`@intrinsic` is for std only");
            return None;
        }
        if !native {
            self.error(attr.span, "`@intrinsic` goes on a native fn");
            return None;
        }
        match attr.args.as_slice() {
            [arg] if arg.key.is_none() => match arg.value {
                AttrValue::String(name) => Some(name),
                _ => {
                    self.error(arg.span, "`@intrinsic` takes a name in quotes");
                    None
                }
            },
            _ => {
                self.error(attr.span, "`@intrinsic` takes one name");
                None
            }
        }
    }

    fn link_args(&mut self, attr: &ast::Attribute) -> Option<LinkArgs> {
        let mut library = None;
        let mut symbol = None;
        let mut valid = true;
        for arg in &attr.args {
            let key = arg.key.as_ref().and_then(|key| (key.segments.len() == 1).then(|| self.text(key.segments[0].name)));
            match (&arg.value, arg.key.is_some(), key.as_deref()) {
                (AttrValue::String(text), false, _) if library.is_none() => library = Some(*text),
                (AttrValue::String(text), true, Some("symbol")) if symbol.is_none() => symbol = Some(*text),
                _ => valid = false,
            }
        }
        match (library, valid) {
            (Some(library), true) => Some(LinkArgs { library, symbol }),
            _ => {
                self.error(attr.span, "`@link` takes a library name and an optional `symbol`");
                None
            }
        }
    }

    fn lang_attr(&mut self, item: &ast::Item, def: DefId, attr: &ast::Attribute, is_std: bool) {
        if !is_std {
            self.error(attr.span, "`@lang` is for std only");
            return;
        }
        if !matches!(item.kind, ItemKind::Record(_) | ItemKind::Enum(_) | ItemKind::Trait(_)) {
            self.error(attr.span, "`@lang` goes on a type or trait");
            return;
        }
        let [arg] = attr.args.as_slice() else {
            self.error(attr.span, "`@lang` takes one name");
            return;
        };
        let AttrValue::String(name) = arg.value else {
            self.error(arg.span, "`@lang` takes a name in quotes");
            return;
        };
        let text = self.text(name);
        let Some(lang) = LangItem::from_name(&text) else {
            self.error(arg.span, format!("unknown lang item `{text}`"));
            return;
        };
        if self.program.lang.contains_key(&lang) {
            self.error(attr.span, format!("lang item `{text}` is declared twice"));
            return;
        }
        self.program.defs[def].lang = Some(lang);
        self.program.lang.insert(lang, def);
    }
}
