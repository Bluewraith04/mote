//! The `std` sources bundled into the compiler. A non-relative `import std.<name>` resolves to one of these instead of the filesystem, and every non-`std` module is compiled with an injected `import std.prelude`.

use compiler::ast::{Item, ImportDecl, ModulePath, Program};
use compiler::span::Span;

use crate::path::CanonicalModuleId;

const EMBEDDED_STD: &[(&str, &str)] = &[
    ("result", include_str!("../std/result.mote")),
    ("error", include_str!("../std/error.mote")),
    ("prelude", include_str!("../std/prelude.mote")),
    ("traits", include_str!("../std/traits.mote")),
    ("iter", include_str!("../std/iter.mote")),
    ("collections", include_str!("../std/collections.mote")),
    ("string", include_str!("../std/string.mote")),
    ("math", include_str!("../std/math.mote")),
    ("sys", include_str!("../std/sys.mote")),
    ("sys.env", include_str!("../std/sys/env.mote")),
    ("sys.io", include_str!("../std/sys/io.mote")),
    ("sys.fs", include_str!("../std/sys/fs.mote")),
    ("sys.process", include_str!("../std/sys/process.mote")),
    ("sys.net", include_str!("../std/sys/net.mote")),
    ("sys.http_server", include_str!("../std/sys/http_server.mote")),
    ("sys.tls", include_str!("../std/sys/tls.mote")),
    ("sys.gui", include_str!("../std/sys/gui.mote")),
    ("data.crypto", include_str!("../std/data/crypto.mote")),
    ("time", include_str!("../std/time.mote")),
    ("date", include_str!("../std/date.mote")),
    ("data", include_str!("../std/data.mote")),
    ("data.json", include_str!("../std/data/json.mote")),
    ("dev", include_str!("../std/dev.mote")),
    ("dev.args", include_str!("../std/dev/args.mote")),
    ("data.base64", include_str!("../std/data/base64.mote")),
    ("data.uuid", include_str!("../std/data/uuid.mote")),
    ("dev.libtools", include_str!("../std/dev/libtools.mote")),
    ("regex", include_str!("../std/regex.mote")),
    ("random", include_str!("../std/random.mote")),
    ("stream", include_str!("../std/stream.mote")),
    ("task", include_str!("../std/task.mote")),
    ("test", include_str!("../std/test.mote")),
    ("experimental.types", include_str!("../std/experimental/types.mote")),
];

/// The synthetic module-id / path string for embedded module `name`.
pub(crate) fn embedded_id(name: &str) -> String {
    format!("<std>/{name}")
}

/// True for an id produced by [`embedded_id`].
pub(crate) fn is_embedded_id(id: &str) -> bool {
    id.strip_prefix("<std>/").is_some()
}

/// The source of the embedded module with canonical id `id`.
pub(crate) fn source_for_id(id: &str) -> Option<&'static str> {
    let name = id.strip_prefix("<std>/")?;
    EMBEDDED_STD.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

/// Every embedded `std.*` module's `(name, source)`.
pub fn all() -> &'static [(&'static str, &'static str)] {
    EMBEDDED_STD
}

/// The embedded modules nested under `segments`, as dotted paths: `std.experimental` gives `std.experimental.types`.
pub(crate) fn modules_under(segments: &[String]) -> Vec<String> {
    let prefix = format!("{}.", segments.join("."));
    EMBEDDED_STD.iter().filter_map(|(n, _)| format!("std.{n}").strip_prefix(&prefix).map(|_| format!("std.{n}"))).collect()
}

/// The direct members of the embedded group `segments` (`std.data` gives `json`, `yaml`,...), each with its full path; empty when it is not a group.
pub(crate) fn group_members(segments: &[String]) -> Vec<(String, Vec<String>)> {
    let Some(group) = segments.strip_prefix(&["std".to_string()]).map(|rest| rest.join(".")) else {
        return Vec::new();
    };
    if !EMBEDDED_STD.iter().any(|(n, _)| *n == group) {
        return Vec::new();
    }
    let prefix = format!("{group}.");
    EMBEDDED_STD
        .iter()
        .filter_map(|(n, _)| n.strip_prefix(&prefix).filter(|member| !member.contains('.')).map(|member| {
            let path = std::iter::once("std".to_string()).chain(n.split('.').map(String::from)).collect();
            (member.to_string(), path)
        }))
        .collect()
}

/// The grouped path an old top-level `std.<name>` moved to (`json` gives `std.data.json`), if any.
pub(crate) fn moved_to(name: &str) -> Option<String> {
    let suffix = format!(".{name}");
    EMBEDDED_STD.iter().find(|(n, _)| n.ends_with(&suffix)).map(|(n, _)| format!("std.{n}"))
}

/// The package a `std` module became (`std.sys.ui` is the package `pane`), if it did.
pub(crate) fn packaged_as(segments: &[String]) -> Option<&'static str> {
    let path: Vec<&str> = segments.iter().map(String::as_str).collect();
    match path.as_slice() {
        ["std", "sys", "ui"] => Some("pane"),
        ["std", "dev", "log"] | ["std", "log"] => Some("log"),
        ["std", "sys", "path"] | ["std", "path"] => Some("path"),
        ["std", "data", "toml"] | ["std", "toml"] => Some("toml"),
        ["std", "data", "yaml"] | ["std", "yaml"] => Some("yaml"),
        ["std", "data", "compress"] | ["std", "compress"] => Some("compress"),
        ["std", "data", "archive"] | ["std", "archive"] => Some("archive"),
        ["std", "data", "sql"] | ["std", "sql"] => Some("sqlite"),
        ["std", "sys", "http"] | ["std", "http"] => Some("http"),
        _ => None,
    }
}

/// Resolves `import std.<name>` to an embedded id, or `None` when no such module is embedded.
pub fn resolve(segments: &[String]) -> Option<CanonicalModuleId> {
    let (first, rest) = segments.split_first()?;
    if first != "std" || rest.is_empty() {
        return None;
    }
    let name = rest.join(".");
    if EMBEDDED_STD.iter().any(|(n, _)| *n == name) {
        Some(CanonicalModuleId::new(embedded_id(&name)))
    } else {
        None
    }
}

/// Prepends `import std.prelude` to a user module; a no-op for embedded modules and for one that already imports it.
pub(crate) fn inject_prelude_import(program: &mut Program) {
    let already = program.items.iter().any(|it| {
        matches!(it, Item::Import(imp)
            if imp.glob
                && !imp.path.is_relative
                && imp.path.segments == ["std", "prelude"])
    });
    if already {
        return;
    }
    let span = Span::new(0, 0, 1, 1);
    let import = Item::Import(ImportDecl {
        path: ModulePath::new(vec!["std".into(), "prelude".into()], false, 0, span),
        from_path: None,
        alias: None,
        symbols: Vec::new(),
        glob: true,
        is_pub: false,
        span,
    });
    program.items.insert(0, import);
}
