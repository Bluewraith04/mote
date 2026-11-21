//! Deep copies that change nothing: of a value that holds no object, and of a value built only to be copied.

use mote_base::Interner;
use mote_semantics::{DefKind, GenericArg, Payload, Program, TyId, TyKind};

use crate::ir::{Callee, Function, Op, Operand, RegId};
use crate::uses::{reads, terminator_reads, writes};

/// Whether a value of the type holds no object: only numbers, text, and structs, tuples, enums and optionals of those.
fn flat(program: &Program, ty: TyId, env: &[GenericArg], open: &mut Vec<(mote_semantics::DefId, Vec<GenericArg>)>) -> bool {
    match program.types.kind(ty) {
        TyKind::Unit | TyKind::Bool | TyKind::String | TyKind::Int(_) | TyKind::Float(_) => true,
        TyKind::Param { index, .. } => env.get(*index as usize).and_then(GenericArg::as_type).is_some_and(|arg| flat(program, arg, &[], open)),
        TyKind::FixedArray(inner, _) | TyKind::Optional(inner) => flat(program, *inner, env, open),
        TyKind::Tuple(members) | TyKind::Union(members) => members.iter().all(|&member| flat(program, member, env, open)),
        TyKind::Builtin { .. } => false,
        TyKind::Adt { def, args } => {
            if !args.iter().filter_map(GenericArg::as_type).all(|arg| flat(program, arg, env, open)) {
                return false;
            }
            if program.std.map == Some(*def) || program.std.set == Some(*def) {
                return false;
            }
            let item = &program.defs[*def];
            let key = (*def, args.clone());
            if open.contains(&key) {
                return true;
            }
            open.push(key);
            let answer = match &item.kind {
                DefKind::Record(record) => !record.is_class && record.fields.iter().all(|field| flat(program, field.ty, args, open)),
                DefKind::Enum(en) => en.variants.iter().all(|variant| match &variant.payload {
                    Payload::Unit => true,
                    Payload::Tuple(types) => types.iter().all(|&member| flat(program, member, args, open)),
                    Payload::Struct(fields) => fields.iter().all(|field| flat(program, field.ty, args, open)),
                }),
                _ => false,
            };
            open.pop();
            answer
        }
        TyKind::Infer(_) | TyKind::Function(_) | TyKind::Array(_) | TyKind::Any | TyKind::Error => false,
    }
}

fn is_deep_clone(interner: &Interner, op: &Op) -> Option<Operand> {
    match op {
        Op::Call { callee: Callee::Intrinsic(name), args, outs } if outs.is_empty() && args.len() == 1 && interner.resolve(*name) == "deep_clone" => Some(args[0]),
        _ => None,
    }
}

/// How many times each register is read and written in the function.
fn counts(function: &Function) -> (Vec<usize>, Vec<usize>) {
    let mut read = vec![0usize; function.regs.len()];
    let mut written = vec![0usize; function.regs.len()];
    for &param in &function.params {
        written[param.index()] += 1;
    }
    for &capture in &function.captures {
        written[capture.index()] += 1;
    }
    for &position in &function.out_params {
        read[function.params[position].index()] += 1;
    }
    for (_, block) in function.blocks.iter() {
        for inst in &block.insts {
            for reg in reads(&inst.op) {
                read[reg.index()] += 1;
            }
            for reg in writes(inst) {
                written[reg.index()] += 1;
            }
        }
        for reg in terminator_reads(&block.term) {
            read[reg.index()] += 1;
        }
    }
    (read, written)
}

/// Replaces a `deep_clone` that cannot be told from the original by a copy: of a flat value, or of a list, array, tuple or struct that is made once, holds only flat values, and is read by nothing else. True when something changed.
pub(super) fn elide(function: &mut Function, program: &Program, interner: &Interner) -> bool {
    let (read, written) = counts(function);
    let mut built: Vec<Option<bool>> = vec![None; function.regs.len()];
    for (_, block) in function.blocks.iter() {
        for inst in &block.insts {
            let Some(dest) = inst.dest else { continue };
            built[dest.index()] = match &inst.op {
                Op::List(items) | Op::Array(items) | Op::Tuple(items) | Op::Record(items) => Some(items.iter().all(|item| match item {
                    Operand::Const(_) => true,
                    Operand::Reg(reg) => !function.regs[*reg].cell && flat(program, function.regs[*reg].ty, &[], &mut Vec::new()),
                })),
                _ => None,
            };
        }
    }
    let mut changed = false;
    let ids: Vec<_> = function.blocks.iter().map(|(id, _)| id).collect();
    for id in ids {
        for at in 0..function.blocks[id].insts.len() {
            let inst = &function.blocks[id].insts[at];
            let (Some(original), Some(dest)) = (is_deep_clone(interner, &inst.op), inst.dest) else { continue };
            let plain = flat(program, function.regs[dest].ty, &[], &mut Vec::new());
            let fresh = matches!(original, Operand::Reg(reg) if is_fresh(function, &read, &written, &built, reg));
            if plain || fresh {
                function.blocks[id].insts[at].op = Op::Copy(original);
                changed = true;
            }
        }
    }
    changed
}

fn is_fresh(function: &Function, read: &[usize], written: &[usize], built: &[Option<bool>], reg: RegId) -> bool {
    !function.regs[reg].cell && read[reg.index()] == 1 && written[reg.index()] == 1 && built[reg.index()] == Some(true)
}
