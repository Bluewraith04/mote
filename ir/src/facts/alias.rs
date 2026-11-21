//! Which objects a register may hold, and which of them leave the function or are written.

use std::collections::{HashMap, HashSet};

use mote_base::Interner;
use mote_semantics::{Program, TyId, TyKind};

use super::{Allocation, Summary};
use crate::intrinsics;
use crate::ir::{Block, Callee, FuncId, Function, Inst, Module, Op, Operand, RegId, Terminator};
use crate::uses::{reads, writes};

/// A set of site numbers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Bits(Vec<u64>);

impl Bits {
    fn of(index: usize) -> Self {
        let mut bits = Self::default();
        bits.insert(index);
        bits
    }

    fn insert(&mut self, index: usize) -> bool {
        let word = index / 64;
        if word >= self.0.len() {
            self.0.resize(word + 1, 0);
        }
        let mask = 1u64 << (index % 64);
        let fresh = self.0[word] & mask == 0;
        self.0[word] |= mask;
        fresh
    }

    fn remove(&mut self, index: usize) {
        if let Some(word) = self.0.get_mut(index / 64) {
            *word &= !(1u64 << (index % 64));
        }
    }

    fn contains(&self, index: usize) -> bool {
        self.0.get(index / 64).is_some_and(|word| word >> (index % 64) & 1 == 1)
    }

    fn union(&mut self, other: &Bits) -> bool {
        if other.0.len() > self.0.len() {
            self.0.resize(other.0.len(), 0);
        }
        let mut changed = false;
        for (word, &more) in self.0.iter_mut().zip(&other.0) {
            let next = *word | more;
            changed |= next != *word;
            *word = next;
        }
        changed
    }

    fn is_empty(&self) -> bool {
        self.0.iter().all(|&word| word == 0)
    }

    fn iter(&self) -> BitsIter<'_> {
        BitsIter { words: &self.0, word_index: 0, current: 0 }
    }
}

struct BitsIter<'a> {
    words: &'a [u64],
    word_index: usize,
    current: u64,
}

impl Iterator for BitsIter<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        while self.current == 0 {
            if self.word_index >= self.words.len() {
                return None;
            }
            self.current = self.words[self.word_index];
            self.word_index += 1;
        }
        let bit = self.current.trailing_zeros() as usize;
        self.current &= self.current - 1;
        Some((self.word_index - 1) * 64 + bit)
    }
}

/// What a site stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Anything the function did not make; always escaped.
    Unknown,
    /// What the caller passed for a parameter or a capture.
    Input,
    /// The object an instruction makes; `closure` is the body of a `Closure`.
    Alloc { closure: Option<FuncId> },
}

const UNKNOWN: usize = 0;

fn input(position: usize) -> usize {
    1 + position
}

/// Ops whose result is a value they read, not a new object.
fn is_alias(op: &Op) -> bool {
    matches!(
        op,
        Op::Copy(_) | Op::Convert { .. } | Op::SomePayload(_) | Op::Unwrap(_) | Op::UnionPayload { .. } | Op::Cast(_) | Op::Field { .. } | Op::TupleField { .. } | Op::VariantField { .. } | Op::Index { .. } | Op::LoadCell(_) | Op::LoadGlobal(_) | Op::Call { .. }
    )
}

/// A type with no object to track.
fn scalar(program: &Program, ty: TyId) -> bool {
    match program.types.kind(ty) {
        TyKind::Unit | TyKind::Bool | TyKind::Int(_) | TyKind::Float(_) | TyKind::Error => true,
        TyKind::Optional(inner) => scalar(program, *inner),
        _ => false,
    }
}

/// The facts of one function.
pub(super) struct Outcome {
    pub(super) summary: Summary,
    /// Per block and instruction: the object it makes.
    pub(super) allocations: Vec<Vec<Option<Allocation>>>,
    /// Per register: every object it may hold is frozen.
    pub(super) frozen: Vec<bool>,
}

pub(super) fn run(module: &Module, program: &Program, interner: &Interner, summaries: &[Summary], function: &Function) -> Outcome {
    let mut analysis = Analysis::new(module, program, interner, summaries, function);
    analysis.solve();
    analysis.finish()
}

