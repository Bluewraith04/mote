//! The control-flow graph: constant branches, empty blocks, straight-line merges, switches and unreachable blocks.

use mote_base::Arena;
use mote_semantics::BinaryOp;

use crate::ir::{Block, BlockId, Const, Function, Op, Operand, RegId, Terminator};
use crate::uses::{reads, terminator_reads, writes};

pub(super) fn targets_mut(term: &mut Terminator) -> Vec<&mut BlockId> {
    match term {
        Terminator::Jump(target) => vec![target],
        Terminator::Branch { then, otherwise, .. } => vec![then, otherwise],
        Terminator::Switch { cases, default, .. } => cases.iter_mut().map(|(_, block)| block).chain([default]).collect(),
        Terminator::Return(_) | Terminator::Unreachable => Vec::new(),
    }
}

/// Branches and switches on a constant, and branches to one block, become jumps.
fn fold_terminators(function: &mut Function, ids: &[BlockId]) -> bool {
    let mut changed = false;
    for &id in ids {
        let next = match &function.blocks[id].term {
            Terminator::Branch { condition: Operand::Const(Const::Bool(taken)), then, otherwise } => Some(if *taken { *then } else { *otherwise }),
            Terminator::Branch { then, otherwise, .. } if then == otherwise => Some(*then),
            Terminator::Switch { value: Operand::Const(Const::Int(n)), cases, default } => Some(cases.iter().find(|(case, _)| case == n).map_or(*default, |&(_, block)| block)),
            _ => None,
        };
        if let Some(target) = next {
            function.blocks[id].term = Terminator::Jump(target);
            changed = true;
        }
    }
    changed
}

/// Sends every edge past the empty blocks that only jump on.
fn thread_jumps(function: &mut Function, ids: &[BlockId]) -> bool {
    let through = |function: &Function, mut block: BlockId| {
        for _ in 0..16 {
            let data = &function.blocks[block];
            match data.term {
                Terminator::Jump(next) if data.insts.is_empty() && next != block => block = next,
                _ => break,
            }
        }
        block
    };
    let mut changed = false;
    for &id in ids {
        let mut term = function.blocks[id].term.clone();
        for target in targets_mut(&mut term) {
            let end = through(function, *target);
            if end != *target {
                *target = end;
                changed = true;
            }
        }
        function.blocks[id].term = term;
    }
    let entry = through(function, function.entry);
    if entry != function.entry {
        function.entry = entry;
        changed = true;
    }
    changed
}

fn reachable(function: &Function) -> Vec<bool> {
    let mut seen = vec![false; function.blocks.len()];
    let mut work = vec![function.entry];
    while let Some(block) = work.pop() {
        if std::mem::replace(&mut seen[block.index()], true) {
            continue;
        }
        work.extend(function.blocks[block].term.targets());
    }
    seen
}

fn predecessors(function: &Function, seen: &[bool]) -> Vec<usize> {
    let mut count = vec![0; function.blocks.len()];
    for (id, block) in function.blocks.iter() {
        if seen[id.index()] {
            for target in block.term.targets() {
                count[target.index()] += 1;
            }
        }
    }
    count
}

/// Joins a block to the block it jumps to when no one else enters that one.
fn merge(function: &mut Function, ids: &[BlockId]) -> bool {
    let seen = reachable(function);
    let preds = predecessors(function, &seen);
    let mut changed = false;
    for &id in ids {
        if !seen[id.index()] {
            continue;
        }
        while let Terminator::Jump(next) = function.blocks[id].term {
            if next == id || next == function.entry || preds[next.index()] != 1 {
                break;
            }
            let tail = std::mem::replace(&mut function.blocks[next], Block { insts: Vec::new(), term: Terminator::Unreachable });
            function.blocks[id].insts.extend(tail.insts);
            function.blocks[id].term = tail.term;
            changed = true;
        }
    }
    changed
}

/// The `x == k` that ends a block: the register tested, the constant, and the register holding the result.
fn test_of(block: &Block) -> Option<(RegId, i128, RegId)> {
    let last = block.insts.last()?;
    let Op::Binary { op: BinaryOp::Eq, left, right } = &last.op else { return None };
    let ((Operand::Reg(value), Operand::Const(Const::Int(case))) | (Operand::Const(Const::Int(case)), Operand::Reg(value))) = (left, right) else { return None };
    Some((*value, *case, last.dest?))
}

/// The most cases a switch gets when ranges are written out value by value.
const SWITCH_CASES: usize = 512;

/// The bound a comparison of a register with a constant puts on the register: its least and greatest value when the test holds.
fn bound_of(block: &Block) -> Option<(RegId, (i128, i128), RegId)> {
    let last = block.insts.last()?;
    let Op::Binary { op, left, right } = &last.op else { return None };
    let (value, constant, op) = match (left, right) {
        (Operand::Reg(value), Operand::Const(Const::Int(constant))) => (*value, *constant, *op),
        (Operand::Const(Const::Int(constant)), Operand::Reg(value)) => {
            let flipped = match op {
                BinaryOp::Lt => BinaryOp::Gt,
                BinaryOp::LtEq => BinaryOp::GtEq,
                BinaryOp::Gt => BinaryOp::Lt,
                BinaryOp::GtEq => BinaryOp::LtEq,
                other => *other,
            };
            (*value, *constant, flipped)
        }
        _ => return None,
    };
    let range = match op {
        BinaryOp::GtEq => (constant, i128::MAX),
        BinaryOp::Gt => (constant.checked_add(1)?, i128::MAX),
        BinaryOp::LtEq => (i128::MIN, constant),
        BinaryOp::Lt => (i128::MIN, constant.checked_sub(1)?),
        _ => return None,
    };
    Some((value, range, last.dest?))
}

