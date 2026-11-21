//! The registers an instruction or a terminator reads and writes.

use mote_base::Symbol;

use crate::ir::{Callee, Const, Function, Inst, Op, Operand, RegId, Terminator};

/// The text constants a function reads, each once, in the order they are first read.
pub fn string_constants(function: &Function) -> Vec<Symbol> {
    let mut found = Vec::new();
    let mut note = |operand: &Operand| {
        if let Operand::Const(Const::String(text)) = operand
            && !found.contains(text)
        {
            found.push(*text);
        }
    };
    for (_, block) in function.blocks.iter() {
        for inst in &block.insts {
            let mut op = inst.op.clone();
            operands_mut(&mut op).into_iter().for_each(|operand| note(operand));
        }
        let mut term = block.term.clone();
        terminator_operands_mut(&mut term).into_iter().for_each(|operand| note(operand));
    }
    found
}

fn add(out: &mut Vec<RegId>, operand: &Operand) {
    if let Operand::Reg(reg) = operand {
        out.push(*reg);
    }
}

fn add_all<'o>(out: &mut Vec<RegId>, operands: impl IntoIterator<Item = &'o Operand>) {
    for operand in operands {
        add(out, operand);
    }
}

/// The registers an op reads, with a repeat for each operand.
pub(crate) fn reads(op: &Op) -> Vec<RegId> {
    let mut out = Vec::new();
    match op {
        Op::Copy(a) | Op::Len(a) | Op::VariantTag(a) | Op::Cast(a) | Op::Show(a) | Op::NewCell(a) | Op::LoadCell(a) | Op::IsSome(a) | Op::SomePayload(a) | Op::Unwrap(a) | Op::ScopeEnd(a) | Op::Yield(a) | Op::Detach(a) => add(&mut out, a),
        Op::Unary { operand, .. } | Op::Convert { operand, .. } | Op::IsType { operand, .. } | Op::UnionPayload { operand, .. } => add(&mut out, operand),
        Op::Binary { left, right, .. } => add_all(&mut out, [left, right]),
        Op::Concat(items) | Op::Tuple(items) | Op::List(items) | Op::Array(items) | Op::Record(items) => add_all(&mut out, items),
        Op::Variant { fields, .. } => add_all(&mut out, fields),
        Op::ArrayRepeat { value, len } => add_all(&mut out, [value, len]),
        Op::Range { start, end, .. } => add_all(&mut out, [start, end]),
        Op::Field { object, .. } | Op::TupleField { object, .. } | Op::IsVariant { object, .. } | Op::VariantField { object, .. } => add(&mut out, object),
        Op::SetField { object, value, .. } => add_all(&mut out, [object, value]),
        Op::Index { object, index } => add_all(&mut out, [object, index]),
        Op::SetIndex { object, index, value } => add_all(&mut out, [object, index, value]),
        Op::Slice { object, start, end, step, .. } => add_all(&mut out, [object].into_iter().chain(start).chain(end).chain(step)),
        Op::SetSlice { object, start, end, value, .. } => add_all(&mut out, [object, value].into_iter().chain(start).chain(end)),
        Op::StoreCell { cell, value } => add_all(&mut out, [cell, value]),
        Op::LoadGlobal(_) | Op::ScopeBegin | Op::Undefer | Op::FrameLoad(_) => {}
        Op::FrameStore { value, .. } => add(&mut out, value),
        Op::Stream { init, .. } => add_all(&mut out, init),
        Op::Defer { resource, .. } => add(&mut out, resource),
        Op::StoreGlobal { value, .. } => add(&mut out, value),
        Op::Call { callee, args, .. } => {
            add_all(&mut out, args);
            if let Callee::Value(operand) = callee {
                add(&mut out, operand);
            }
        }
        Op::Closure { captures, .. } => add_all(&mut out, captures),
        Op::Spawn { captures, scope, .. } => {
            add_all(&mut out, captures);
            add_all(&mut out, scope);
        }
    }
    out
}

