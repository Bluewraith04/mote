//! Marking: a precise scan from the roots through the slots of the type table, setting the object and line marks.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use mote_abi::{FrameDescriptor, Header, Object as Shape, PairPayload, Slot, TypeId, TypeTable, FLAG_LARGE, FLAG_STATIC, LENGTH_OFFSET};

use crate::blocks::{Pool, LINE_SIZE};
use crate::object::{read_uint, read_word, Object};

/// Entries a marker keeps to itself before it shares half of them.
const SPILL_AT: usize = 128;

/// Addresses below this are never read for a header when the check looks for a static object.
const STATIC_FLOOR: usize = 4096;

/// What the program holds that the collector starts from.
pub trait Roots {
    /// Reports every root to the marker.
    fn visit(&mut self, marker: &mut Marker<'_>);

    /// Empties each allocation region, whose memory the heap reuses.
    fn retire(&mut self);
}

/// The large objects of one cycle by address, each with its mark.
pub(crate) struct LargeMarks {
    index: HashMap<usize, usize>,
    marks: Vec<AtomicBool>,
}

impl LargeMarks {
    pub fn new(addresses: impl Iterator<Item = usize>) -> Self {
        let index: HashMap<usize, usize> = addresses.enumerate().map(|(at, address)| (address, at)).collect();
        let marks = (0..index.len()).map(|_| AtomicBool::new(false)).collect();
        Self { index, marks }
    }

    /// The objects with every one already marked, as after a collection.
    pub fn all_marked(addresses: impl Iterator<Item = usize>) -> Self {
        let this = Self::new(addresses);
        for mark in &this.marks {
            mark.store(true, Ordering::Relaxed);
        }
        this
    }

    /// Marks the object; true when it was not marked.
    fn mark(&self, address: usize) -> bool {
        !self.marks[self.index[&address]].swap(true, Ordering::Relaxed)
    }

    pub fn contains(&self, address: usize) -> bool {
        self.index.contains_key(&address)
    }

    pub fn is_marked(&self, address: usize) -> bool {
        self.index.get(&address).is_some_and(|&at| self.marks[at].load(Ordering::Relaxed))
    }
}

/// The check that replaces marking after a collection: every reference found must be a marked live object.
pub(crate) struct Checker {
    /// Whether each block holds objects.
    pub in_use: Vec<bool>,
    seen: Mutex<HashSet<usize>>,
    pub problems: Mutex<Vec<String>>,
}

impl Checker {
    pub fn new(in_use: Vec<bool>) -> Self {
        Self { in_use, seen: Mutex::new(HashSet::new()), problems: Mutex::new(Vec::new()) }
    }

    fn problem(&self, message: String) {
        self.problems.lock().expect("the problem list").push(message);
    }
}

/// One marking cycle: the metadata it sets, the work shared between markers and what it found.
pub(crate) struct Cycle<'a> {
    pub table: &'a TypeTable,
    pub pool: &'a Pool,
    pub large: &'a LargeMarks,
    queues: Vec<Mutex<Vec<usize>>>,
    pending: AtomicUsize,
    pub objects: AtomicUsize,
    pub bytes: AtomicUsize,
    check: Option<&'a Checker>,
}

impl<'a> Cycle<'a> {
    pub fn new(table: &'a TypeTable, pool: &'a Pool, large: &'a LargeMarks, workers: usize) -> Self {
        Self {
            table,
            pool,
            large,
            queues: (0..workers.max(1)).map(|_| Mutex::new(Vec::new())).collect(),
            pending: AtomicUsize::new(0),
            objects: AtomicUsize::new(0),
            bytes: AtomicUsize::new(0),
            check: None,
        }
    }

    pub fn checking(mut self, checker: &'a Checker) -> Self {
        self.check = Some(checker);
        self
    }
}

/// A marker follows references from the roots on one thread.
pub struct Marker<'c> {
    cycle: &'c Cycle<'c>,
    worker: usize,
    local: Vec<usize>,
}

impl<'c> Marker<'c> {
    pub(crate) fn new(cycle: &'c Cycle<'c>, worker: usize) -> Self {
        Self { cycle, worker, local: Vec::new() }
    }

    pub fn table(&self) -> &TypeTable {
        self.cycle.table
    }

    /// Marks an object and, later, what it refers to.
    pub fn mark(&mut self, object: Option<Object>) {
        if let Some(object) = object {
            self.mark_address(object.address() as u64);
        }
    }

