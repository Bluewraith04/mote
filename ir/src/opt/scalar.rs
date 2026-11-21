//! Scalar replacement: a record or tuple that is only built and read or written by field lives in one register per field.

use mote_semantics::{DefKind, Program, TyId, TyKind};

use crate::ir::{BlockId, Function, Inst, Op, Operand, RegId};
use crate::uses::{reads, terminator_reads, writes};

/// The field count of a record, class or tuple type, and whether a copy of a register holding one is a separate value.
fn shape(program: &Program, ty: TyId) -> Option<(usize, bool)> {
    match program.types.kind(ty) {
        TyKind::Tuple(elements) => Some((elements.len(), true)),
        TyKind::Adt { def, .. } => match &program.defs[*def].kind {
            DefKind::Record(record) => Some((record.fields.len(), !record.is_class)),
            _ => None,
        },
        _ => None,
    }
}

/// The field types with the type arguments applied.
fn field_types(program: &mut Program, ty: TyId) -> Vec<TyId> {
    match program.types.kind(ty).clone() {
        TyKind::Tuple(elements) => elements,
        TyKind::Adt { def, args } => {
            let declared: Vec<TyId> = program.defs[def].record().map(|record| record.fields.iter().map(|field| field.ty).collect()).unwrap_or_default();
            declared.into_iter().map(|field| program.types.subst_generic(field, &args)).collect()
        }
        _ => Vec::new(),
    }
}

/// The registers an op reads other than as the object of a field access or the source of a copy.
fn whole_reads(op: &Op) -> Vec<RegId> {
    match op {
        Op::Field { .. } | Op::TupleField { .. } | Op::Copy(_) => Vec::new(),
        Op::SetField { value: Operand::Reg(value), .. } => vec![*value],
        Op::SetField { .. } => Vec::new(),
        _ => reads(op),
    }
}

/// The registers that can be split: every definition builds the value or copies another such register, and every use is a field access.
fn candidates(function: &Function, program: &Program) -> Vec<Option<(usize, bool)>> {
    let mut found: Vec<Option<(usize, bool)>> = function.regs.iter().map(|(_, reg)| if reg.cell { None } else { shape(program, reg.ty) }).collect();
    for &input in function.params.iter().chain(&function.captures) {
        found[input.index()] = None;
    }
    let mut edges: Vec<(RegId, RegId)> = Vec::new();
    for (_, block) in function.blocks.iter() {
        for inst in &block.insts {
            for reg in whole_reads(&inst.op) {
                found[reg.index()] = None;
            }
            if let Op::Field { object: Operand::Reg(object), index } | Op::TupleField { object: Operand::Reg(object), index } | Op::SetField { object: Operand::Reg(object), index, .. } = &inst.op
                && found[object.index()].is_some_and(|(count, _)| *index as usize >= count)
            {
                found[object.index()] = None;
            }
            if let Some(dest) = inst.dest {
                match &inst.op {
                    Op::Record(items) | Op::Tuple(items) if found[dest.index()].is_some_and(|(count, _)| count == items.len()) => {}
                    Op::Copy(Operand::Reg(source)) => edges.push((dest, *source)),
                    _ => found[dest.index()] = None,
                }
            }
            if let Op::Call { outs, .. } = &inst.op {
                for out in outs {
                    found[out.index()] = None;
                }
            }
        }
        for reg in terminator_reads(&block.term) {
            found[reg.index()] = None;
        }
    }
    for &position in &function.out_params {
        found[function.params[position].index()] = None;
    }
    loop {
        let mut changed = false;
        for &(dest, source) in &edges {
            let both = matches!((found[dest.index()], found[source.index()]), (Some((_, true)), Some((_, true))));
            if !both && (found[dest.index()].is_some() || found[source.index()].is_some()) {
                found[dest.index()] = None;
                found[source.index()] = None;
                changed = true;
            }
        }
        if !changed {
            return found;
        }
    }
}

/// Where an instruction of a split register is rewritten to; `None` when it does not touch one.
fn rewrite(inst: &Inst, fields: &[Option<Vec<RegId>>]) -> Option<Vec<Inst>> {
    let copy = |dest: RegId, value: Operand| Inst { dest: Some(dest), op: Op::Copy(value), span: inst.span };
    let held = |operand: &Operand| match operand {
        Operand::Reg(reg) => fields[reg.index()].as_ref(),
        Operand::Const(_) => None,
    };
    match (&inst.op, inst.dest) {
        (Op::Record(items) | Op::Tuple(items), Some(dest)) => {
            let regs = fields[dest.index()].as_ref()?;
            Some(regs.iter().zip(items).map(|(&field, &item)| copy(field, item)).collect())
        }
        (Op::Copy(source), Some(dest)) => {
            let regs = fields[dest.index()].as_ref()?;
            let from = held(source)?;
            Some(regs.iter().zip(from).filter(|(to, from)| to != from).map(|(&to, &from)| copy(to, Operand::Reg(from))).collect())
        }
        (Op::Field { object, index } | Op::TupleField { object, index }, Some(dest)) => {
            let regs = held(object)?;
            Some(vec![copy(dest, Operand::Reg(regs[*index as usize]))])
        }
        (Op::SetField { object, index, value }, None) => {
            let regs = held(object)?;
            Some(vec![copy(regs[*index as usize], *value)])
        }
        _ => None,
    }
}

/// Splits the registers that only hold fields; true when something changed.
pub(super) fn split(function: &mut Function, program: &mut Program) -> bool {
    let found = candidates(function, program);
    let mut mentioned = vec![false; found.len()];
    for (_, block) in function.blocks.iter() {
        for inst in &block.insts {
            for reg in writes(inst).chain(reads(&inst.op)) {
                mentioned[reg.index()] = true;
            }
        }
    }
    let mut fields: Vec<Option<Vec<RegId>>> = vec![None; found.len()];
    for index in 0..found.len() {
        if found[index].is_none() || !mentioned[index] {
            continue;
        }
        let ty = function.regs.as_slice()[index].ty;
        let types = field_types(program, ty);
        fields[index] = Some(types.into_iter().map(|ty| function.regs.alloc(crate::ir::Reg { ty, name: None, cell: false })).collect());
    }
    if fields.iter().all(Option::is_none) {
        return false;
    }
    let ids: Vec<BlockId> = function.blocks.iter().map(|(id, _)| id).collect();
    for id in ids {
        let old = std::mem::take(&mut function.blocks[id].insts);
        let mut insts = Vec::with_capacity(old.len());
        for inst in old {
            match rewrite(&inst, &fields) {
                Some(replaced) => insts.extend(replaced),
                None => insts.push(inst),
            }
        }
        function.blocks[id].insts = insts;
    }
    true
}