/// Every operand of an op, in the order `reads` lists them.
pub(crate) fn operands_mut(op: &mut Op) -> Vec<&mut Operand> {
    match op {
        Op::Copy(a) | Op::Len(a) | Op::VariantTag(a) | Op::Cast(a) | Op::Show(a) | Op::NewCell(a) | Op::LoadCell(a) | Op::IsSome(a) | Op::SomePayload(a) | Op::Unwrap(a) | Op::ScopeEnd(a) | Op::Yield(a) | Op::Detach(a) => vec![a],
        Op::Unary { operand, .. } | Op::Convert { operand, .. } | Op::IsType { operand, .. } | Op::UnionPayload { operand, .. } => vec![operand],
        Op::Binary { left, right, .. } => vec![left, right],
        Op::Concat(items) | Op::Tuple(items) | Op::List(items) | Op::Array(items) | Op::Record(items) => items.iter_mut().collect(),
        Op::Variant { fields, .. } => fields.iter_mut().collect(),
        Op::ArrayRepeat { value, len } => vec![value, len],
        Op::Range { start, end, .. } => vec![start, end],
        Op::Field { object, .. } | Op::TupleField { object, .. } | Op::IsVariant { object, .. } | Op::VariantField { object, .. } => vec![object],
        Op::SetField { object, value, .. } => vec![object, value],
        Op::Index { object, index } => vec![object, index],
        Op::SetIndex { object, index, value } => vec![object, index, value],
        Op::Slice { object, start, end, step, .. } => [object].into_iter().chain(start.iter_mut()).chain(end.iter_mut()).chain(step.iter_mut()).collect(),
        Op::SetSlice { object, start, end, value, .. } => [object, value].into_iter().chain(start.iter_mut()).chain(end.iter_mut()).collect(),
        Op::StoreCell { cell, value } => vec![cell, value],
        Op::LoadGlobal(_) | Op::ScopeBegin | Op::Undefer | Op::FrameLoad(_) => Vec::new(),
        Op::FrameStore { value, .. } => vec![value],
        Op::Stream { init, .. } => init.iter_mut().collect(),
        Op::Defer { resource, .. } => vec![resource],
        Op::StoreGlobal { value, .. } => vec![value],
        Op::Call { callee, args, .. } => {
            let mut out: Vec<&mut Operand> = args.iter_mut().collect();
            if let Callee::Value(operand) = callee {
                out.push(operand);
            }
            out
        }
        Op::Closure { captures, .. } => captures.iter_mut().collect(),
        Op::Spawn { captures, scope, .. } => captures.iter_mut().chain(scope.iter_mut()).collect(),
    }
}

/// The operands of a terminator.
pub(crate) fn terminator_operands_mut(term: &mut Terminator) -> Vec<&mut Operand> {
    match term {
        Terminator::Branch { condition, .. } => vec![condition],
        Terminator::Switch { value, .. } => vec![value],
        Terminator::Return(Some(value)) => vec![value],
        Terminator::Jump(_) | Terminator::Return(None) | Terminator::Unreachable => Vec::new(),
    }
}

/// The registers a terminator reads.
pub(crate) fn terminator_reads(term: &Terminator) -> Vec<RegId> {
    let mut out = Vec::new();
    match term {
        Terminator::Branch { condition, .. } => add(&mut out, condition),
        Terminator::Switch { value, .. } => add(&mut out, value),
        Terminator::Return(Some(value)) => add(&mut out, value),
        Terminator::Jump(_) | Terminator::Return(None) | Terminator::Unreachable => {}
    }
    out
}

/// The registers an instruction writes: its destination, then the `outs` of a call.
pub(crate) fn writes(inst: &Inst) -> impl Iterator<Item = RegId> + '_ {
    let outs: &[RegId] = match &inst.op {
        Op::Call { outs, .. } => outs,
        _ => &[],
    };
    inst.dest.iter().chain(outs).copied()
}
