//! Dead code: an instruction whose results are never read and that cannot fault or change anything.

use mote_base::Interner;
use mote_semantics::{BinaryOp, Program, TyKind, UnaryOp};

use crate::facts::live::{ends_with, live_in, live_out};
use crate::intrinsics;
use crate::ir::{Callee, Function, Inst, Op};
use crate::uses::{reads, writes};

/// Whether dropping the instruction, when nothing reads its results, changes no behavior.
fn removable(function: &Function, program: &Program, interner: &Interner, inst: &Inst) -> bool {
    let result = inst.dest.map(|dest| program.types.kind(function.regs[dest].ty));
    match &inst.op {
        Op::Copy(_)
        | Op::Show(_)
        | Op::Concat(_)
        | Op::Convert { .. }
        | Op::Cast(_)
        | Op::Tuple(_)
        | Op::List(_)
        | Op::Array(_)
        | Op::Record(_)
        | Op::Variant { .. }
        | Op::Range { .. }
        | Op::Field { .. }
        | Op::TupleField { .. }
        | Op::Len(_)
        | Op::NewCell(_)
        | Op::LoadCell(_)
        | Op::LoadGlobal(_)
        | Op::IsSome(_)
        | Op::SomePayload(_)
        | Op::VariantTag(_)
        | Op::IsVariant { .. }
        | Op::VariantField { .. }
        | Op::IsType { .. }
        | Op::UnionPayload { .. }
        | Op::Closure { .. }
        | Op::Stream { .. }
        | Op::FrameLoad(_)
        | Op::Detach(_) => true,
        Op::Unary { op, .. } => match op {
            UnaryOp::Neg => matches!(result, Some(TyKind::Float(_))),
            UnaryOp::Not | UnaryOp::BitNot => true,
        },
        Op::Binary { op, .. } => match op {
            BinaryOp::Eq | BinaryOp::NotEq | BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq | BinaryOp::And | BinaryOp::Or | BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => true,
            _ => matches!(result, Some(TyKind::Float(_) | TyKind::String)),
        },
        Op::Call { callee: Callee::Intrinsic(name), .. } => intrinsics::lookup(interner.resolve(*name)).is_some_and(|row| row.removable()),
        _ => false,
    }
}

/// Removes the instructions that are removable and whose results are dead; true when something went.
pub(super) fn remove(function: &mut Function, program: &Program, interner: &Interner) -> bool {
    let count = function.regs.len();
    let live_in = live_in(function);
    let ids: Vec<_> = function.blocks.iter().map(|(id, _)| id).collect();
    let mut changed = false;
    for id in ids {
        let block = &function.blocks[id];
        let mut live = live_out(block, &live_in, count);
        for reg in ends_with(function, &block.term) {
            live[reg.index()] = true;
        }
        let mut keep = vec![true; block.insts.len()];
        for (at, inst) in block.insts.iter().enumerate().rev() {
            if removable(function, program, interner, inst) && writes(inst).all(|reg| !live[reg.index()]) {
                keep[at] = false;
                continue;
            }
            for reg in writes(inst) {
                live[reg.index()] = false;
            }
            for reg in reads(&inst.op) {
                live[reg.index()] = true;
            }
        }
        if keep.iter().all(|&kept| kept) {
            continue;
        }
        changed = true;
        let mut at = 0;
        function.blocks[id].insts.retain(|_| {
            at += 1;
            keep[at - 1]
        });
    }
    changed
}
