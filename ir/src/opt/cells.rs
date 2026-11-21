//! Cells: a cell that only this function makes, loads and stores is a plain register.

use crate::ir::{Const, Function, Inst, Op, Operand, RegId};
use crate::uses::{reads, terminator_reads};

/// The cells that are made, loaded and stored here and used nowhere else.
fn local(function: &Function) -> Vec<bool> {
    let mut found: Vec<bool> = function.regs.iter().map(|(_, reg)| reg.cell).collect();
    for &input in function.params.iter().chain(&function.captures) {
        found[input.index()] = false;
    }
    for &position in &function.out_params {
        found[function.params[position].index()] = false;
    }
    for (_, block) in function.blocks.iter() {
        for inst in &block.insts {
            let loose: Vec<RegId> = match &inst.op {
                Op::LoadCell(_) | Op::StoreCell { value: Operand::Const(_), .. } => Vec::new(),
                Op::StoreCell { value: Operand::Reg(value), .. } => vec![*value],
                op => reads(op),
            };
            for reg in loose {
                found[reg.index()] = false;
            }
            if let Some(dest) = inst.dest
                && !matches!(&inst.op, Op::NewCell(value) if *value != Operand::Const(Const::Null))
            {
                found[dest.index()] = false;
            }
            if let Op::Call { outs, .. } = &inst.op {
                for out in outs {
                    found[out.index()] = false;
                }
            }
        }
        for reg in terminator_reads(&block.term) {
            found[reg.index()] = false;
        }
    }
    found
}

/// Turns the local cells into registers; true when one went.
pub(super) fn drop_local(function: &mut Function) -> bool {
    let found = local(function);
    if !found.iter().any(|&local| local) {
        return false;
    }
    let ids: Vec<_> = function.blocks.iter().map(|(id, _)| id).collect();
    for id in ids {
        for inst in &mut function.blocks[id].insts {
            let replaced = match &inst.op {
                Op::NewCell(value) if inst.dest.is_some_and(|dest| found[dest.index()]) => Some(Inst { dest: inst.dest, op: Op::Copy(*value), span: inst.span }),
                Op::LoadCell(Operand::Reg(cell)) if found[cell.index()] => Some(Inst { dest: inst.dest, op: Op::Copy(Operand::Reg(*cell)), span: inst.span }),
                Op::StoreCell { cell: Operand::Reg(cell), value } if found[cell.index()] => Some(Inst { dest: Some(*cell), op: Op::Copy(*value), span: inst.span }),
                _ => None,
            };
            if let Some(replaced) = replaced {
                *inst = replaced;
            }
        }
    }
    let regs: Vec<RegId> = function.regs.iter().map(|(id, _)| id).collect();
    for reg in regs {
        if found[reg.index()] {
            function.regs[reg].cell = false;
        }
    }
    true
}