/// One step of a chain of tests: the register tested, the values for which the test holds, the block when it holds, the block when it does not, and the blocks the step used.
struct Link {
    value: RegId,
    cases: Vec<i128>,
    then: BlockId,
    otherwise: BlockId,
    ranged: bool,
}

/// The test that ends `id`: an equality with a constant, or a lower and an upper bound on the register in two blocks that both go to the same block when they fail. After the first link of a chain (`ways_in` is the edges the previous link sends here) a test block holds nothing else.
fn link(function: &Function, preds: &[usize], single: &dyn Fn(RegId) -> bool, id: BlockId, ways_in: Option<usize>) -> Option<Link> {
    let block = &function.blocks[id];
    if ways_in.is_some_and(|ways| preds[id.index()] != ways || block.insts.len() != 1) {
        return None;
    }
    let Terminator::Branch { condition: Operand::Reg(tested), then, otherwise } = block.term else { return None };
    if let Some((value, case, result)) = test_of(block) {
        return (result == tested && single(tested)).then_some(Link { value, cases: vec![case], then, otherwise, ranged: false });
    }
    let (value, (low, high), result) = bound_of(block)?;
    if result != tested || !single(tested) {
        return None;
    }
    let second = &function.blocks[then];
    if preds[then.index()] != 1 || second.insts.len() != 1 {
        return None;
    }
    let Terminator::Branch { condition: Operand::Reg(again), then: inside, otherwise: outside } = second.term else { return None };
    let (same, (more_low, more_high), result) = bound_of(second)?;
    if same != value || result != again || !single(again) || outside != otherwise || (low == i128::MIN) == (more_low == i128::MIN) {
        return None;
    }
    let (low, high) = (low.max(more_low), high.min(more_high));
    if low > high || low == i128::MIN || high == i128::MAX || high - low >= SWITCH_CASES as i128 {
        return None;
    }
    Some(Link { value, cases: (low..=high).collect(), then: inside, otherwise, ranged: true })
}

/// A chain of tests of one register against constants and ranges becomes a switch.
fn form_switches(function: &mut Function, ids: &[BlockId]) -> bool {
    let seen = reachable(function);
    let preds = predecessors(function, &seen);
    let mut reads_of = vec![0usize; function.regs.len()];
    for (_, block) in function.blocks.iter() {
        for inst in &block.insts {
            for reg in reads(&inst.op) {
                reads_of[reg.index()] += 1;
            }
        }
        for reg in terminator_reads(&block.term) {
            reads_of[reg.index()] += 1;
        }
    }
    let mut written = vec![0usize; function.regs.len()];
    for (_, block) in function.blocks.iter() {
        for inst in &block.insts {
            for reg in writes(inst) {
                written[reg.index()] += 1;
            }
        }
    }
    let single = |reg: RegId| reads_of[reg.index()] == 1 && written[reg.index()] == 1;
    let mut changed = false;
    for &id in ids {
        if !seen[id.index()] {
            continue;
        }
        let Some(start) = link(function, &preds, &single, id, None) else { continue };
        let value = start.value;
        let mut cases: Vec<(i128, BlockId)> = start.cases.iter().map(|&case| (case, start.then)).collect();
        let mut links = 1;
        let mut ranged = start.ranged;
        let mut next = start.otherwise;
        let mut ways = if start.ranged { 2 } else { 1 };
        while let Some(step) = link(function, &preds, &single, next, Some(ways)) {
            if step.value != value || cases.len() + step.cases.len() > SWITCH_CASES {
                break;
            }
            for &case in &step.cases {
                if !cases.iter().any(|&(held, _)| held == case) {
                    cases.push((case, step.then));
                }
            }
            links += 1;
            ranged |= step.ranged;
            next = step.otherwise;
            ways = if step.ranged { 2 } else { 1 };
        }
        if links < 3 && !(links == 2 && ranged) {
            continue;
        }
        let block = &mut function.blocks[id];
        block.insts.pop();
        block.term = Terminator::Switch { value: Operand::Reg(value), cases, default: next };
        changed = true;
    }
    changed
}

/// Drops the blocks nothing reaches and renumbers the rest in order.
fn compact(function: &mut Function) -> bool {
    let seen = reachable(function);
    if seen.iter().all(|&kept| kept) {
        return false;
    }
    let old: Vec<Block> = function.blocks.as_slice().to_vec();
    let mut blocks = Arena::new();
    let mut map: Vec<Option<BlockId>> = vec![None; old.len()];
    for (index, block) in old.into_iter().enumerate() {
        if seen[index] {
            map[index] = Some(blocks.alloc(block));
        }
    }
    let ids: Vec<BlockId> = blocks.iter().map(|(id, _)| id).collect();
    for id in ids {
        for target in targets_mut(&mut blocks[id].term) {
            *target = map[target.index()].expect("a reachable block has reachable targets");
        }
    }
    function.entry = map[function.entry.index()].expect("the entry is reachable");
    function.blocks = blocks;
    true
}

/// Simplifies the graph once; true when something changed.
pub(super) fn simplify(function: &mut Function) -> bool {
    let ids: Vec<BlockId> = function.blocks.iter().map(|(id, _)| id).collect();
    let mut changed = fold_terminators(function, &ids);
    changed |= thread_jumps(function, &ids);
    changed |= form_switches(function, &ids);
    changed |= merge(function, &ids);
    changed |= compact(function);
    changed
}
