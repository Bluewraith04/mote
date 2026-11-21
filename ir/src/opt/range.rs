//! Overflow proofs: an integer `+`, `-` or `*` whose operands have known bounds cannot fault, so it becomes the wrapping operation.

use std::collections::{HashMap, HashSet};

use mote_base::Interner;
use mote_semantics::{BinaryOp, IntTy, Program, TyKind};

use crate::ir::{BlockId, Callee, Const, Function, Op, Operand, RegId, Terminator};
use crate::uses::writes;

/// The least and the greatest value a register holds.
type Bounds = HashMap<RegId, (i128, i128)>;

/// The integer type of a register that is not a cell.
fn int_of(function: &Function, program: &Program, reg: RegId) -> Option<IntTy> {
    match program.types.kind(function.regs[reg].ty) {
        TyKind::Int(int) if !function.regs[reg].cell => Some(*int),
        _ => None,
    }
}

fn range_of(function: &Function, program: &Program, bounds: &Bounds, operand: &Operand) -> Option<(i128, i128)> {
    match operand {
        Operand::Const(Const::Int(value)) => Some((*value, *value)),
        Operand::Reg(reg) => {
            let int = int_of(function, program, *reg)?;
            Some(bounds.get(reg).copied().unwrap_or((int.min(), int.max())))
        }
        Operand::Const(_) => None,
    }
}

/// The bounds of `left op right` when it does not fault.
fn combine(op: BinaryOp, (a_low, a_high): (i128, i128), (b_low, b_high): (i128, i128)) -> Option<(i128, i128)> {
    match op {
        BinaryOp::Add => Some((a_low.checked_add(b_low)?, a_high.checked_add(b_high)?)),
        BinaryOp::Sub => Some((a_low.checked_sub(b_high)?, a_high.checked_sub(b_low)?)),
        BinaryOp::Mul => {
            let products = [
                a_low.checked_mul(b_low)?,
                a_low.checked_mul(b_high)?,
                a_high.checked_mul(b_low)?,
                a_high.checked_mul(b_high)?,
            ];
            Some((*products.iter().min()?, *products.iter().max()?))
        }
        _ => None,
    }
}

/// The intrinsic that does the operation without a check.
fn wrapping(op: BinaryOp) -> Option<&'static str> {
    match op {
        BinaryOp::Add => Some("num_wrapping_add"),
        BinaryOp::Sub => Some("num_wrapping_sub"),
        BinaryOp::Mul => Some("num_wrapping_mul"),
        _ => None,
    }
}

/// What the instruction leaves in the bounds.
fn transfer(function: &Function, program: &Program, inst: &crate::ir::Inst, bounds: &mut Bounds) {
    let found = inst.dest.and_then(|dest| {
        let int = int_of(function, program, dest)?;
        let (low, high) = match &inst.op {
            Op::Copy(source) => range_of(function, program, bounds, source)?,
            Op::Binary { op, left, right } => combine(*op, range_of(function, program, bounds, left)?, range_of(function, program, bounds, right)?)?,
            _ => return None,
        };
        let (low, high) = (low.max(int.min()), high.min(int.max()));
        (low <= high).then_some((dest, (low, high)))
    });
    for reg in writes(inst) {
        bounds.remove(&reg);
    }
    if let Some((dest, range)) = found {
        bounds.insert(dest, range);
    }
}

/// `a < b` when `strict`, else `a <= b`.
struct Order<'a> {
    a: &'a Operand,
    b: &'a Operand,
    strict: bool,
}

/// Narrows the bounds by what the edge of the branch says, when the condition is a comparison of integers still unchanged at the end of the block.
fn refine(function: &Function, program: &Program, block: BlockId, taken: bool, bounds: &mut Bounds) {
    let data = &function.blocks[block];
    let Terminator::Branch { condition: Operand::Reg(condition), .. } = &data.term else { return };
    let Some(at) = data.insts.iter().rposition(|inst| writes(inst).any(|reg| reg == *condition)) else { return };
    let Op::Binary { op, left, right } = &data.insts[at].op else { return };
    let order = match (op, taken) {
        (BinaryOp::Lt, true) | (BinaryOp::GtEq, false) => Order { a: left, b: right, strict: true },
        (BinaryOp::LtEq, true) | (BinaryOp::Gt, false) => Order { a: left, b: right, strict: false },
        (BinaryOp::Gt, true) | (BinaryOp::LtEq, false) => Order { a: right, b: left, strict: true },
        (BinaryOp::GtEq, true) | (BinaryOp::Lt, false) => Order { a: right, b: left, strict: false },
        _ => return,
    };
    let unchanged = |operand: &Operand| match operand {
        Operand::Reg(reg) => data.insts[at + 1..].iter().all(|inst| writes(inst).all(|written| written != *reg)),
        Operand::Const(_) => true,
    };
    if !unchanged(order.a) || !unchanged(order.b) {
        return;
    }
    let (Some((a_low, a_high)), Some((b_low, b_high))) = (range_of(function, program, bounds, order.a), range_of(function, program, bounds, order.b)) else { return };
    let gap = i128::from(order.strict);
    if let Operand::Reg(reg) = order.a {
        let high = a_high.min(b_high - gap);
        if a_low <= high {
            bounds.insert(*reg, (a_low, high));
        }
    }
    if let Operand::Reg(reg) = order.b {
        let low = b_low.max(a_low + gap);
        if low <= b_high {
            bounds.insert(*reg, (low, b_high));
        }
    }
}

