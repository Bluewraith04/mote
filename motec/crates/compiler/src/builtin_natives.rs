//! The builtin natives' interface file: the checker takes their signatures from it and codegen their names.

use std::sync::LazyLock;

use crate::ast::{Item, NativeFnDecl};
use crate::lexer::Lexer;
use crate::parser::Parser;

const SOURCE: &str = include_str!("builtin_natives.mote");

static DECLS: LazyLock<Vec<NativeFnDecl>> = LazyLock::new(|| {
    let tokens = Lexer::new(SOURCE).tokenize().expect("builtin natives lex");
    let program = Parser::new(tokens).parse().expect("builtin natives parse");
    program
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::NativeFunction(decl) => Some(decl),
            _ => None,
        })
        .collect()
});

/// Every declared builtin native.
pub fn decls() -> &'static [NativeFnDecl] {
    &DECLS
}

/// The registry name of a declared native: the declared name without a leading `__`.
pub fn registry_name(declared: &str) -> &str {
    declared.strip_prefix("__").unwrap_or(declared)
}

/// The registry name for a call to `declared`, if it is a builtin native.
pub fn lookup(declared: &str) -> Option<&'static str> {
    DECLS.iter().find(|d| d.name == declared).map(|d| registry_name(&d.name))
}

/// The payload type of a fallible native, written `-> __Fallible<T>`; `None` for any other native.
pub(crate) fn fallible_payload(decl: &NativeFnDecl) -> Option<&crate::ast::TypeNode> {
    match &decl.return_type {
        Some(crate::ast::TypeNode::Generic(name, args, _)) if name == "__Fallible" => args.first(),
        _ => None,
    }
}

/// Whether the builtin native `declared` is fallible: its call answers a `Result`.
pub(crate) fn is_fallible(declared: &str) -> bool {
    DECLS.iter().any(|d| d.name == declared && fallible_payload(d).is_some())
}
