//! Generators as state machines: a call builds a heap frame, and a resume function runs it to the next `yield`.

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use mote_base::{Interner, Span};
use mote_semantics::{Conversion, Program, TyId, TyKind};

use crate::builder::FunctionBuilder;
use crate::facts::live::live_after;
use crate::ir::{Block, BlockId, Callee, Const, FrameSlot, FuncId, Function, FunctionKind, Inst, Module, Op, Operand, Reg, RegId, Terminator};

/// Slot 0 of every frame: the resume point.
const STATE: u32 = 0;
/// The state while the body runs, so a read of the stream from inside it is caught.
const RUNNING: i128 = -1;
/// The state once the body has ended.
const DONE: i128 = -2;

/// The slots of one frame: the state, the parameters and captures, then each register live across a `yield`.
struct Frame {
    slots: Vec<FrameSlot>,
    of: HashMap<RegId, u32>,
}

impl Frame {
    fn new() -> Self {
        Self { slots: vec![FrameSlot { ty: TyId::INT, cell: false }], of: HashMap::new() }
    }

    fn slot(&mut self, body: &Function, reg: RegId) -> u32 {
        if let Some(&slot) = self.of.get(&reg) {
            return slot;
        }
        let slot = self.slots.len() as u32;
        self.slots.push(FrameSlot { ty: body.regs[reg].ty, cell: body.regs[reg].cell });
        self.of.insert(reg, slot);
        slot
    }
}

fn inst(dest: Option<RegId>, op: Op, span: Span) -> Inst {
    Inst { dest, op, span }
}

fn store_state(state: i128) -> Inst {
    inst(None, Op::FrameStore { slot: STATE, value: Operand::Const(Const::Int(state)) }, Span::default())
}

/// Replaces every generator by a function that allocates its frame and a resume function that runs it.
pub(crate) fn lower_generators(module: &mut Module, program: &mut Program, interner: &mut Interner) {
    let generators: Vec<FuncId> = module.functions.iter().filter(|(_, function)| function.is_generator).map(|(id, _)| id).collect();
    for id in generators {
        split(module, program, interner, id);
    }
}

fn split(module: &mut Module, program: &mut Program, interner: &mut Interner, id: FuncId) {
    let body = module.functions[id].clone();
    let element = match program.types.kind(body.ret) {
        TyKind::Adt { args, .. } => args.first().copied().unwrap_or(TyId::ERROR),
        _ => TyId::ERROR,
    };
    let next = program.types.optional(element);
    let held_after = live_after(&body);
    let mut frame = Frame::new();
    let inputs: Vec<RegId> = body.params.iter().chain(&body.captures).copied().collect();
    for &reg in &inputs {
        frame.slot(&body, reg);
    }

    let mut regs = body.regs.clone();
    let state = regs.alloc(Reg { ty: TyId::INT, name: None, cell: false });
    let mut blocks = body.blocks.clone();
    let order: Vec<BlockId> = blocks.iter().map(|(block, _)| block).collect();
    let mut points: Vec<BlockId> = Vec::new();
    let unfinished = Block { insts: Vec::new(), term: Terminator::Unreachable };
    for block_id in order {
        let source = std::mem::replace(&mut blocks[block_id], unfinished.clone());
        let mut current = block_id;
        let mut insts: Vec<Inst> = Vec::new();
        for (at, step) in source.insts.into_iter().enumerate() {
            let Op::Yield(value) = step.op else {
                insts.push(step);
                continue;
            };
            let point = points.len() as i128 + 1;
            let mut loads = Vec::new();
            for &reg in &held_after[block_id.index()][at] {
                let slot = frame.slot(&body, reg);
                insts.push(inst(None, Op::FrameStore { slot, value: Operand::Reg(reg) }, step.span));
                loads.push(inst(Some(reg), Op::FrameLoad(slot), step.span));
            }
            insts.push(store_state(point));
            let out = regs.alloc(Reg { ty: next, name: None, cell: false });
            insts.push(inst(Some(out), Op::Convert { conversion: Conversion::ToOptional, operand: value }, step.span));
            blocks[current] = Block { insts: std::mem::take(&mut insts), term: Terminator::Return(Some(Operand::Reg(out))) };
            current = blocks.alloc(unfinished.clone());
            points.push(current);
            insts = loads;
        }
        let term = match source.term {
            Terminator::Return(_) => {
                insts.push(store_state(DONE));
                Terminator::Return(Some(Operand::Const(Const::Null)))
            }
            other => other,
        };
        blocks[current] = Block { insts, term };
    }

    let start = blocks.alloc(Block {
        insts: inputs.iter().map(|&reg| inst(Some(reg), Op::FrameLoad(frame.of[&reg]), Span::default())).collect(),
        term: Terminator::Jump(body.entry),
    });
    let finished = blocks.alloc(Block { insts: vec![store_state(DONE)], term: Terminator::Return(Some(Operand::Const(Const::Null))) });
    let message = interner.intern("a stream was read while it runs");
    let fault = interner.intern("fault");
    let reentered = blocks.alloc(Block {
        insts: vec![inst(None, Op::Call { callee: Callee::Intrinsic(fault), args: vec![Operand::Const(Const::String(message))], outs: Vec::new() }, Span::default())],
        term: Terminator::Unreachable,
    });
    let mut cases = vec![(0, start), (DONE, finished)];
    cases.extend(points.iter().enumerate().map(|(at, &block)| (at as i128 + 1, block)));
    let dispatch = blocks.alloc(Block {
        insts: vec![inst(Some(state), Op::FrameLoad(STATE), Span::default()), store_state(RUNNING)],
        term: Terminator::Switch { value: Operand::Reg(state), cases, default: reentered },
    });
    let resume = module.add(Function {
        kind: FunctionKind::Resume { generator: id },
        regs,
        captures: Vec::new(),
        params: Vec::new(),
        out_params: Vec::new(),
        ret: next,
        is_generator: false,
        frame: frame.slots,
        blocks,
        entry: dispatch,
    });

    let mut b = FunctionBuilder::new(body.kind.clone(), body.ret);
    let mut init = Vec::new();
    for (position, &param) in body.params.iter().enumerate() {
        let reg = &body.regs[param];
        init.push(Operand::Reg(b.param(reg.ty, reg.name, body.out_params.contains(&position))));
    }
    for &capture in &body.captures {
        let reg = &body.regs[capture];
        init.push(Operand::Reg(b.capture(reg.ty, reg.name, reg.cell)));
    }
    let stream = b.value(body.ret, Op::Stream { resume, init }, Span::default());
    b.terminate(Terminator::Return(Some(stream)));
    module.functions[id] = b.finish().expect("the constructor ends");
}
