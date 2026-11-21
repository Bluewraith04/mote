//! A read of the reflection table at a constant position becomes the constant.

use std::collections::HashMap;

use mote_base::Interner;

use crate::ir::{Callee, Const, Function, Op, Operand, RegId};
use crate::reflect::Table;
use crate::uses::writes;

/// Which array of the table a register holds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Array {
    Ints,
    Texts,
}

/// Replaces `meta_ints()[k]` and `meta_texts()[k]` with a constant `k`; true when something changed.
pub(super) fn fold(function: &mut Function, table: &Table, interner: &mut Interner) -> bool {
    // A register is an array of the table when every write to it is the call that makes that array.
    let mut found: HashMap<RegId, Option<Array>> = HashMap::new();
    for (_, block) in function.blocks.iter() {
        for inst in &block.insts {
            let made = match (&inst.op, inst.dest) {
                (Op::Call { callee: Callee::Intrinsic(name), args, .. }, Some(_)) if args.is_empty() => match interner.resolve(*name) {
                    "meta_ints" => Some(Array::Ints),
                    "meta_texts" => Some(Array::Texts),
                    _ => None,
                },
                _ => None,
            };
            for reg in writes(inst) {
                let kept = found.entry(reg).or_insert(made);
                if *kept != made {
                    *kept = None;
                }
            }
        }
    }
    let mut arrays: HashMap<RegId, Array> = found.into_iter().filter_map(|(reg, array)| array.filter(|_| !function.params.contains(&reg) && !function.regs[reg].cell).map(|array| (reg, array))).collect();
    for (_, block) in function.blocks.iter() {
        for inst in &block.insts {
            if let Op::SetIndex { object: Operand::Reg(object), .. } | Op::SetSlice { object: Operand::Reg(object), .. } = &inst.op {
                arrays.remove(object);
            }
        }
    }
    if arrays.is_empty() {
        return false;
    }
    let mut changed = false;
    let ids: Vec<_> = function.blocks.iter().map(|(id, _)| id).collect();
    for id in ids {
        for position in 0..function.blocks[id].insts.len() {
            let inst = &function.blocks[id].insts[position];
            let Op::Index { object: Operand::Reg(object), index: Operand::Const(Const::Int(at)) } = &inst.op else { continue };
            let Some(array) = arrays.get(object) else { continue };
            let Ok(at) = usize::try_from(*at) else { continue };
            let folded = match array {
                Array::Ints => table.ints.get(at).map(|&n| Const::Int(n)),
                Array::Texts => table.texts.get(at).map(|text| Const::String(interner.intern(text))),
            };
            if let Some(folded) = folded {
                function.blocks[id].insts[position].op = Op::Copy(Operand::Const(folded));
                changed = true;
            }
        }
    }
    changed
}