    /// Marks the object a reference word points at; null points at none.
    pub fn mark_address(&mut self, address: u64) {
        self.mark_ref(address as usize);
    }

    /// Marks what the frame holds at `base`: its references and its dynamic slots.
    pub fn mark_frame(&mut self, base: usize, frame: &FrameDescriptor) {
        for &offset in &frame.refs {
            self.mark_ref(read_word(base + offset as usize) as usize);
        }
        self.mark_value(base, &frame.dynamic);
    }

    /// Marks what the slots of a value at `base` hold.
    pub fn mark_value(&mut self, base: usize, slots: &[Slot]) {
        for slot in slots {
            self.scan_slot(base, *slot);
        }
    }

    fn mark_ref(&mut self, address: usize) {
        if address == 0 {
            return;
        }
        if let Some(checker) = self.cycle.check {
            self.check_ref(checker, address);
            return;
        }
        let cycle = self.cycle;
        let header = Header::from_bits(read_word(address));
        if header.has(FLAG_STATIC) {
            return;
        }
        let shape = shape_of(cycle.table, header.type_id());
        let size = size_of(shape, address);
        let fresh = if header.has(FLAG_LARGE) {
            cycle.large.mark(address)
        } else {
            let block = cycle.pool.block_of(address).expect("a reference points into a block");
            let offset = address - cycle.pool.base(block);
            let marks = cycle.pool.marks(block);
            let fresh = marks.objects.set(offset / 8);
            if fresh {
                marks.lines.mark_range(offset / LINE_SIZE, (offset + size - 1) / LINE_SIZE);
            }
            fresh
        };
        if fresh {
            cycle.objects.fetch_add(1, Ordering::Relaxed);
            cycle.bytes.fetch_add(size, Ordering::Relaxed);
            if holds_references(cycle.table, shape) {
                self.push(address);
            }
        }
    }

    fn check_ref(&mut self, checker: &Checker, address: usize) {
        let cycle = self.cycle;
        let problem = |what: &str| checker.problem(format!("reference {address:#x} {what}"));
        if !address.is_multiple_of(8) {
            return problem("is not word-aligned");
        }
        let block = cycle.pool.block_of(address);
        match block {
            Some(block) if !checker.in_use[block.index()] => return problem("points into a free block"),
            None if !cycle.large.contains(address) => {
                if address < STATIC_FLOOR || !Header::from_bits(read_word(address)).has(FLAG_STATIC) {
                    return problem("points outside the heap");
                }
                return;
            }
            _ => {}
        }
        let header = Header::from_bits(read_word(address));
        let Some(shape) = cycle.table.get_raw(header.type_id().raw()).and_then(|info| info.object.as_ref()) else {
            return problem("has a header that names no object type");
        };
        let size = size_of(shape, address);
        match block {
            Some(block) => {
                let base = cycle.pool.base(block);
                let offset = address - base;
                let marks = cycle.pool.marks(block);
                if header.has(FLAG_LARGE) {
                    return problem("is in a block but flagged large");
                }
                if !marks.objects.is_set(offset / 8) {
                    return problem("is reachable but not marked");
                }
                if offset + size > crate::blocks::BLOCK_SIZE {
                    return problem("runs past the end of its block");
                }
                if !(offset / LINE_SIZE..=(offset + size - 1) / LINE_SIZE).all(|line| marks.lines.is_marked(line)) {
                    return problem("lies on a line that is not marked");
                }
            }
            None => {
                if !header.has(FLAG_LARGE) {
                    return problem("is large but not flagged");
                }
                if !cycle.large.is_marked(address) {
                    return problem("is reachable but not marked");
                }
            }
        }
        if checker.seen.lock().expect("the visited set").insert(address) && holds_references(cycle.table, shape) {
            self.push(address);
        }
    }

    fn push(&mut self, address: usize) {
        self.cycle.pending.fetch_add(1, Ordering::SeqCst);
        self.local.push(address);
        if self.local.len() > SPILL_AT {
            let share: Vec<usize> = self.local.drain(..SPILL_AT / 2).collect();
            self.cycle.queues[self.worker].lock().expect("a work queue").extend(share);
        }
    }

    /// Moves everything this marker holds to its shared queue, where others can take it.
    pub(crate) fn spill_all(&mut self) {
        let all = std::mem::take(&mut self.local);
        self.cycle.queues[self.worker].lock().expect("a work queue").extend(all);
    }

