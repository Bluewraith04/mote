//! Compiles Mote source against the real `std/src`, for the tests of this and later crates.

use std::path::Path;

use mote_base::{Interner, Severity};

use crate::analyze::analyze;
use crate::check::check;
use crate::defs::{DefId, DefKind, GenericParam, ModuleId, Resolved};
use crate::instantiate::instantiate;
use crate::program::{Instance, Program};
use crate::resolve::resolve;
use crate::testing::parse_sources;
use crate::types::{ConstArg, ConstVal, GenericArg, IntTy, TyId, TyKind};

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
    collect(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../mote_std/std/src"), &[], &mut sources);
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
        let int_arr = program.types.array(TyId::INT);
        let bool_arr = program.types.array(TyId::BOOL);
        let nested_arr = program.types.array(int_arr);
        let pair_ty = program.types.tuple(vec![TyId::INT, TyId::INT]);
        let pair_arr = program.types.array(pair_ty);
        let dummy_arg = |param: &GenericParam| -> GenericArg {
            match param {
                GenericParam::Type { .. } => GenericArg::Type(TyId::INT),
                GenericParam::Const { ty, .. } => match program.types.kind(*ty) {
                    TyKind::Bool => GenericArg::Const(ConstArg::Val(ConstVal::Bool(false))),
                    TyKind::Int(i) => GenericArg::Const(ConstArg::Val(ConstVal::Int(1, *i))),
                    _ => GenericArg::Const(ConstArg::Val(ConstVal::Int(1, IntTy::I64))),
                },
            }
        };
        let std_array_args = |name: &str, count: usize| -> Option<Vec<GenericArg>> {
            if count == 0 {
                return Some(Vec::new());
            }
            let tys = match name {
                "full" | "from_fn" => vec![TyId::INT],
                "all" | "any" | "none" => vec![bool_arr],
                "select" => vec![bool_arr, int_arr],
                "flatten" => vec![nested_arr, TyId::INT],
                "zip" => vec![int_arr, TyId::INT, int_arr, TyId::INT, pair_arr],
                "enumerate" => vec![int_arr, TyId::INT, pair_arr],
                "concat" => vec![int_arr, int_arr, TyId::INT],
                "scan" | "map" => vec![int_arr, TyId::INT, TyId::INT, int_arr],
                "zip_with" => vec![int_arr, TyId::INT, int_arr, TyId::INT, TyId::INT, int_arr],
                _ => std::iter::once(int_arr).chain(std::iter::repeat_n(TyId::INT, count.saturating_sub(1))).collect(),
            };
            Some(tys.into_iter().map(GenericArg::Type).collect())
        };
        let is_std_array = |m: ModuleId| -> bool {
            let p = &program.modules[m].path;
            p.len() == 2 && parsed.interner.resolve(p[0]) == "std" && parsed.interner.resolve(p[1]) == "array"
        };
        let type_args = |def: DefId| -> Option<Vec<GenericArg>> {
            let item = &program.defs[def];
            let function = item.function().filter(|function| function.has_body && function.link.is_none())?;
            if item.extend.is_none() && item.owner.is_none() && is_std_array(item.module) {
                return std_array_args(parsed.interner.resolve(item.name), function.generics.len());
            }
            let mut out = Vec::new();
            if let Some(ext) = item.extend {
                out.extend(program.extends[ext].generics.iter().map(dummy_arg));
            } else if let Some(owner) = item.owner {
                match &program.defs[owner].kind {
                    DefKind::Record(record) => out.extend(record.generics.iter().map(dummy_arg)),
                    DefKind::Enum(en) => out.extend(en.generics.iter().map(dummy_arg)),
                    _ => return None,
                }
            }
            out.extend(function.generics.iter().map(dummy_arg));
            Some(out)
        };
        let extra: Vec<Instance> = program
            .defs
            .iter()
            .filter(|&(_, def)| def.module != module && program.modules[def.module].is_std)
            .filter_map(|(def, _)| type_args(def).map(|args| Instance { def, args }))
            .collect();
        wanted.extend(extra);
    }
    instantiate(&mut program, &mut parsed.interner, &mut parsed.sink, &wanted);
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
