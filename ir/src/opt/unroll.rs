//! Unrolls a counting loop whose bounds are constants, so that what depends on the counter can fold.
//!
//! The loop is peeled: the copies follow one another and the last one jumps back to the original loop, so the result is right for any count and the count only says how many copies are worth making.

use std::collections::{HashMap, HashSet};

use mote_semantics::BinaryOp;

use super::cfg::targets_mut;
use crate::ir::{Block, BlockId, Const, Function, Op, Operand, RegId, Terminator};
use crate::uses::writes;

/// The most iterations a loop has and is still unrolled.
const MAX_TRIPS: i128 = 32;

/// The most instructions the copies of a loop add up to.
const MAX_INSTS: usize = 4096;

/// What `find` says about a loop worth peeling.
struct Plan {
    header: BlockId,
    before: BlockId,
    blocks: Vec<BlockId>,
    copies: usize,
}

/// Peels one loop of the shape `i = s; while i < K { …; i = i + 1 }`; true when it did.
pub(super) fn run(function: &mut Function) -> bool {
    let ids: Vec<BlockId> = function.blocks.iter().map(|(id, _)| id).collect();
    let mut succs: HashMap<BlockId, Vec<BlockId>> = HashMap::new();
    let mut preds: HashMap<BlockId, Vec<BlockId>> = HashMap::new();
    for &id in &ids {
        for target in function.blocks[id].term.targets() {
            succs.entry(id).or_default().push(target);
            preds.entry(target).or_default().push(id);
        }
    }
    for &header in &ids {
        if let Some(plan) = find(function, &succs, &preds, header) {
            peel(function, &plan);
            return true;
        }
    }
    false
}

/// The blocks a walk along `edges` from `start` reaches, `start` included.
fn reached(edges: &HashMap<BlockId, Vec<BlockId>>, start: BlockId) -> HashSet<BlockId> {
    let mut seen = HashSet::from([start]);
    let mut work = vec![start];
    while let Some(block) = work.pop() {
        for &next in edges.get(&block).into_iter().flatten() {
            if seen.insert(next) {
                work.push(next);
            }
        }
    }
    seen
}

fn find(function: &Function, succs: &HashMap<BlockId, Vec<BlockId>>, preds: &HashMap<BlockId, Vec<BlockId>>, header: BlockId) -> Option<Plan> {
    let head = &function.blocks[header];
    let [test] = &head.insts[..] else { return None };
    let (Some(condition), Op::Binary { op: BinaryOp::Lt, left: Operand::Reg(counter), right: Operand::Const(Const::Int(limit)) }) = (test.dest, &test.op) else { return None };
    let Terminator::Branch { condition: Operand::Reg(tested), then: body, otherwise: exit } = head.term else { return None };
    if tested != condition || condition == *counter || function.regs[*counter].cell || body == header {
        return None;
    }
    let looping: HashSet<BlockId> = reached(succs, header).intersection(&reached(preds, header)).copied().collect();
    if !looping.contains(&body) || looping.contains(&exit) {
        return None;
    }
    let entering: Vec<BlockId> = preds.get(&header)?.iter().copied().filter(|block| !looping.contains(block)).collect();
    let [before] = entering[..] else { return None };
    if function.blocks[before].term != Terminator::Jump(header) || looping.iter().any(|&block| block != header && preds.get(&block).into_iter().flatten().any(|from| !looping.contains(from))) {
        return None;
    }
    let mut blocks: Vec<BlockId> = looping.iter().copied().collect();
    blocks.sort_by_key(|block| block.index());
    let insts = || blocks.iter().flat_map(|&block| function.blocks[block].insts.iter());
    if insts().any(|inst| matches!(inst.op, Op::Defer { .. } | Op::Undefer | Op::ScopeBegin | Op::ScopeEnd(_) | Op::Yield(_) | Op::FrameLoad(_) | Op::FrameStore { .. })) {
        return None;
    }
    let steps = insts().filter(|inst| writes(inst).any(|reg| reg == *counter)).collect::<Vec<_>>();
    let [step] = steps[..] else { return None };
    if !matches!((step.dest, &step.op), (Some(dest), Op::Binary { op: BinaryOp::Add, left: Operand::Reg(left), right: Operand::Const(Const::Int(1)) }) if dest == *counter && left == counter) {
        return None;
    }
    let trips = (*limit - starts_at(function, before, *counter)?).max(0);
    let copies = trips as usize + 1;
    (trips <= MAX_TRIPS && copies * insts().count() <= MAX_INSTS).then_some(Plan { header, before, blocks, copies })
}

/// The constant the block last sets the counter to.
fn starts_at(function: &Function, block: BlockId, counter: RegId) -> Option<i128> {
    let last = function.blocks[block].insts.iter().rev().find(|inst| writes(inst).any(|reg| reg == counter))?;
    match (last.dest, &last.op) {
        (Some(dest), Op::Copy(Operand::Const(Const::Int(start)))) if dest == counter => Some(*start),
        _ => None,
    }
}

/// Puts `plan.copies` copies of the loop before it, each jumping to the next and the last to the loop itself.
fn peel(function: &mut Function, plan: &Plan) {
    let placed: Vec<HashMap<BlockId, BlockId>> = (0..plan.copies).map(|_| plan.blocks.iter().map(|&block| (block, function.blocks.alloc(Block { insts: Vec::new(), term: Terminator::Unreachable }))).collect()).collect();
    for (round, names) in placed.iter().enumerate() {
        for &block in &plan.blocks {
            let mut copy = function.blocks[block].clone();
            for target in targets_mut(&mut copy.term) {
                *target = if *target == plan.header {
                    placed.get(round + 1).map_or(plan.header, |next| next[&plan.header])
                } else {
                    names.get(target).copied().unwrap_or(*target)
                };
            }
            function.blocks[names[&block]] = copy;
        }
    }
    function.blocks[plan.before].term = Terminator::Jump(placed[0][&plan.header]);
}
