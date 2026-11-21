//! Inlining: a call of a small function, or the only call of a function, is replaced by the callee's blocks.

use std::collections::{HashMap, HashSet};

use mote_base::Interner;

use super::cfg::targets_mut;
use super::copies;
use crate::ir::{Block, BlockId, Callee, Const, FuncId, Function, FunctionKind, Inst, Module, Op, Operand, Reg, RegId, Terminator};
use crate::uses::{operands_mut, terminator_operands_mut, writes};

/// The most a function weighs and is still inlined at every call.
const SMALL: usize = 8;

/// The most a function that reads the reflection table weighs and is still inlined at every call, so that the reads can fold at a known type.
const READS_TABLE: usize = 400;

/// Which functions read the reflection table, directly or through a call.
fn reading_table(module: &Module, interner: &Interner) -> Vec<bool> {
    let mut reads = vec![false; module.functions.len()];
    for (id, function) in module.functions.iter() {
        reads[id.index()] = function.blocks.iter().any(|(_, block)| {
            block.insts.iter().any(|inst| matches!(&inst.op, Op::Call { callee: Callee::Intrinsic(name), .. } if matches!(interner.resolve(*name), "meta_ints" | "meta_texts")))
        });
    }
    loop {
        let mut changed = false;
        for (id, function) in module.functions.iter() {
            if !reads[id.index()] && function.blocks.iter().any(|(_, block)| block.insts.iter().any(|inst| matches!(&inst.op, Op::Call { callee: Callee::Function(callee), .. } if reads[callee.index()]))) {
                reads[id.index()] = true;
                changed = true;
            }
        }
        if !changed {
            return reads;
        }
    }
}

/// What a call of a function weighs against `SMALL`; a body made of calls gains little from being copied. An intrinsic weighs one.
const CALL: usize = 4;

/// The most rounds of inlining.
const ROUNDS: usize = 8;

/// The parts of a callee that a call site copies.
struct Template {
    regs: Vec<Reg>,
    blocks: Vec<Block>,
    params: Vec<RegId>,
    captures: Vec<RegId>,
    out_params: Vec<usize>,
    entry: BlockId,
}

fn template(function: &Function) -> Template {
    Template { regs: function.regs.as_slice().to_vec(), blocks: function.blocks.as_slice().to_vec(), params: function.params.clone(), captures: function.captures.clone(), out_params: function.out_params.clone(), entry: function.entry }
}

/// The instructions of a function, a call counting as `CALL`.
fn size(function: &Function) -> usize {
    function.blocks.iter().map(|(_, block)| block.insts.iter().map(|inst| if matches!(inst.op, Op::Call { callee: Callee::Function(_) | Callee::Value(_), .. }) { CALL } else { 1 }).sum::<usize>()).sum()
}

/// Whether a call can be replaced by the body: a plain function that keeps no state in its frame.
fn inlinable(id: FuncId, function: &Function) -> bool {
    if !matches!(function.kind, FunctionKind::Instance(_)) || !function.captures.is_empty() || function.params.iter().any(|&param| function.regs[param].cell) {
        return false;
    }
    function.blocks.iter().all(|(_, block)| {
        block.insts.iter().all(|inst| match &inst.op {
            Op::Defer { .. } | Op::Undefer | Op::ScopeBegin | Op::ScopeEnd(_) => false,
            Op::Call { callee: Callee::Function(callee), .. } => *callee != id,
            _ => true,
        })
    })
}

/// How often each function is called by name, and how often it is named in any other way; retired functions do not count.
fn references(module: &Module, retired: &HashSet<FuncId>) -> (Vec<usize>, Vec<usize>) {
    let mut calls = vec![0; module.functions.len()];
    let mut others = vec![0; module.functions.len()];
    for entry in &module.entries {
        others[entry.func.index()] += 1;
    }
    if let Some(start) = module.start {
        others[start.index()] += 1;
    }
    for (id, function) in module.functions.iter() {
        if retired.contains(&id) {
            continue;
        }
        for (_, block) in function.blocks.iter() {
            for inst in &block.insts {
                match &inst.op {
                    Op::Call { callee: Callee::Function(callee), .. } => calls[callee.index()] += 1,
                    Op::Closure { func, .. } | Op::Spawn { func, .. } | Op::Defer { func, .. } | Op::Stream { resume: func, .. } => others[func.index()] += 1,
                    _ => {}
                }
            }
        }
    }
    (calls, others)
}