struct Analysis<'a> {
    module: &'a Module,
    interner: &'a Interner,
    summaries: &'a [Summary],
    function: &'a Function,
    tracked: Vec<bool>,
    kinds: Vec<Kind>,
    site_at: HashMap<(usize, usize), usize>,
    pts: Vec<Bits>,
    contents: Vec<Bits>,
    written: Bits,
    roots: Bits,
    changed: bool,
}

impl<'a> Analysis<'a> {
    fn new(module: &'a Module, program: &'a Program, interner: &'a Interner, summaries: &'a [Summary], function: &'a Function) -> Self {
        let tracked: Vec<bool> = function.regs.iter().map(|(_, reg)| reg.cell || !scalar(program, reg.ty)).collect();
        let inputs = function.params.len() + function.captures.len();
        let mut kinds = vec![Kind::Unknown];
        kinds.extend((0..inputs).map(|_| Kind::Input));
        let mut site_at = HashMap::new();
        let mut pts = vec![Bits::default(); function.regs.len()];
        for (position, reg) in function.params.iter().chain(&function.captures).enumerate() {
            if tracked[reg.index()] {
                pts[reg.index()].insert(input(position));
            }
        }
        for (block, data) in function.blocks.iter() {
            for (at, inst) in data.insts.iter().enumerate() {
                if let Some(dest) = inst.dest
                    && tracked[dest.index()]
                    && !is_alias(&inst.op)
                {
                    pts[dest.index()].insert(kinds.len());
                    site_at.insert((block.index(), at), kinds.len());
                    kinds.push(Kind::Alloc { closure: if let Op::Closure { func, .. } = &inst.op { Some(*func) } else { None } });
                }
            }
        }
        let mut contents = vec![Bits::default(); kinds.len()];
        contents[UNKNOWN].insert(UNKNOWN);
        for position in 0..inputs {
            contents[input(position)].insert(input(position));
        }
        Self { module, interner, summaries, function, tracked, kinds, site_at, pts, contents, written: Bits::default(), roots: Bits::default(), changed: false }
    }

    fn solve(&mut self) {
        let function = self.function;
        loop {
            self.changed = false;
            for (block, data) in function.blocks.iter() {
                for (at, inst) in data.insts.iter().enumerate() {
                    self.step(block.index(), at, inst.dest, &inst.op);
                }
            }
            if !self.changed {
                break;
            }
        }
    }

    fn operand(&self, operand: &Operand) -> Bits {
        match operand {
            Operand::Reg(reg) => self.pts[reg.index()].clone(),
            Operand::Const(_) => Bits::default(),
        }
    }

