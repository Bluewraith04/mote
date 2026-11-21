//! The heap: allocation from regions, the large-object space, the limit, and collection with a lazy sweep.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use mote_abi::{is_finalized, FaultKind, Header, Region, TypeId, TypeTable, FLAG_FINALIZE, FLAG_LARGE, LENGTH_OFFSET};

use crate::blocks::{BlockId, Pool, BLOCK_SIZE, CHUNK_BLOCKS, LINE_SIZE};
use crate::mark::{self, Checker, Cycle, LargeMarks, Roots};
use crate::object::{read_word, Object};
use crate::stats::{Census, Stats};
use crate::sys::{self, PAGE_SIZE};

/// Largest object, in bytes, that lives in a block; a larger one gets a mapping of its own.
pub const LARGE_THRESHOLD: usize = 8 * 1024;

/// Settings of a heap.
#[derive(Clone, Debug)]
pub struct Config {
    /// Bytes of blocks in use plus large objects that an allocation may not pass.
    pub limit: usize,
    /// Bytes handed out since the last collection at which `wants_collection` turns true.
    pub trigger: usize,
    /// Empty blocks kept warm; the rest are given back to the system.
    pub retain: usize,
    /// Collect before every allocation made through `alloc_collecting`.
    pub torture: bool,
    /// Check after each collection that every reachable object is marked and no reference dangles.
    pub verify: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self { limit: 1 << 30, trigger: 8 << 20, retain: 16, torture: false, verify: false }
    }
}

/// An allocation found no room: a collection may make some.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Full;

/// An allocation found no room after a collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutOfMemory {
    pub limit: usize,
}

impl OutOfMemory {
    /// The fault a program stops with.
    pub fn fault(self) -> FaultKind {
        FaultKind::MemoryLimit
    }
}

/// What a collection found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Collection {
    pub marked_objects: usize,
    pub marked_bytes: usize,
    /// Large objects given back.
    pub large_freed: usize,
    /// Objects whose finalizer ran.
    pub finalized: usize,
    /// What the check after the collection found wrong; empty when the heap is sound or the check is off.
    pub problems: Vec<String>,
}

/// Called with each dying object that owns an OS resource, while the heap is locked; it reads the object's words and does not use the heap.
pub type Finalizer = Box<dyn FnMut(Object) + Send>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlockState {
    /// Holds no object.
    Free,
    /// Marked by the last collection and not yet swept.
    Unswept,
    /// Swept, with holes left to hand out.
    Recyclable,
    /// Handed out whole or without a usable hole.
    Used,
}

struct BlockInfo {
    state: BlockState,
    /// Not touched since it was mapped or given back: its memory is zero.
    fresh: bool,
    /// Where the next hole search of a recyclable block starts.
    next_line: usize,
}

struct Large {
    address: usize,
    mapped: usize,
}

struct Inner {
    pool: Pool,
    info: Vec<BlockInfo>,
    free: Vec<BlockId>,
    /// Free blocks that are not fresh.
    warm_free: usize,
    recyclable: VecDeque<BlockId>,
    unswept: VecDeque<BlockId>,
    large: Vec<Large>,
    used_blocks: usize,
    large_bytes: usize,
    finalizable: Vec<usize>,
    finalizer: Option<Finalizer>,
    retain: usize,
    stats: Stats,
}

impl Inner {
    fn used_bytes(&self) -> usize {
        self.used_blocks * BLOCK_SIZE + self.large_bytes
    }

    fn note_peak(&mut self) {
        self.stats.peak_used = self.stats.peak_used.max(self.used_bytes());
    }

    fn add_chunk(&mut self) -> Result<(), Full> {
        let first = self.pool.grow().map_err(|_| Full)?.index();
        for at in 0..CHUNK_BLOCKS {
            self.info.push(BlockInfo { state: BlockState::Free, fresh: true, next_line: 0 });
            debug_assert_eq!(self.info.len() - 1, first + at);
        }
        let ids: Vec<BlockId> = self.pool.ids().skip(first).collect();
        self.free.extend(ids.into_iter().rev());
        Ok(())
    }

