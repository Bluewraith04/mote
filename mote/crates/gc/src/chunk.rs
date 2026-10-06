//! Size-class chunks: a fixed-size block of same-sized object slots, bump-allocated by one
//! mutator and swept in place by the collector. A block's memory belongs to its slab (`slab.rs`)
//! and is re-carved for another size class when the block empties.

use std::cell::UnsafeCell;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::object_model::{GC_FREE_BIT, GC_MARK_BIT, GC_OLD_GEN_BIT, HEADER_SIZE};
use isa::value::ObjectHeader;

/// Block sizes are multiples of this; it is also `size_of::<Value>()` and `HEADER_SIZE`.
pub(crate) const BLOCK_UNIT: usize = 16;
/// The largest block, in units, a chunk serves; bigger objects are `malloc`ed individually.
pub(crate) const MAX_UNITS: usize = 32;
pub const CHUNK_BYTES: usize = 32 * 1024;
/// Slabs are what the OS is asked for; each is cut into this many blocks.
pub(crate) const BLOCKS_PER_SLAB: usize = 32;
pub const SLAB_BYTES: usize = CHUNK_BYTES * BLOCKS_PER_SLAB;

const _: () = assert!(HEADER_SIZE == BLOCK_UNIT && std::mem::size_of::<isa::value::Value>() == BLOCK_UNIT);

/// The block size in units for an object with `slot_count` slots (at least 2, so a free block
/// can hold its list link).
pub(crate) fn units_for(slot_count: usize) -> usize {
    (1 + slot_count).max(2)
}

/// A block of same-sized object slots.
pub struct Chunk {
    base: NonNull<u8>,
    slab: usize,
    units: usize,
    capacity: usize,
    bump: usize,
    free_head: *mut u8,
    live: AtomicUsize,
    owned: AtomicBool,
}

impl Chunk {
    /// A free block over `CHUNK_BYTES` at `base`, cut from slab `slab`.
    pub fn free_block(base: NonNull<u8>, slab: usize) -> Self {
        Chunk {
            base,
            slab,
            units: 0,
            capacity: 0,
            bump: 0,
            free_head: std::ptr::null_mut(),
            live: AtomicUsize::new(0),
            owned: AtomicBool::new(false),
        }
    }

    /// Serves size class `units` from now on, forgetting whatever the block held.
    pub fn assign(&mut self, units: usize) {
        self.units = units;
        self.capacity = CHUNK_BYTES / (units * BLOCK_UNIT);
        self.bump = 0;
        self.free_head = std::ptr::null_mut();
        self.live.store(0, Ordering::Relaxed);
    }

    /// Returns the block to the free state; only valid with no live object in it.
    pub fn release(&mut self) {
        self.units = 0;
        self.capacity = 0;
        self.bump = 0;
        self.free_head = std::ptr::null_mut();
        self.live.store(0, Ordering::Relaxed);
    }

    pub(crate) fn is_free(&self) -> bool {
        self.units == 0
    }

    pub(crate) fn slab(&self) -> usize {
        self.slab
    }

    pub fn units(&self) -> usize {
        self.units
    }

    pub(crate) fn block_size(&self) -> usize {
        self.units * BLOCK_UNIT
    }

    pub(crate) fn has_room(&self) -> bool {
        !self.free_head.is_null() || self.bump < self.capacity
    }

    pub fn live(&self) -> usize {
        self.live.load(Ordering::Relaxed)
    }

    pub(crate) fn set_owned(&self, owned: bool) {
        self.owned.store(owned, Ordering::Relaxed);
    }

    pub(crate) fn is_owned(&self) -> bool {
        self.owned.load(Ordering::Relaxed)
    }

    /// A block for one object, from the free list or the untouched tail.
    pub fn take(&mut self) -> Option<NonNull<u8>> {
        let block = if let Some(free) = NonNull::new(self.free_head) {
            // SAFETY: a free block's first slot holds the next free block.
            self.free_head = unsafe { *(free.as_ptr().add(HEADER_SIZE) as *mut *mut u8) };
            free
        } else if self.bump < self.capacity {
            // SAFETY: `bump < capacity` keeps the block inside the chunk.
            let p = unsafe { self.base.as_ptr().add(self.bump * self.block_size()) };
            self.bump += 1;
            NonNull::new(p)?
        } else {
            return None;
        };
        self.live.store(self.live.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
        Some(block)
    }

    /// Frees every unmarked object, clears the mark on the rest, and returns the bytes freed.
    /// Only with every mutator stopped.
    pub fn sweep(&mut self) -> usize {
        let size = self.block_size();
        let mut freed = 0;
        let mut live = 0;
        for i in 0..self.bump {
            // SAFETY: blocks below `bump` were each initialized with an `ObjectHeader`.
            let header = unsafe { &*(self.base.as_ptr().add(i * size) as *const ObjectHeader) };
            let state = header.gc_state.load(Ordering::Relaxed);
            if state & GC_FREE_BIT != 0 {
                continue;
            }
            if state & GC_MARK_BIT != 0 {
                header.gc_state.store(state & !GC_MARK_BIT | GC_OLD_GEN_BIT, Ordering::Relaxed);
                live += 1;
            } else {
                header.gc_state.store(GC_FREE_BIT, Ordering::Relaxed);
                let block = header as *const ObjectHeader as *mut u8;
                // SAFETY: a block holds at least the header plus one slot.
                unsafe { *(block.add(HEADER_SIZE) as *mut *mut u8) = self.free_head };
                self.free_head = block;
                freed += size;
            }
        }
        self.live.store(live, Ordering::Relaxed);
        freed
    }
}

/// A chunk shared between the collector's registry and its owning mutator.
///
/// Access rule: only the owning mutator touches the chunk while mutators run, and only the
/// collector touches it while every mutator is stopped at a safepoint.
pub struct ChunkCell(UnsafeCell<Chunk>);

// SAFETY: see the access rule above.
unsafe impl Send for ChunkCell {}
unsafe impl Sync for ChunkCell {}

impl ChunkCell {
    pub fn new(chunk: Chunk) -> Self {
        ChunkCell(UnsafeCell::new(chunk))
    }

    /// # Safety
    /// The caller is the owning mutator, or the collector with every mutator stopped.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get(&self) -> &mut Chunk {
        unsafe { &mut *self.0.get() }
    }
}

impl ChunkCell {
    /// Objects allocated here; safe to read while the owner runs (it is an atomic counter).
    pub fn live(&self) -> usize {
        // SAFETY: reads only the atomic field, through a raw pointer, without forming a `&mut Chunk`.
        unsafe { (*self.0.get()).live.load(Ordering::Relaxed) }
    }
}