/// Where the first call to inline in the block is, and whether the callee has no other caller.
fn find(module: &Module, caller: FuncId, block: BlockId, calls: &[usize], others: &[usize], table: &[bool]) -> Option<(usize, FuncId, bool)> {
    module.functions[caller].blocks[block].insts.iter().enumerate().find_map(|(at, inst)| {
        let Op::Call { callee: Callee::Function(callee), args, .. } = &inst.op else { return None };
        let target = &module.functions[*callee];
        let single = calls[callee.index()] == 1 && others[callee.index()] == 0;
        let fits = *callee != caller && args.len() == target.params.len() && inlinable(*callee, target) && (single || size(target) <= SMALL || (table[callee.index()] && size(target) <= READS_TABLE));
        fits.then_some((at, *callee, single))
    })
}

fn rename(operand: &mut Operand, regs: &[RegId]) {
    if let Operand::Reg(reg) = operand {
        *reg = regs[reg.index()];
    }
}

fn renamed(inst: &Inst, regs: &[RegId]) -> Inst {
    let mut inst = inst.clone();
    inst.dest = inst.dest.map(|dest| regs[dest.index()]);
    if let Op::Call { outs, .. } = &mut inst.op {
        for out in outs {
            *out = regs[out.index()];
        }
    }
    for operand in operands_mut(&mut inst.op) {
        rename(operand, regs);
    }
    inst
}

/// Replaces instruction `at` of the block, a call, by a copy of the callee, whose captures hold `captured`; returns the block that holds what followed the call.
fn splice(caller: &mut Function, block: BlockId, at: usize, callee: &Template, captured: &[Operand]) -> BlockId {
    let tail = caller.blocks[block].insts.split_off(at + 1);
    let call = caller.blocks[block].insts.pop().expect("the call is the last instruction left");
    let Op::Call { args, outs, .. } = call.op else { unreachable!("the site is a call") };
    let (dest, span) = (call.dest, call.span);
    let term = std::mem::replace(&mut caller.blocks[block].term, Terminator::Unreachable);
    let after = caller.blocks.alloc(Block { insts: tail, term });
    let regs: Vec<RegId> = callee.regs.iter().map(|reg| caller.regs.alloc(reg.clone())).collect();
    let blocks: Vec<BlockId> = callee.blocks.iter().map(|_| caller.blocks.alloc(Block { insts: Vec::new(), term: Terminator::Unreachable })).collect();
    for (&param, arg) in callee.params.iter().zip(args) {
        caller.blocks[block].insts.push(Inst { dest: Some(regs[param.index()]), op: Op::Copy(arg), span });
    }
    for (&capture, &value) in callee.captures.iter().zip(captured) {
        caller.blocks[block].insts.push(Inst { dest: Some(regs[capture.index()]), op: Op::Copy(value), span });
    }
    caller.blocks[block].term = Terminator::Jump(blocks[callee.entry.index()]);
    for (index, source) in callee.blocks.iter().enumerate() {
        let mut insts: Vec<Inst> = source.insts.iter().map(|inst| renamed(inst, &regs)).collect();
        let mut term = source.term.clone();
        for operand in terminator_operands_mut(&mut term) {
            rename(operand, &regs);
        }
        for target in targets_mut(&mut term) {
            *target = blocks[target.index()];
        }
        if let Terminator::Return(value) = term {
            if let Some(dest) = dest {
                insts.push(Inst { dest: Some(dest), op: Op::Copy(value.unwrap_or(Operand::Const(Const::Null))), span });
            }
            for (&out, &position) in outs.iter().zip(&callee.out_params) {
                insts.push(Inst { dest: Some(out), op: Op::Copy(Operand::Reg(regs[callee.params[position].index()])), span });
            }
            term = Terminator::Jump(after);
        }
        caller.blocks[blocks[index]] = Block { insts, term };
    }
    after
}

/// Inlines calls across the module; true when a call went.
pub(super) fn run(module: &mut Module, interner: &Interner) -> bool {
    let mut retired = HashSet::new();
    let mut changed = false;
    for _ in 0..ROUNDS {
        let (calls, others) = references(module, &retired);
        let table = reading_table(module, interner);
        let callers: Vec<FuncId> = module.functions.iter().map(|(id, _)| id).collect();
        let mut round = false;
        for caller in callers {
            if retired.contains(&caller) {
                continue;
            }
            let mut work: Vec<BlockId> = module.functions[caller].blocks.iter().map(|(id, _)| id).collect();
            while let Some(block) = work.pop() {
                let Some((at, callee, single)) = find(module, caller, block, &calls, &others, &table) else { continue };
                let copy = template(&module.functions[callee]);
                work.push(splice(&mut module.functions[caller], block, at, &copy, &[]));
                if single {
                    retired.insert(callee);
                }
                round = true;
            }
        }
        changed |= round;
        if !round {
            break;
        }
    }
    changed
}