    /// The next hole of at least `lines` lines, zeroed, as an address range: from the block being recycled, a swept block, a free block or a new chunk.
    fn take_hole(&mut self, lines: usize, limit: usize) -> Result<(usize, usize), Full> {
        loop {
            if let Some(&block) = self.recyclable.front() {
                let next = self.info[block.index()].next_line;
                if let Some(hole) = self.pool.marks(block).lines.next_hole(next, lines) {
                    self.info[block.index()].next_line = hole.first + hole.lines;
                    let start = hole.start(self.pool.base(block));
                    zero(start, hole.bytes());
                    self.stats.holes += 1;
                    return Ok((start, start + hole.bytes()));
                }
                self.recyclable.pop_front();
                self.info[block.index()].state = BlockState::Used;
                continue;
            }
            if let Some(block) = self.unswept.pop_front() {
                self.sweep_block(block);
                continue;
            }
            if self.used_bytes() + BLOCK_SIZE > limit {
                return Err(Full);
            }
            if self.free.is_empty() {
                self.add_chunk()?;
            }
            let block = self.free.pop().expect("a free block");
            let start = self.pool.base(block);
            let info = &mut self.info[block.index()];
            info.state = BlockState::Used;
            if std::mem::replace(&mut info.fresh, false) {
                // Never touched: still zero.
            } else {
                self.warm_free -= 1;
                zero(start, BLOCK_SIZE);
            }
            self.used_blocks += 1;
            self.stats.blocks_handed += 1;
            self.note_peak();
            return Ok((start, start + BLOCK_SIZE));
        }
    }

    /// Turns the marks of an unswept block into its state: free, with holes, or full.
    fn sweep_block(&mut self, block: BlockId) {
        debug_assert_eq!(self.info[block.index()].state, BlockState::Unswept);
        self.stats.swept_blocks += 1;
        let lines = &self.pool.marks(block).lines;
        if lines.marked() == 0 {
            self.free_block(block);
        } else if lines.next_hole(0, 1).is_some() {
            let info = &mut self.info[block.index()];
            info.state = BlockState::Recyclable;
            info.next_line = 0;
            self.recyclable.push_back(block);
        } else {
            self.info[block.index()].state = BlockState::Used;
        }
    }

    fn free_block(&mut self, block: BlockId) {
        self.used_blocks -= 1;
        let discard = self.warm_free >= self.retain;
        let info = &mut self.info[block.index()];
        info.state = BlockState::Free;
        info.next_line = 0;
        info.fresh = discard;
        if discard {
            let memory = std::ptr::NonNull::new(self.pool.base(block) as *mut u8).expect("a block is mapped");
            // SAFETY: the block is whole pages of the pool's mapping and holds no live object.
            unsafe { sys::discard(memory, BLOCK_SIZE) };
            self.stats.discarded_blocks += 1;
        } else {
            self.warm_free += 1;
        }
        self.free.push(block);
    }

    fn sweep_all(&mut self) {
        while let Some(block) = self.unswept.pop_front() {
            self.sweep_block(block);
        }
    }
}

fn zero(address: usize, bytes: usize) {
    // SAFETY: the range is free memory of the heap, inside one mapped block.
    unsafe { std::ptr::write_bytes(address as *mut u8, 0, bytes) };
}

/// Runs the finalizer on each finalizable object the cycle did not mark and forgets it; the count.
fn finalize(inner: &mut Inner, large: &LargeMarks) -> usize {
    let candidates = std::mem::take(&mut inner.finalizable);
    let mut finalizer = inner.finalizer.take();
    let mut count = 0;
    for address in candidates {
        let marked = if Header::from_bits(read_word(address)).has(FLAG_LARGE) {
            large.is_marked(address)
        } else {
            let block = inner.pool.block_of(address).expect("a finalizable object is in a block");
            inner.pool.marks(block).objects.is_set((address - inner.pool.base(block)) / 8)
        };
        if marked {
            inner.finalizable.push(address);
        } else {
            count += 1;
            if let (Some(finalizer), Some(object)) = (finalizer.as_mut(), Object::from_address(address)) {
                finalizer(object);
            }
        }
    }
    inner.finalizer = finalizer;
    count
}

/// Gives back the large objects the cycle did not mark; the count.
fn free_large(inner: &mut Inner, large: &LargeMarks) -> usize {
    let before = inner.large.len();
    let mut freed_bytes = 0;
    inner.large.retain(|entry| {
        if large.is_marked(entry.address) {
            return true;
        }
        let memory = std::ptr::NonNull::new(entry.address as *mut u8).expect("a large object is mapped");
        // SAFETY: the mapping was made by `alloc_large` and nothing reachable refers to it.
        unsafe { sys::unmap(memory, entry.mapped) };
        freed_bytes += entry.mapped;
        false
    });
    inner.large_bytes -= freed_bytes;
    before - inner.large.len()
}

/// A non-moving heap of objects described by a type table.
pub struct Heap {
    table: Arc<TypeTable>,
    config: Config,
    inner: Mutex<Inner>,
    /// Bytes handed out since the last collection.
    since: AtomicUsize,
    /// Bytes handed out after which a collection is wanted.
    gap: AtomicUsize,
    allocations: AtomicUsize,
}

