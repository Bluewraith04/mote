//! Definite assignment: a register is written on every path to a read of it.

use super::Check;
use crate::ir::{Function, Inst};
use crate::uses::{reads, terminator_reads, writes};

/// Marks what the instruction writes.
fn write(inst: &Inst, state: &mut [bool]) {
    for reg in writes(inst) {
        if let Some(slot) = state.get_mut(reg.index()) {
            *slot = true;
        }
    }
}

impl<'a> Check<'a> {
    pub(super) fn flow(&mut self) {
        let function: &'a Function = self.function;
        let count = function.regs.len();
        let blocks = function.blocks.as_slice();
        let start = function.entry.index();
        if start >= blocks.len() {
            return;
        }
        let mut first = vec![false; count];
        for reg in function.params.iter().chain(&function.captures) {
            if let Some(slot) = first.get_mut(reg.index()) {
                *slot = true;
            }
        }
        let mut ins: Vec<Option<Vec<bool>>> = vec![None; blocks.len()];
        ins[start] = Some(first);
        let mut work = vec![start];
        while let Some(index) = work.pop() {
            let mut state = ins[index].clone().expect("a queued block has a state");
            for inst in &blocks[index].insts {
                write(inst, &mut state);
            }
            for target in blocks[index].term.targets() {
                let next = target.index();
                if next >= blocks.len() {
                    continue;
                }
                match &mut ins[next] {
                    None => {
                        ins[next] = Some(state.clone());
                        work.push(next);
                    }
                    Some(held) => {
                        let mut changed = false;
                        for (slot, &now) in held.iter_mut().zip(&state) {
                            if *slot && !now {
                                *slot = false;
                                changed = true;
                            }
                        }
                        if changed {
                            work.push(next);
                        }
                    }
                }
            }
        }
        for (index, block) in blocks.iter().enumerate() {
            let Some(mut state) = ins[index].clone() else { continue };
            for inst in &block.insts {
                for reg in reads(&inst.op) {
                    if state.get(reg.index()) == Some(&false) {
                        self.inst = Some(inst);
                        self.problem(format!("r{} is read before it is written", reg.index()));
                        self.inst = None;
                    }
                }
                write(inst, &mut state);
            }
            for reg in terminator_reads(&block.term) {
                if state.get(reg.index()) == Some(&false) {
                    self.fail(format!("r{} is read before it is written", reg.index()));
                }
            }
        }
    }
}
