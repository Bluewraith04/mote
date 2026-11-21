//! Liveness: a register is live where a read of it can follow before a write.

use crate::ir::{Block, Function, RegId, Terminator};
use crate::uses::{reads, terminator_reads, writes};

/// The registers a block ends by reading: the terminator's, and at a `Return` the `var` parameters that go back.
pub(crate) fn ends_with(function: &Function, term: &Terminator) -> Vec<RegId> {
    let mut regs = terminator_reads(term);
    if matches!(term, Terminator::Return(_)) {
        regs.extend(function.out_params.iter().map(|&position| function.params[position]));
    }
    regs
}

/// Adds each register of `regs` that is not live to `into`, once.
fn take(live: &[bool], regs: &[RegId], into: &mut Vec<RegId>) {
    for &reg in regs {
        if !live[reg.index()] && !into.contains(&reg) {
            into.push(reg);
        }
    }
}

/// The registers live after the last instruction of the block, given each block's live-in.
pub(crate) fn live_out(block: &Block, live_in: &[Vec<bool>], count: usize) -> Vec<bool> {
    let mut live = vec![false; count];
    for target in block.term.targets() {
        for (slot, &now) in live.iter_mut().zip(&live_in[target.index()]) {
            *slot |= now;
        }
    }
    live
}

/// Per block: the registers live on entry.
pub(crate) fn live_in(function: &Function) -> Vec<Vec<bool>> {
    let count = function.regs.len();
    let blocks = function.blocks.as_slice();
    let mut live_in: Vec<Vec<bool>> = vec![vec![false; count]; blocks.len()];
    let mut changed = true;
    while changed {
        changed = false;
        for (index, block) in blocks.iter().enumerate().rev() {
            let mut live = live_out(block, &live_in, count);
            for reg in ends_with(function, &block.term) {
                live[reg.index()] = true;
            }
            for inst in block.insts.iter().rev() {
                for reg in writes(inst) {
                    live[reg.index()] = false;
                }
                for reg in reads(&inst.op) {
                    live[reg.index()] = true;
                }
            }
            if live != live_in[index] {
                live_in[index] = live;
                changed = true;
            }
        }
    }
    live_in
}

/// Per block and instruction: the registers live right after it.
pub(crate) fn live_after(function: &Function) -> Vec<Vec<Vec<RegId>>> {
    let count = function.regs.len();
    let live_in = live_in(function);
    function
        .blocks
        .as_slice()
        .iter()
        .map(|block| {
            let mut live = live_out(block, &live_in, count);
            for reg in ends_with(function, &block.term) {
                live[reg.index()] = true;
            }
            let mut after: Vec<Vec<RegId>> = vec![Vec::new(); block.insts.len()];
            for (at, inst) in block.insts.iter().enumerate().rev() {
                after[at] = function.regs.iter().filter(|(id, _)| live[id.index()]).map(|(id, _)| id).collect();
                for reg in writes(inst) {
                    live[reg.index()] = false;
                }
                for reg in reads(&inst.op) {
                    live[reg.index()] = true;
                }
            }
            after
        })
        .collect()
}

/// Per block, per instruction and then the terminator: the registers read there for the last time.
pub(super) fn last_uses(function: &Function) -> Vec<Vec<Vec<RegId>>> {
    let count = function.regs.len();
    let live_in = live_in(function);
    function
        .blocks
        .as_slice()
        .iter()
        .map(|block| {
            let mut live = live_out(block, &live_in, count);
            let mut lasts: Vec<Vec<RegId>> = vec![Vec::new(); block.insts.len() + 1];
            let end = ends_with(function, &block.term);
            take(&live, &end, &mut lasts[block.insts.len()]);
            for reg in end {
                live[reg.index()] = true;
            }
            for (at, inst) in block.insts.iter().enumerate().rev() {
                for reg in writes(inst) {
                    live[reg.index()] = false;
                }
                let read = reads(&inst.op);
                take(&live, &read, &mut lasts[at]);
                for reg in read {
                    live[reg.index()] = true;
                }
            }
            lasts
        })
        .collect()
}