impl Heap {
    pub fn new(table: Arc<TypeTable>, config: Config) -> Self {
        let inner = Inner {
            pool: Pool::new(),
            info: Vec::new(),
            free: Vec::new(),
            warm_free: 0,
            recyclable: VecDeque::new(),
            unswept: VecDeque::new(),
            large: Vec::new(),
            used_blocks: 0,
            large_bytes: 0,
            finalizable: Vec::new(),
            finalizer: None,
            retain: config.retain,
            stats: Stats::default(),
        };
        Self { table, gap: AtomicUsize::new(config.trigger), config, inner: Mutex::new(inner), since: AtomicUsize::new(0), allocations: AtomicUsize::new(0) }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().expect("the heap")
    }

    pub fn table(&self) -> &TypeTable {
        &self.table
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Sets what runs for each dying finalizable object.
    pub fn set_finalizer(&self, finalizer: Finalizer) {
        self.lock().finalizer = Some(finalizer);
    }

    /// Whether the heap has handed out enough since the last collection to want another.
    pub fn wants_collection(&self) -> bool {
        self.since.load(Ordering::Relaxed) >= self.gap.load(Ordering::Relaxed)
    }

    fn finalizes(&self, ty: TypeId) -> bool {
        is_finalized(&self.table.get(ty).kind)
    }

    /// A zeroed object of the type with its header written; `count` is the element count of a string or an array and 0 for any other type.
    pub fn alloc(&self, region: &mut Region, ty: TypeId, count: u64) -> Result<Object, Full> {
        let shape = self.table.get(ty).object.as_ref().expect("an allocation names an object type");
        assert!(shape.element.is_some() || count == 0, "only a string or an array has elements");
        if count > self.config.limit as u64 {
            return Err(Full);
        }
        let size = shape.byte_size(count) as usize;
        self.allocations.fetch_add(1, Ordering::Relaxed);
        let mut flags = if self.finalizes(ty) { FLAG_FINALIZE } else { 0 };
        let object = if size > LARGE_THRESHOLD {
            flags |= FLAG_LARGE;
            self.alloc_large(size)?
        } else {
            let address = match region.bump(size as u32) {
                Some(address) => address,
                None => self.refill(region, size)?,
            };
            Object::from_address(address as usize).expect("a region holds no address 0")
        };
        object.set_word(0, Header::new(ty, flags).bits());
        if shape.element.is_some() {
            object.set_word(LENGTH_OFFSET, count);
        }
        if flags & FLAG_FINALIZE != 0 {
            self.lock().finalizable.push(object.address());
        }
        Ok(object)
    }

    /// Like `alloc`, but collects when a collection is wanted or the heap is full, and retries; `region` is empty after a collection.
    pub fn alloc_collecting(&self, region: &mut Region, roots: &mut dyn Roots, ty: TypeId, count: u64) -> Result<Object, OutOfMemory> {
        if self.config.torture || self.wants_collection() {
            self.collect(roots);
            *region = Region { cursor: 0, limit: 0 };
        }
        match self.alloc(region, ty, count) {
            Ok(object) => Ok(object),
            Err(Full) => {
                self.collect(roots);
                *region = Region { cursor: 0, limit: 0 };
                self.alloc(region, ty, count).map_err(|Full| OutOfMemory { limit: self.config.limit })
            }
        }
    }

    fn refill(&self, region: &mut Region, size: usize) -> Result<u64, Full> {
        let mut inner = self.lock();
        let (start, end) = inner.take_hole(size.div_ceil(LINE_SIZE), self.config.limit)?;
        self.since.fetch_add(end - start, Ordering::Relaxed);
        *region = Region { cursor: start as u64, limit: end as u64 };
        Ok(region.bump(size as u32).expect("a hole holds the object"))
    }

    fn alloc_large(&self, size: usize) -> Result<Object, Full> {
        let mapped = size.next_multiple_of(PAGE_SIZE);
        let mut inner = self.lock();
        if inner.used_bytes() + mapped > self.config.limit {
            inner.sweep_all();
            if inner.used_bytes() + mapped > self.config.limit {
                return Err(Full);
            }
        }
        let memory = sys::map(mapped).map_err(|_| Full)?;
        let address = memory.as_ptr() as usize;
        inner.large.push(Large { address, mapped });
        inner.large_bytes += mapped;
        inner.stats.large_allocated += 1;
        inner.note_peak();
        self.since.fetch_add(mapped, Ordering::Relaxed);
        Ok(Object::from_address(address).expect("a mapping is not at address 0"))
    }

    /// Collects with the roots on this thread; the world is stopped.
    pub fn collect(&self, roots: &mut dyn Roots) -> Collection {
        self.collect_parallel(roots, 1)
    }

    /// Collects with the roots on `workers` threads; the world is stopped.
    pub fn collect_parallel(&self, roots: &mut dyn Roots, workers: usize) -> Collection {
        let mut inner = self.lock();
        let inner = &mut *inner;
        for block in inner.pool.ids() {
            if inner.info[block.index()].state != BlockState::Free {
                inner.pool.marks(block).clear();
            }
        }
        let large = LargeMarks::new(inner.large.iter().map(|large| large.address));
        roots.retire();
        let cycle = Cycle::new(&self.table, &inner.pool, &large, workers);
        mark::run(&cycle, roots, workers);
        let mut found = Collection {
            marked_objects: cycle.objects.load(Ordering::Relaxed),
            marked_bytes: cycle.bytes.load(Ordering::Relaxed),
            ..Collection::default()
        };
        if self.config.verify {
            found.problems = self.check(inner, &large, roots);
            inner.stats.problems += found.problems.len();
        }
        found.finalized = finalize(inner, &large);
        found.large_freed = free_large(inner, &large);
        inner.recyclable.clear();
        inner.unswept.clear();
        for (at, info) in inner.info.iter_mut().enumerate() {
            if info.state != BlockState::Free {
                info.state = BlockState::Unswept;
                inner.unswept.push_back(BlockId::from_index(at));
            }
        }
        inner.stats.collections += 1;
        inner.stats.marked_objects = found.marked_objects;
        inner.stats.marked_bytes = found.marked_bytes;
        inner.stats.finalized += found.finalized;
        inner.stats.large_freed += found.large_freed;
        self.since.store(0, Ordering::Relaxed);
        self.gap.store(self.config.trigger.max(found.marked_bytes), Ordering::Relaxed);
        found
    }

    fn check(&self, inner: &Inner, large: &LargeMarks, roots: &mut dyn Roots) -> Vec<String> {
        let in_use = inner.info.iter().map(|info| info.state != BlockState::Free).collect();
        let checker = Checker::new(in_use);
        let check = Cycle::new(&self.table, &inner.pool, large, 1).checking(&checker);
        mark::run(&check, roots, 1);
        let mut problems = std::mem::take(&mut *checker.problems.lock().expect("the problem list"));
        problems.sort();
        problems
    }

    /// Checks the marks of the last collection against the roots: every reachable object is marked, on marked lines, and no reference dangles.
    pub fn verify(&self, roots: &mut dyn Roots) -> Vec<String> {
        let inner = self.lock();
        let large = LargeMarks::all_marked(inner.large.iter().map(|large| large.address));
        self.check(&inner, &large, roots)
    }

    /// Whether the last collection marked the object.
    pub fn is_marked(&self, object: Object) -> bool {
        let inner = self.lock();
        let address = object.address();
        match inner.pool.block_of(address) {
            Some(block) => inner.pool.marks(block).objects.is_set((address - inner.pool.base(block)) / 8),
            None => inner.large.iter().any(|large| large.address == address),
        }
    }

    /// Finishes the lazy sweep of every block.
    pub fn sweep_all(&self) {
        self.lock().sweep_all();
    }

    /// How many lines of the object's block the last collection marked within the object.
    #[cfg(test)]
    pub(crate) fn marked_lines(&self, object: Object, size: usize) -> usize {
        let inner = self.lock();
        let address = object.address();
        let block = inner.pool.block_of(address).expect("a small object");
        let offset = address - inner.pool.base(block);
        (offset / LINE_SIZE..=(offset + size - 1) / LINE_SIZE).filter(|&line| inner.pool.marks(block).lines.is_marked(line)).count()
    }

    /// Clears the object marks (`objects`) or the line marks (otherwise) of the object's block.
    #[cfg(test)]
    pub(crate) fn corrupt_marks(&self, object: Object, objects: bool) {
        let inner = self.lock();
        let block = inner.pool.block_of(object.address()).expect("a small object");
        if objects {
            inner.pool.marks(block).objects.clear();
        } else {
            inner.pool.marks(block).lines.clear();
        }
    }

    /// Bytes of blocks in use plus large objects.
    pub fn used_bytes(&self) -> usize {
        self.lock().used_bytes()
    }

    pub fn stats(&self) -> Stats {
        let inner = self.lock();
        let mut stats = inner.stats.clone();
        stats.allocations = self.allocations.load(Ordering::Relaxed);
        stats.used_bytes = inner.used_bytes();
        stats.large_bytes = inner.large_bytes;
        stats.pool_bytes = inner.pool.blocks() * BLOCK_SIZE;
        stats
    }

    /// The blocks by state.
    pub fn census(&self) -> Census {
        let inner = self.lock();
        let mut census = Census { large: inner.large.len(), ..Census::default() };
        for info in &inner.info {
            match info.state {
                BlockState::Free => census.free += 1,
                BlockState::Unswept => census.unswept += 1,
                BlockState::Recyclable => census.recyclable += 1,
                BlockState::Used => census.used += 1,
            }
        }
        census
    }
}