/// The most a closure body weighs and is still inlined at a call through the value.
const CLOSURE: usize = 24;

/// The most closure calls inlined into one function.
const CLOSURE_CALLS: usize = 32;

/// Whether a call through a closure value can be replaced by the body: a small function with no state of its frame and no cells in its parameters or captures.
fn closure_inlinable(id: FuncId, function: &Function) -> bool {
    matches!(function.kind, FunctionKind::Closure { .. } | FunctionKind::Instance(_))
        && !function.is_generator
        && size(function) <= CLOSURE
        && function.params.iter().chain(&function.captures).all(|&reg| !function.regs[reg].cell)
        && function.blocks.iter().all(|(_, block)| {
            block.insts.iter().all(|inst| match &inst.op {
                Op::Defer { .. } | Op::Undefer | Op::ScopeBegin | Op::ScopeEnd(_) | Op::Yield(_) | Op::FrameLoad(_) | Op::FrameStore { .. } => false,
                Op::Call { callee: Callee::Function(callee), .. } => *callee != id,
                _ => true,
            })
        })
}

/// A call through a register that holds a closure made by one instruction: the call, the closure's function, and where it was made.
struct ClosureCall {
    block: BlockId,
    at: usize,
    func: FuncId,
    value: RegId,
    made: (BlockId, usize),
}

fn closure_call(module: &Module, caller: FuncId) -> Option<ClosureCall> {
    let function = &module.functions[caller];
    let mut count = vec![0usize; function.regs.len()];
    let mut made: HashMap<RegId, (BlockId, usize)> = HashMap::new();
    for (block, data) in function.blocks.iter() {
        for (at, inst) in data.insts.iter().enumerate() {
            for reg in writes(inst) {
                count[reg.index()] += 1;
            }
            if let (Some(dest), Op::Closure { .. }) = (inst.dest, &inst.op) {
                made.insert(dest, (block, at));
            }
        }
    }
    for (block, data) in function.blocks.iter() {
        for (at, inst) in data.insts.iter().enumerate() {
            let Op::Call { callee: Callee::Value(Operand::Reg(value)), args, .. } = &inst.op else { continue };
            let Some(&(made_block, made_at)) = made.get(value) else { continue };
            if count[value.index()] != 1 || function.params.contains(value) || function.captures.contains(value) {
                continue;
            }
            let Op::Closure { func, captures } = &function.blocks[made_block].insts[made_at].op else { continue };
            let target = &module.functions[*func];
            if *func != caller && args.len() == target.params.len() && captures.len() == target.captures.len() && closure_inlinable(*func, target) {
                return Some(ClosureCall { block, at, func: *func, value: *value, made: (made_block, made_at) });
            }
        }
    }
    None
}

/// Replaces a call through a closure value of known function by the body, with the captured values copied right where the closure was made; true when a call went.
pub(super) fn closures(module: &mut Module) -> bool {
    let mut changed = false;
    let callers: Vec<FuncId> = module.functions.iter().map(|(id, _)| id).collect();
    for caller in callers {
        if !module.functions[caller].blocks.iter().any(|(_, block)| block.insts.iter().any(|inst| matches!(inst.op, Op::Call { callee: Callee::Value(_), .. }))) {
            continue;
        }
        let mut held: HashMap<RegId, Vec<RegId>> = HashMap::new();
        for _ in 0..CLOSURE_CALLS {
            copies::propagate(&mut module.functions[caller]);
            let Some(site) = closure_call(module, caller) else { break };
            let body = template(&module.functions[site.func]);
            let captured: Vec<Operand> = match held.get(&site.value) {
                Some(fresh) => fresh.iter().map(|&reg| Operand::Reg(reg)).collect(),
                None => {
                    let function = &mut module.functions[caller];
                    let (made_block, made_at) = site.made;
                    let span = function.blocks[made_block].insts[made_at].span;
                    let Op::Closure { captures, .. } = function.blocks[made_block].insts[made_at].op.clone() else { unreachable!("the closure was found there") };
                    let mut copies = Vec::new();
                    let mut fresh = Vec::new();
                    for (&capture, value) in body.captures.iter().zip(captures) {
                        let reg = function.regs.alloc(Reg { ty: body.regs[capture.index()].ty, name: None, cell: false });
                        copies.push(Inst { dest: Some(reg), op: Op::Copy(value), span });
                        fresh.push(reg);
                    }
                    function.blocks[made_block].insts.splice(made_at + 1..made_at + 1, copies);
                    held.insert(site.value, fresh);
                    continue;
                }
            };
            splice(&mut module.functions[caller], site.block, site.at, &body, &captured);
            changed = true;
        }
    }
    changed
}