    fn scan(&mut self, address: usize) {
        let table = self.cycle.table;
        let header = Header::from_bits(read_word(address));
        let shape = shape_of(table, header.type_id());
        for slot in &shape.slots {
            self.scan_slot(address, *slot);
        }
        if let Some(element) = shape.element
            && let Some(ty) = element.ty
        {
            let slots = &table.get(ty).slots;
            if !slots.is_empty() {
                let count = read_word(address + LENGTH_OFFSET as usize) as usize;
                for at in 0..count {
                    let base = address + element.offset as usize + at * element.stride as usize;
                    for slot in slots {
                        self.scan_slot(base, *slot);
                    }
                }
            }
        }
    }

    fn scan_slot(&mut self, base: usize, slot: Slot) {
        let table = self.cycle.table;
        match slot {
            Slot::Ref(offset) => self.mark_ref(read_word(base + offset as usize) as usize),
            Slot::Enum { offset, ty } => {
                let info = table.get(ty);
                let tag = read_uint(base + offset as usize, info.tag_size);
                if let Some(variant) = info.variants.get(tag as usize) {
                    for inner in &variant.slots {
                        self.scan_slot(base + offset as usize, *inner);
                    }
                }
            }
            Slot::Pair(offset) => {
                let at = base + offset as usize;
                let ty = TypeId::from_raw(read_word(at) as u32);
                if ty != TypeId::NONE && table.pair_payload(ty) != PairPayload::Inline {
                    self.mark_ref(read_word(at + mote_abi::PAIR_PAYLOAD_OFFSET as usize) as usize);
                }
            }
            Slot::Repeat { offset, count, stride, ty } => {
                let slots = &table.get(ty).slots;
                for at in 0..count as usize {
                    for inner in slots {
                        self.scan_slot(base + offset as usize + at * stride as usize, *inner);
                    }
                }
            }
        }
    }

    /// Scans until no marker has work left; others steal from the shared queues meanwhile.
    pub(crate) fn drain(&mut self) {
        let cycle = self.cycle;
        loop {
            while let Some(address) = self.local.pop() {
                self.scan(address);
                cycle.pending.fetch_sub(1, Ordering::SeqCst);
            }
            if self.take_own() || self.steal() {
                continue;
            }
            if cycle.pending.load(Ordering::SeqCst) == 0 {
                return;
            }
            std::thread::yield_now();
        }
    }

    fn take_own(&mut self) -> bool {
        let mut queue = self.cycle.queues[self.worker].lock().expect("a work queue");
        let keep = queue.len().saturating_sub(SPILL_AT);
        self.local.extend(queue.drain(keep..));
        !self.local.is_empty()
    }

    fn steal(&mut self) -> bool {
        let others = self.cycle.queues.len();
        for step in 1..others {
            let mut queue = self.cycle.queues[(self.worker + step) % others].lock().expect("a work queue");
            let take = queue.len().div_ceil(2);
            if take > 0 {
                self.local.extend(queue.drain(..take));
                return true;
            }
        }
        false
    }
}

fn shape_of(table: &TypeTable, ty: TypeId) -> &Shape {
    table.get(ty).object.as_ref().expect("a header names an object type")
}

/// Bytes of the object at the address, from its type and, for an array or string, its length word.
pub(crate) fn size_of(shape: &Shape, address: usize) -> usize {
    let count = if shape.element.is_some() { read_word(address + LENGTH_OFFSET as usize) } else { 0 };
    shape.byte_size(count) as usize
}

/// Whether an object of the shape can hold a reference.
fn holds_references(table: &TypeTable, shape: &Shape) -> bool {
    !shape.slots.is_empty() || shape.element.and_then(|element| element.ty).is_some_and(|ty| !table.get(ty).slots.is_empty())
}

/// Runs a marking cycle from the roots on `workers` threads, the first of which is the caller.
pub(crate) fn run(cycle: &Cycle<'_>, roots: &mut dyn Roots, workers: usize) {
    let mut main = Marker::new(cycle, 0);
    roots.visit(&mut main);
    if workers <= 1 {
        main.drain();
        return;
    }
    main.spill_all();
    std::thread::scope(|scope| {
        for worker in 1..workers {
            scope.spawn(move || Marker::new(cycle, worker).drain());
        }
        main.drain();
    });
}
