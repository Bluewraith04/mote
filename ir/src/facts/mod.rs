//! Facts about each function that the passes after lowering rely on: last uses, escapes, writes and deep immutability.

mod alias;
mod immutable;
pub(crate) mod live;
#[cfg(test)]
mod tests;

use mote_base::Interner;
use mote_semantics::Program;

use crate::ir::{BlockId, FuncId, Module, RegId};

/// What a function does to each of its inputs: the parameters, then the captures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Summary {
    /// The value, or something it holds, may be reachable after the function returns.
    pub(crate) escapes: Vec<bool>,
    /// The value, or something it holds, may be written; a parameter that is assigned counts.
    pub(crate) writes: Vec<bool>,
}

/// The object an instruction makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allocation {
    /// It may be reachable after the function returns.
    pub escapes: bool,
    /// The function may write it after it is made.
    pub written: bool,
    /// It escapes nowhere, is never written, and holds only frozen objects.
    pub frozen: bool,
}

/// The facts of every function of a module.
pub struct Facts {
    functions: Vec<FunctionFacts>,
}

impl Facts {
    pub fn function(&self, func: FuncId) -> &FunctionFacts {
        &self.functions[func.index()]
    }
}

pub struct FunctionFacts {
    last_uses: Vec<Vec<Vec<RegId>>>,
    allocations: Vec<Vec<Option<Allocation>>>,
    immutable: Vec<bool>,
    summary: Summary,
}

impl FunctionFacts {
    /// The registers read for the last time by instruction `at` of the block; `at` equal to the instruction count is the terminator.
    pub fn last_uses(&self, block: BlockId, at: usize) -> &[RegId] {
        &self.last_uses[block.index()][at]
    }

    pub fn is_last_use(&self, block: BlockId, at: usize, reg: RegId) -> bool {
        self.last_uses(block, at).contains(&reg)
    }

    /// The object instruction `at` makes, if it makes one the analysis tracks.
    pub fn allocation(&self, block: BlockId, at: usize) -> Option<Allocation> {
        self.allocations[block.index()][at]
    }

    /// No code changes the value the register holds, or anything inside it, while the function runs.
    pub fn immutable(&self, reg: RegId) -> bool {
        self.immutable[reg.index()]
    }

    /// The input, by position in `params` and then `captures`, or something it holds, may be reachable after the function returns.
    pub fn input_escapes(&self, position: usize) -> bool {
        self.summary.escapes[position]
    }

    /// The input is assigned, or it or something it holds may be written.
    pub fn input_written(&self, position: usize) -> bool {
        self.summary.writes[position]
    }
}

/// Computes the facts of every function; calls are followed through what each callee does to its inputs.
pub fn analyze(module: &Module, program: &Program, interner: &Interner) -> Facts {
    let mut summaries: Vec<Summary> = module
        .functions
        .iter()
        .map(|(_, function)| {
            let inputs = function.params.len() + function.captures.len();
            Summary { escapes: vec![false; inputs], writes: vec![false; inputs] }
        })
        .collect();
    loop {
        let outcomes: Vec<alias::Outcome> = module.functions.iter().map(|(_, function)| alias::run(module, program, interner, &summaries, function)).collect();
        let next: Vec<Summary> = outcomes.iter().map(|outcome| outcome.summary.clone()).collect();
        if next == summaries {
            let mut types = immutable::Immutable::default();
            let functions = module
                .functions
                .iter()
                .zip(outcomes)
                .map(|((_, function), outcome)| FunctionFacts {
                    last_uses: live::last_uses(function),
                    immutable: function.regs.iter().zip(&outcome.frozen).map(|((_, reg), &frozen)| frozen || types.of(program, reg.ty)).collect(),
                    allocations: outcome.allocations,
                    summary: outcome.summary,
                })
                .collect();
            return Facts { functions };
        }
        summaries = next;
    }
}
