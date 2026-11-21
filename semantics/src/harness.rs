//! Compiles Mote source against the real `std/src`, for the tests of this and later crates.

use std::path::Path;

use mote_base::{Interner, Severity};

use crate::analyze::analyze;
use crate::check::check;
use crate::defs::{DefId, DefKind, Resolved};
use crate::instantiate::instantiate;
use crate::program::{Instance, Program};
use crate::resolve::resolve;
use crate::testing::parse_sources;
use crate::types::TyId;

/// A program that went through every pass, and what the passes reported.
pub struct Compiled {
    pub program: Program,
    pub interner: Interner,
    /// `file:line message` per error.
    pub errors: Vec<String>,
    /// Every plain function of `main` with a body and every `test` block, in source order.
    pub entries: Vec<Instance>,
}

/// Every `.mote` file under `dir` as (module path, text); `lib` is `std` and `prelude` is the prelude.
fn collect(dir: &Path, path: &[String], out: &mut Vec<(String, String)>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).expect("the std source directory").map(|entry| entry.expect("a directory entry").path()).collect();
    entries.sort();
    for entry in entries {
        let name = entry.file_stem().and_then(|stem| stem.to_str()).expect("a file name").to_string();
        if entry.is_dir() {
            let mut inner = path.to_vec();
            inner.push(name);
            collect(&entry, &inner, out);
        } else if entry.extension().is_some_and(|extension| extension == "mote") {
            let module = match (path.is_empty(), name.as_str()) {
                (true, "lib") => "std".to_string(),
                (true, "prelude") => "prelude".to_string(),
                _ => format!("std.{}", path.iter().cloned().chain([name]).collect::<Vec<_>>().join(".")),
            };
            out.push((module, std::fs::read_to_string(&entry).expect("a readable source")));
        }
    }
}

/// The std source as (module path, text).
pub fn std_sources() -> Vec<(String, String)> {
    let mut sources = Vec::new();
    collect(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../std/src"), &[], &mut sources);
    sources
}

/// The std source and `main` through resolve, check, instantiate and analyze; every plain function of `main` and every `test` block is an entry.
pub fn compile(main: &str) -> Compiled {
    build(main, false)
}

/// Like [`compile`], and every function and method of std that has a body is instantiated too, each type parameter as `Int`.
pub fn compile_all(main: &str) -> Compiled {
    build(main, true)
}

fn build(main: &str, whole_std: bool) -> Compiled {
    let mut sources = std_sources();
    sources.push(("main".to_string(), main.to_string()));
    let borrowed: Vec<(&str, &str)> = sources.iter().map(|(path, text)| (path.as_str(), text.as_str())).collect();
    let mut parsed = parse_sources(&borrowed);
    let mut program = resolve(parsed.input, &mut parsed.interner, &mut parsed.sink);
    check(&mut program, &mut parsed.interner, &mut parsed.sink);
    let module = program.modules.iter().find(|(_, module)| module.path.len() == 1 && parsed.interner.resolve(module.path[0]) == "main").map(|(id, _)| id).expect("a main module");
    let mut entries: Vec<_> = program.modules[module]
        .scope
        .values()
        .filter_map(|resolved| match resolved {
            Resolved::Def(def) => Some(*def),
            Resolved::Module(_) => None,
        })
        .filter(|&def| program.defs[def].module == module && program.defs[def].owner.is_none())
        .filter(|&def| program.defs[def].function().is_some_and(|function| function.has_body && function.generics.is_empty()))
        .collect();
    entries.extend(program.defs.iter().filter(|(_, def)| def.module == module && matches!(def.kind, DefKind::Test)).map(|(id, _)| id));
    entries.sort_by_key(|&def| program.defs[def].span.start);
    let entries: Vec<Instance> = entries.into_iter().map(|def| Instance { def, args: Vec::new() }).collect();
    let mut wanted = entries.clone();
    if whole_std {
        let type_args = |def: DefId| {
            let item = &program.defs[def];
            let function = item.function().filter(|function| function.has_body)?;
            let owned = match item.owner.map(|owner| &program.defs[owner].kind) {
                None => 0,
                Some(DefKind::Record(record)) => record.generics.len(),
                Some(DefKind::Enum(item)) => item.generics.len(),
                Some(_) => return None,
            };
            Some(owned + function.generics.len())
        };
        let extra: Vec<Instance> = program
            .defs
            .iter()
            .filter(|&(_, def)| def.module != module && program.modules[def.module].is_std)
            .filter_map(|(def, _)| type_args(def).map(|count| Instance { def, args: vec![TyId::INT; count] }))
            .collect();
        wanted.extend(extra);
    }
    instantiate(&mut program, &parsed.interner, &mut parsed.sink, &wanted);
    analyze(&mut program, &parsed.interner, &mut parsed.sink);
    let errors = parsed
        .sink
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .map(|diagnostic| match parsed.map.lookup(diagnostic.span.start) {
            Some(at) => format!("{}:{} {}", parsed.map.file(at.file).name, at.line, diagnostic.message),
            None => diagnostic.message.clone(),
        })
        .collect();
    Compiled { program, interner: parsed.interner, errors, entries }
}
