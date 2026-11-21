//! Copy and constant propagation by available copies, and constant folding.

use std::collections::HashMap;

use mote_semantics::{Program, TyKind};

use super::constants;
use crate::ir::{Function, Inst, Op, Operand, RegId, Terminator};
use crate::uses::{operands_mut, terminator_operands_mut, writes};

/// What a register is known to equal.
type Known = HashMap<RegId, Operand>;

/// What an operand position may be replaced by.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// The operand names the object the op writes.
    Keep,
    Reg,
    Any,
}

fn slots(op: &Op, count: usize) -> Vec<Slot> {
    let mut out = vec![Slot::Reg; count];
    match op {
        Op::Copy(_) | Op::Unary { .. } | Op::Binary { .. } => out.fill(Slot::Any),
        Op::Call { callee: crate::ir::Callee::Function(_), args, .. } => out[..args.len()].fill(Slot::Any),
        Op::Index { .. } => out[1] = Slot::Any,
        Op::SetField { .. } | Op::SetIndex { .. } | Op::SetSlice { .. } | Op::StoreCell { .. } => out[0] = Slot::Keep,
        _ => {}
    }
    out
}

fn resolve(known: &Known, operand: Operand) -> Operand {
    match operand {
        Operand::Reg(reg) => known.get(&reg).copied().unwrap_or(operand),
        constant => constant,
    }
}

fn kill(known: &mut Known, reg: RegId) {
    known.remove(&reg);
    known.retain(|_, value| *value != Operand::Reg(reg));
}

/// Updates what is known after the instruction.
fn transfer(function: &Function, inst: &Inst, known: &mut Known) {
    if let Op::SetField { object: Operand::Reg(object), .. } | Op::SetIndex { object: Operand::Reg(object), .. } | Op::SetSlice { object: Operand::Reg(object), .. } | Op::StoreCell { cell: Operand::Reg(object), .. } = &inst.op {
        kill(known, *object);
    }
    for reg in writes(inst) {
        kill(known, reg);
    }
    if let (Some(dest), Op::Copy(source)) = (inst.dest, &inst.op) {
        let source = resolve(known, *source);
        let fits = match source {
            Operand::Reg(reg) => reg != dest && function.regs[reg].ty == function.regs[dest].ty && function.regs[reg].cell == function.regs[dest].cell,
            Operand::Const(_) => !function.regs[dest].cell,
        };
        if fits {
            known.insert(dest, source);
        }
    }
}

fn meet(held: &mut Known, other: &Known) -> bool {
    let before = held.len();
    held.retain(|reg, value| other.get(reg) == Some(value));
    held.len() != before
}

/// Replaces each read of a register by what it is known to equal; true when something changed.
pub(super) fn propagate(function: &mut Function) -> bool {
    let count = function.blocks.len();
    let ids: Vec<_> = function.blocks.iter().map(|(id, _)| id).collect();
    let mut ins: Vec<Option<Known>> = vec![None; count];
    ins[function.entry.index()] = Some(Known::new());
    let mut work = vec![function.entry.index()];
    while let Some(index) = work.pop() {
        let mut known = ins[index].clone().expect("a queued block has a state");
        for inst in &function.blocks[ids[index]].insts {
            transfer(function, inst, &mut known);
        }
        for target in function.blocks[ids[index]].term.targets() {
            let next = target.index();
            match &mut ins[next] {
                None => {
                    ins[next] = Some(known.clone());
                    work.push(next);
                }
                Some(held) => {
                    if meet(held, &known) {
                        work.push(next);
                    }
                }
            }
        }
    }
    let mut changed = false;
    for (index, id) in ids.iter().enumerate() {
        let Some(mut known) = ins[index].clone() else { continue };
        let mut block = std::mem::replace(&mut function.blocks[*id], crate::ir::Block { insts: Vec::new(), term: Terminator::Unreachable });
        for inst in &mut block.insts {
            let count = operands_mut(&mut inst.op).len();
            let kinds = slots(&inst.op, count);
            for (slot, operand) in kinds.into_iter().zip(operands_mut(&mut inst.op)) {
                changed |= rewrite(operand, slot, &known);
            }
            transfer(function, inst, &mut known);
        }
        for operand in terminator_operands_mut(&mut block.term) {
            changed |= rewrite(operand, Slot::Any, &known);
        }
        function.blocks[*id] = block;
    }
    changed
}

fn rewrite(operand: &mut Operand, slot: Slot, known: &Known) -> bool {
    if slot == Slot::Keep {
        return false;
    }
    let Operand::Reg(reg) = operand else { return false };
    match known.get(reg) {
        Some(Operand::Const(constant)) if slot == Slot::Any => {
            *operand = Operand::Const(*constant);
            true
        }
        Some(Operand::Reg(source)) => {
            *operand = Operand::Reg(*source);
            true
        }
        _ => false,
    }
}

/// Replaces an operation on constants that cannot fault by its result; true when something changed.
pub(super) fn fold(function: &mut Function, program: &Program) -> bool {
    let mut changed = false;
    let ids: Vec<_> = function.blocks.iter().map(|(id, _)| id).collect();
    for id in ids {
        for position in 0..function.blocks[id].insts.len() {
            let inst = &function.blocks[id].insts[position];
            let Some(dest) = inst.dest else { continue };
            let kind = program.types.kind(function.regs[dest].ty);
            let result = match &inst.op {
                Op::Binary { op, left: Operand::Const(left), right: Operand::Const(right) } => constants::binary(*op, left, right, kind),
                Op::Unary { op, operand: Operand::Const(operand) } => constants::unary(*op, operand, kind),
                Op::Cast(Operand::Const(operand)) if !matches!(kind, TyKind::Adt { .. }) => constants::cast(operand, kind),
                _ => continue,
            };
            if let Some(folded) = result {
                function.blocks[id].insts[position].op = Op::Copy(Operand::Const(folded));
                changed = true;
            }
        }
    }
    changed
}
