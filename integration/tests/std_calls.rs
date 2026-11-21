//! Every public function of `std` is reached by a program the gates compile and run.

mod common;

use std::collections::BTreeSet;

use mote::{Options, Pipeline, Target};
use mote_packages::testing::Scratch;
use mote_semantics::{DefId, DefKind};

/// `module.Owner.name` of a definition.
fn qualified(pipeline: &Pipeline, def: DefId) -> String {
    let program = &pipeline.program;
    let item = &program.defs[def];
    let mut parts: Vec<&str> = program.modules[item.module].path.iter().map(|part| pipeline.interner.resolve(*part)).collect();
    if let Some(owner) = item.owner {
        parts.push(pipeline.interner.resolve(program.defs[owner].name));
    }
    parts.push(pipeline.interner.resolve(item.name));
    parts.join(".")
}

/// The public functions and methods `std` declares that have a body or an extern declaration.
fn declared(pipeline: &Pipeline) -> BTreeSet<String> {
    let program = &pipeline.program;
    program
        .defs
        .iter()
        .filter(|(_, item)| item.is_pub && item.origin.is_some() && program.modules[item.module].is_std)
        .filter(|(_, item)| !(program.modules[item.module].path.len() == 2 && pipeline.interner.resolve(program.modules[item.module].path[1]) == "array"))
        .filter(|(_, item)| item.owner.is_none_or(|owner| program.defs[owner].is_pub && !matches!(program.defs[owner].kind, DefKind::Trait(_))))
        .filter(|(_, item)| matches!(&item.kind, DefKind::Function(function) if function.has_body || function.link.is_some()))
        .map(|(def, _)| qualified(pipeline, def))
        .collect()
}

/// The functions a program instantiates.
fn reached(pipeline: &Pipeline) -> BTreeSet<String> {
    pipeline.program.order.iter().map(|instance| qualified(pipeline, instance.def)).collect()
}

fn built(source: &str, target: Target) -> Option<Pipeline> {
    let scratch = Scratch::new();
    let dir = scratch.package("calls", "calls", &[], &[("main.mote", source)]);
    Pipeline::build(&dir, target, &Options { optimize: false, ..Options::default() }).ok()
}

#[test]
fn std_calls_every_public_function() {
    let mut sources: Vec<(Target, String)> = Vec::new();
    for path in common::files_with("mote") {
        let relative = common::relative(&path);
        if relative.starts_with("mote_std/std/tests/") {
            sources.push((Target::Test, std::fs::read_to_string(&path).expect("a readable file")));
        } else if relative.starts_with("integration/corpus/") {
            sources.push((Target::Run, std::fs::read_to_string(&path).expect("a readable file")));
        }
    }
    let (blocks, verdicts) = common::blocks::judged();
    for (_, verdict) in blocks.iter().zip(verdicts) {
        if let common::blocks::Verdict::Runs(source) = verdict {
            sources.push((common::blocks::target_of(source), source.clone()));
            if common::blocks::has_tests(source) {
                sources.push((Target::Check, source.clone()));
            }
        }
    }
    let found = common::blocks::in_parallel(&sources, |_, (target, source)| built(source, *target).map(|pipeline| (declared(&pipeline), reached(&pipeline))));
    let mut all = BTreeSet::new();
    let mut called = BTreeSet::new();
    for (declared, reached) in found.into_iter().flatten() {
        all.extend(declared);
        called.extend(reached);
    }
    assert!(all.len() > 300, "{} functions declared", all.len());
    let uncalled: Vec<&String> = all.iter().filter(|name| !called.contains(*name)).collect();
    println!("{} public functions, {} not called", all.len(), uncalled.len());
    assert_eq!(uncalled, Vec::<&String>::new());
}
