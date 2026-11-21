//! Cleanup after lowering: inlining, propagation, folding, dead code, a simpler graph, fewer copies of arrays, and records and cells held in registers.

mod cells;
mod clones;
mod cfg;
mod constants;
mod copies;
mod dead;
mod inline;
mod meta;
mod range;
mod scalar;
mod share;
mod unroll;
#[cfg(test)]
mod tests;

use mote_base::Interner;
use mote_semantics::Program;

use crate::facts;
use crate::ir::Module;

/// The most rounds of inlining calls by name and then through closure values.
const INLINE_ROUNDS: usize = 4;

/// The most rounds of cleanup one function gets.
const ROUNDS: usize = 48;

/// Which passes run.
#[derive(Clone, Copy, Debug)]
pub struct Passes {
    pub inline: bool,
    pub share: bool,
    pub propagate: bool,
    pub fold: bool,
    pub scalar: bool,
    pub cells: bool,
    pub dead: bool,
    pub cfg: bool,
    pub reflect: bool,
    pub unroll: bool,
    pub range: bool,
    pub clones: bool,
}

impl Passes {
    pub const ALL: Passes = Passes { inline: true, share: true, propagate: true, fold: true, scalar: true, cells: true, dead: true, cfg: true, reflect: true, unroll: true, range: true, clones: true };
    pub const NONE: Passes = Passes { inline: false, share: false, propagate: false, fold: false, scalar: false, cells: false, dead: false, cfg: false, reflect: false, unroll: false, range: false, clones: false };
}

/// The configurations of a differential run: each pass alone, then some together, then all.
pub fn configurations() -> Vec<(&'static str, Passes)> {
    let none = Passes::NONE;
    vec![
        ("inline", Passes { inline: true, ..none }),
        ("share", Passes { share: true, ..none }),
        ("propagate", Passes { propagate: true, ..none }),
        ("fold", Passes { fold: true, ..none }),
        ("scalar", Passes { scalar: true, ..none }),
        ("cells", Passes { cells: true, ..none }),
        ("dead", Passes { dead: true, ..none }),
        ("cfg", Passes { cfg: true, ..none }),
        ("reflect", Passes { reflect: true, ..none }),
        ("unroll", Passes { unroll: true, ..none }),
        ("range", Passes { range: true, ..none }),
        ("clones", Passes { clones: true, ..none }),
        ("range and cleanup", Passes { range: true, propagate: true, fold: true, dead: true, cfg: true, ..none }),
        ("reflect and unroll", Passes { reflect: true, unroll: true, propagate: true, fold: true, scalar: true, dead: true, cfg: true, ..none }),
        ("propagate and fold", Passes { propagate: true, fold: true, ..none }),
        ("inline and cleanup", Passes { share: false, ..Passes::ALL }),
        ("all", Passes::ALL),
    ]
}

/// Rewrites every function so it computes the same results and faults with less work.
pub fn optimize(module: &mut Module, program: &mut Program, interner: &mut Interner) {
    optimize_with(module, program, interner, Passes::ALL);
}

/// [`optimize`] with only the passes that are set.
pub fn optimize_with(module: &mut Module, program: &mut Program, interner: &mut Interner, passes: Passes) {
    if passes.inline {
        for _ in 0..INLINE_ROUNDS {
            let mut changed = inline::run(module, interner);
            changed |= inline::closures(module);
            if !changed {
                break;
            }
        }
    }
    if passes.share {
        let all = facts::analyze(module, program, interner);
        share::share_detached(module, &all);
    }
    let table = (passes.reflect && !program.reflected.is_empty()).then(|| crate::reflect::encode(program));
    let ids: Vec<_> = module.functions.iter().map(|(id, _)| id).collect();
    for id in ids {
        let function = &mut module.functions[id];
        for _ in 0..ROUNDS {
            let mut changed = passes.propagate && copies::propagate(function);
            changed |= passes.fold && copies::fold(function, program);
            changed |= table.as_ref().is_some_and(|table| meta::fold(function, table, interner));
            changed |= passes.clones && clones::elide(function, program, interner);
            changed |= passes.unroll && unroll::run(function);
            changed |= passes.scalar && scalar::split(function, program);
            changed |= passes.cells && cells::drop_local(function);
            changed |= passes.dead && dead::remove(function, program, interner);
            changed |= passes.cfg && cfg::simplify(function);
            if !changed {
                break;
            }
        }
        if passes.range {
            range::prove(function, program, interner);
        }
    }
}