/// The blocks reachable from the entry, each after every block that can come before it without a loop.
fn reverse_postorder(function: &Function) -> Vec<BlockId> {
    let mut seen: HashSet<BlockId> = HashSet::from([function.entry]);
    let mut stack: Vec<(BlockId, Vec<BlockId>)> = vec![(function.entry, function.blocks[function.entry].term.targets())];
    let mut done = Vec::new();
    while let Some((block, pending)) = stack.last_mut() {
        match pending.pop() {
            Some(next) => {
                if seen.insert(next) {
                    stack.push((next, function.blocks[next].term.targets()));
                }
            }
            None => {
                done.push(*block);
                stack.pop();
            }
        }
    }
    done.reverse();
    done
}

/// Rewrites each operation that cannot overflow into its wrapping intrinsic; true when something changed.
pub(super) fn prove(function: &mut Function, program: &Program, interner: &mut Interner) -> bool {
    let order = reverse_postorder(function);
    // The ways into each block: the block it comes from, and which side of its branch when that side says something.
    let mut edges: HashMap<BlockId, Vec<(BlockId, Option<bool>)>> = HashMap::new();
    for &id in &order {
        let term = &function.blocks[id].term;
        let targets = term.targets();
        let sides = matches!(term, Terminator::Branch { then, otherwise, .. } if then != otherwise);
        for (position, target) in targets.into_iter().enumerate() {
            edges.entry(target).or_default().push((id, sides.then_some(position == 0)));
        }
    }
    let mut found: Vec<(BlockId, usize, BinaryOp)> = Vec::new();
    let mut out: HashMap<BlockId, Bounds> = HashMap::new();
    for &id in &order {
        // What every way in agrees on; a way from a block not yet seen (a loop's back edge) leaves nothing known.
        let mut bounds: Option<Bounds> = None;
        let mut known = id != function.entry;
        for &(from, side) in edges.get(&id).map(Vec::as_slice).unwrap_or_default() {
            let Some(held) = out.get(&from) else {
                known = false;
                break;
            };
            let mut arriving = held.clone();
            if let Some(taken) = side {
                refine(function, program, from, taken, &mut arriving);
            }
            bounds = Some(match bounds {
                None => arriving,
                Some(mut so_far) => {
                    so_far.retain(|reg, range| {
                        arriving.get(reg).is_some_and(|other| {
                            *range = (range.0.min(other.0), range.1.max(other.1));
                            true
                        })
                    });
                    so_far
                }
            });
        }
        let mut bounds = if known { bounds.unwrap_or_default() } else { Bounds::new() };
        for (at, inst) in function.blocks[id].insts.iter().enumerate() {
            if let (Some(dest), Op::Binary { op, left, right }) = (inst.dest, &inst.op) {
                let both_constant = matches!((left, right), (Operand::Const(_), Operand::Const(_)));
                if let (false, Some(int), Some(_), Some(left), Some(right)) = (both_constant, int_of(function, program, dest), wrapping(*op), range_of(function, program, &bounds, left), range_of(function, program, &bounds, right))
                    && let Some((low, high)) = combine(*op, left, right)
                    && int.fits(low)
                    && int.fits(high)
                {
                    found.push((id, at, *op));
                }
            }
            transfer(function, program, inst, &mut bounds);
        }
        out.insert(id, bounds);
    }
    let changed = !found.is_empty();
    for (id, at, op) in found {
        let name = wrapping(op).expect("an operation with a wrapping form");
        let symbol = interner.intern(name);
        let inst = &mut function.blocks[id].insts[at];
        if let Op::Binary { left, right, .. } = &inst.op {
            inst.op = Op::Call { callee: Callee::Intrinsic(symbol), args: vec![*left, *right], outs: Vec::new() };
        }
    }
    changed
}
