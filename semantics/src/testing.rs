use std::collections::HashMap;

use mote_base::{DiagnosticSink, Interner, SourceMap};
use mote_syntax::{lex, parse, Import, ItemKind};

use crate::resolve::{Input, ModuleInput};

/// Sources parsed into a resolver input.
pub(crate) struct Parsed {
    pub input: Input,
    pub interner: Interner,
    pub sink: DiagnosticSink,
    pub map: SourceMap,
}

/// Parses sources given as (dotted path, text); the path `prelude` is the prelude and `std…` paths are std.
pub(crate) fn parse_sources(sources: &[(&str, &str)]) -> Parsed {
    let mut map = SourceMap::new();
    let mut interner = Interner::new();
    let mut sink = DiagnosticSink::new();
    let mut parsed = Vec::new();
    let mut starts = Vec::new();
    for (path, text) in sources {
        let id = map.add_file(*path, *text);
        starts.push(map.file(id).start);
        let lexed = lex(map.file(id), &mut interner, &mut sink);
        parsed.push(parse(map.file(id), &lexed, &mut interner, &mut sink));
    }
    let mut modules = Vec::new();
    for (((path, text), ast), source_start) in sources.iter().zip(parsed).zip(starts) {
        let mut imports = HashMap::new();
        for &item_id in &ast.root {
            let target = match &ast[item_id].kind {
                ItemKind::Import(Import::Module { path, .. }) => path,
                ItemKind::Import(Import::Names { from, .. } | Import::Glob { from }) => from,
                _ => continue,
            };
            let dotted: Vec<&str> = target.segments.iter().map(|segment| interner.resolve(segment.name)).collect();
            if let Some(index) = sources.iter().position(|(other, _)| *other == dotted.join(".")) {
                imports.insert(item_id, index);
            }
        }
        let is_std = *path == "std" || path.starts_with("std.");
        let path = path.split('.').map(|part| interner.intern(part)).collect();
        modules.push(ModuleInput { path, ast, is_std, imports, source: text.to_string(), source_start });
    }
    let prelude = sources.iter().position(|(path, _)| *path == "prelude");
    Parsed { input: Input { modules, prelude }, interner, sink, map }
}

#[cfg(test)]
pub(crate) fn path_text(interner: &Interner, path: &[mote_base::Symbol]) -> String {
    path.iter().map(|&part| interner.resolve(part)).collect::<Vec<_>>().join(".")
}
