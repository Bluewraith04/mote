//! Copy elision for arrays that enter a task: a `Detach` of a value nothing writes is a plain copy.

use crate::facts::Facts;
use crate::ir::{Module, Op, Operand, RegId, Terminator};
use crate::uses::{reads, terminator_reads, writes};

/// Turns each `Detach` into a `Copy` where the source is immutable and only spawned tasks that never write it read the result.
pub(super) fn share_detached(module: &mut Module, facts: &Facts) -> bool {
    let ids: Vec<_> = module.functions.iter().map(|(id, _)| id).collect();
    let mut changed = false;
    for id in ids {
        let mut found: Vec<(crate::ir::BlockId, usize, Operand)> = Vec::new();
        let function = &module.functions[id];
        for (block, data) in function.blocks.iter() {
            for (at, inst) in data.insts.iter().enumerate() {
                let (Op::Detach(Operand::Reg(source)), Some(dest)) = (&inst.op, inst.dest) else { continue };
                if facts.function(id).immutable(*source) && only_spawned_readers(module, facts, id, dest) {
                    found.push((block, at, Operand::Reg(*source)));
                }
            }
        }
        for (block, at, source) in found {
            module.functions[id].blocks[block].insts[at].op = Op::Copy(source);
            changed = true;
        }
    }
    changed
}

/// Whether `reg` is written once and read only as a capture of a task whose body never writes that capture.
fn only_spawned_readers(module: &Module, facts: &Facts, id: crate::ir::FuncId, reg: RegId) -> bool {
    let function = &module.functions[id];
    let mut defs = 0;
    for (_, data) in function.blocks.iter() {
        if terminator_reads(&data.term).contains(&reg) || (matches!(data.term, Terminator::Return(_)) && function.out_params.iter().any(|&position| function.params[position] == reg)) {
            return false;
        }
        for inst in &data.insts {
            defs += writes(inst).filter(|&written| written == reg).count();
            if !reads(&inst.op).contains(&reg) {
                continue;
            }
            let Op::Spawn { func, captures, scope } = &inst.op else { return false };
            if scope.as_ref() == Some(&Operand::Reg(reg)) {
                return false;
            }
            let inputs = module.functions[*func].params.len();
            for (position, capture) in captures.iter().enumerate() {
                if *capture == Operand::Reg(reg) && facts.function(*func).input_written(inputs + position) {
                    return false;
                }
            }
        }
    }
    defs == 1
}