    fn operands<'o>(&self, operands: impl IntoIterator<Item = &'o Operand>) -> Bits {
        let mut all = Bits::default();
        for operand in operands {
            all.union(&self.operand(operand));
        }
        all
    }

    /// The sites held by the objects of `set`.
    fn loaded(&self, set: &Bits) -> Bits {
        let mut out = Bits::default();
        for site in set.iter() {
            out.union(&self.contents[site]);
        }
        out
    }

    /// `set` and everything reachable from it.
    fn reach(&self, set: &Bits) -> Bits {
        let mut out = set.clone();
        let mut work: Vec<usize> = set.iter().collect();
        while let Some(site) = work.pop() {
            for next in self.contents[site].iter() {
                if out.insert(next) {
                    work.push(next);
                }
            }
        }
        out
    }

    fn flow(&mut self, dest: Option<RegId>, set: &Bits) {
        if let Some(reg) = dest
            && self.tracked[reg.index()]
            && self.pts[reg.index()].union(set)
        {
            self.changed = true;
        }
    }

    fn mark_written(&mut self, set: &Bits) {
        self.changed |= self.written.union(set);
    }

    fn mark_root(&mut self, set: &Bits) {
        self.changed |= self.roots.union(set);
    }

    /// Puts `value` into the objects of `object`; into one the function does not own it escapes.
    fn store(&mut self, object: &Bits, value: &Bits) {
        for site in object.iter() {
            self.changed |= self.contents[site].union(value);
            if !matches!(self.kinds[site], Kind::Alloc { .. }) {
                self.changed |= self.roots.union(value);
            }
        }
    }

    /// What the object the instruction makes holds.
    fn fill(&mut self, block: usize, at: usize, held: &Bits) {
        if let Some(&site) = self.site_at.get(&(block, at)) {
            self.changed |= self.contents[site].union(held);
        }
    }

    fn step(&mut self, block: usize, at: usize, dest: Option<RegId>, op: &Op) {
        match op {
            Op::Copy(a) | Op::Convert { operand: a, .. } | Op::SomePayload(a) | Op::Unwrap(a) | Op::UnionPayload { operand: a, .. } | Op::Cast(a) => {
                let set = self.operand(a);
                self.flow(dest, &set);
            }
            Op::Field { object, .. } | Op::TupleField { object, .. } | Op::VariantField { object, .. } | Op::Index { object, .. } | Op::LoadCell(object) => {
                let set = self.loaded(&self.operand(object));
                self.flow(dest, &set);
            }
            Op::LoadGlobal(_) | Op::FrameLoad(_) => self.flow(dest, &Bits::of(UNKNOWN)),
            Op::SetField { object, value, .. } | Op::SetIndex { object, value, .. } | Op::SetSlice { object, value, .. } | Op::StoreCell { cell: object, value } => {
                let held = self.operand(object);
                let put = self.operand(value);
                self.mark_written(&held);
                self.store(&held, &put);
            }
            Op::StoreGlobal { value: held, .. } | Op::Defer { resource: held, .. } | Op::Yield(held) | Op::FrameStore { value: held, .. } => {
                let set = self.operand(held);
                self.mark_root(&set);
            }
            Op::Spawn { captures, .. } | Op::Stream { init: captures, .. } => {
                let set = self.operands(captures);
                self.mark_root(&set);
                self.fill(block, at, &set);
            }
            Op::Slice { object, .. } | Op::Detach(object) => {
                let set = self.loaded(&self.operand(object));
                self.fill(block, at, &set);
            }
            Op::Call { callee, args, outs } => self.call(callee, args, dest, outs),
            _ => {
                let mut set = Bits::default();
                for reg in reads(op) {
                    set.union(&self.pts[reg.index()]);
                }
                self.fill(block, at, &set);
            }
        }
    }

    fn call(&mut self, callee: &Callee, args: &[Operand], dest: Option<RegId>, outs: &[RegId]) {
        let sets: Vec<Bits> = args.iter().map(|arg| self.operand(arg)).collect();
        let mut all = Bits::default();
        for set in &sets {
            all.union(set);
        }
        let needs_result = dest.is_some_and(|reg| self.tracked[reg.index()]) || outs.iter().any(|&out| self.tracked[out.index()]);
        let mut result = if needs_result {
            let mut reachable = self.reach(&all);
            reachable.insert(UNKNOWN);
            reachable
        } else {
            Bits::default()
        };
        match callee {
            Callee::Function(func) => self.apply(*func, &sets),
            Callee::Value(function) => {
                let held = self.operand(function);
                let mut unknown = held.is_empty();
                for site in held.iter() {
                    match self.kinds[site] {
                        Kind::Alloc { closure: Some(func) } => {
                            self.apply(func, &sets);
                            let params = self.module.functions[func].params.len();
                            let summary = &self.summaries[func.index()];
                            let (escapes, writes) = (summary.escapes[params..].iter().any(|&flag| flag), summary.writes[params..].iter().any(|&flag| flag));
                            let inside = self.reach(&self.contents[site].clone());
                            if escapes {
                                self.mark_root(&inside);
                            }
                            if writes {
                                self.mark_written(&inside);
                            }
                            if needs_result {
                                result.union(&inside);
                            }
                        }
                        _ => unknown = true,
                    }
                }
                if unknown {
                    let mut everything = self.reach(&all);
                    everything.union(&self.reach(&held));
                    self.mark_root(&everything);
                    self.mark_written(&everything);
                }
            }
            Callee::Native(_) => self.opaque(&all),
            Callee::Intrinsic(name) => match intrinsics::lookup(self.interner.resolve(*name)) {
                Some(row) => {
                    if row.parks() || row.writes() {
                        let everything = self.reach(&all);
                        if row.parks() {
                            self.mark_root(&everything);
                        }
                        if row.writes() {
                            self.mark_written(&everything);
                            for (index, set) in sets.iter().enumerate() {
                                let mut others = Bits::default();
                                for (other, held) in sets.iter().enumerate() {
                                    if other != index {
                                        others.union(held);
                                    }
                                }
                                self.store(set, &others);
                            }
                        }
                    }
                }
                None => self.opaque(&all),
            },
        }
        if needs_result {
            self.flow(dest, &result);
            for &out in outs {
                self.flow(Some(out), &result);
            }
        }
    }

    /// A call to code the analysis cannot see: it may keep and write everything it is given.
    fn opaque(&mut self, all: &Bits) {
        let everything = self.reach(all);
        self.mark_root(&everything);
        self.mark_written(&everything);
    }

    /// What a function does to each argument, by its summary.
    fn apply(&mut self, func: FuncId, sets: &[Bits]) {
        let summaries = self.summaries;
        let summary = &summaries[func.index()];
        let params = self.module.functions[func].params.len();
        for (position, set) in sets.iter().enumerate() {
            let reached = self.reach(set);
            if position >= params || summary.escapes[position] {
                self.mark_root(&reached);
            }
            if position >= params || summary.writes[position] {
                self.mark_written(&reached);
            }
        }
    }

    /// Whether `inst` only copies a `var` parameter back from a call that did not write it: `a = copy param`, `call f(a) -> b`, `param = copy b`.
    fn hands_back(&self, param: RegId, inst: &Inst, defs: &HashMap<RegId, Vec<(usize, usize)>>, blocks: &[Block]) -> bool {
        let (Some(dest), Op::Copy(Operand::Reg(back))) = (inst.dest, &inst.op) else { return false };
        if dest != param {
            return false;
        }
        let Some([(block, at)]) = defs.get(back).map(Vec::as_slice) else { return false };
        let Op::Call { callee: Callee::Function(func), args, outs } = &blocks[*block].insts[*at].op else { return false };
        let target = &self.module.functions[*func];
        let Some(&position) = outs.iter().position(|out| out == back).and_then(|k| target.out_params.get(k)) else { return false };
        if self.summaries[func.index()].writes[position] {
            return false;
        }
        let Some(Operand::Reg(given)) = args.get(position) else { return false };
        defs.get(given).is_some_and(|places| places.iter().all(|&(block, at)| matches!(&blocks[block].insts[at].op, Op::Copy(Operand::Reg(from)) if *from == param && blocks[block].insts[at].dest == Some(*given))))
    }

    fn finish(mut self) -> Outcome {
        let function = self.function;
        for (_, data) in function.blocks.iter() {
            if let Terminator::Return(Some(Operand::Reg(reg))) = &data.term {
                let set = self.pts[reg.index()].clone();
                self.roots.union(&set);
            }
        }
        for &position in &function.out_params {
            let mut set = self.pts[function.params[position].index()].clone();
            set.remove(input(position));
            self.roots.union(&set);
        }
        let mut escaped = self.reach(&self.roots);
        escaped.insert(UNKNOWN);
        let frozen: Vec<bool> = (0..self.kinds.len())
            .map(|site| matches!(self.kinds[site], Kind::Alloc { .. }) && self.reach(&Bits::of(site)).iter().all(|held| matches!(self.kinds[held], Kind::Alloc { .. }) && !self.written.contains(held) && !escaped.contains(held)))
            .collect();
        let allocations = function
            .blocks
            .iter()
            .map(|(block, data)| {
                (0..data.insts.len())
                    .map(|at| self.site_at.get(&(block.index(), at)).map(|&site| Allocation { escapes: escaped.contains(site), written: self.written.contains(site), frozen: frozen[site] }))
                    .collect()
            })
            .collect();
        let held_frozen = self.pts.iter().map(|set| !set.is_empty() && set.iter().all(|site| frozen[site])).collect();
        let inputs: Vec<RegId> = function.params.iter().chain(&function.captures).copied().collect();
        let mut defs: HashMap<RegId, Vec<(usize, usize)>> = HashMap::new();
        for (block, data) in function.blocks.iter() {
            for (at, inst) in data.insts.iter().enumerate() {
                for reg in writes(inst) {
                    defs.entry(reg).or_default().push((block.index(), at));
                }
            }
        }
        let blocks = function.blocks.as_slice();
        let assigned: HashSet<RegId> = inputs
            .iter()
            .copied()
            .filter(|reg| defs.get(reg).is_some_and(|places| places.iter().any(|&(block, at)| !self.hands_back(*reg, &blocks[block].insts[at], &defs, blocks))))
            .collect();
        let summary = Summary {
            escapes: (0..inputs.len()).map(|position| escaped.contains(input(position))).collect(),
            writes: inputs.iter().enumerate().map(|(position, reg)| self.written.contains(input(position)) || assigned.contains(reg)).collect(),
        };
        Outcome { summary, allocations, frozen: held_frozen }
    }
}
