//! The block pool: 1 MiB slabs from the OS, cut into 32 KiB blocks that any size class can reuse.

use std::alloc::{alloc, dealloc, Layout};
use std::ptr::NonNull;
use std::sync::Arc;

use crate::chunk::{Chunk, ChunkCell, BLOCKS_PER_SLAB, CHUNK_BYTES, SLAB_BYTES};

const SLAB_ALIGN: usize = 4096;
const IDLE_COLLECTIONS: u32 = 2;
const MIN_FREE_BYTES: usize = 4 * 1024 * 1024;

struct Slab {
    base: NonNull<u8>,
    blocks: Vec<Arc<ChunkCell>>,
    free: usize,
    idle: u32,
}

/// Every slab, and the blocks free for any size class.
pub struct BlockPool {
    slabs: Vec<Option<Slab>>,
    free: Vec<Arc<ChunkCell>>,
}

// SAFETY: the raw slab pointers are owned by the pool and touched only under the pool's mutex.
unsafe impl Send for BlockPool {}

impl BlockPool {
    pub fn new() -> Self {
        BlockPool { slabs: Vec::new(), free: Vec::new() }
    }

    /// A block serving size class `units`, from the free pool or a new slab.
    pub fn take(&mut self, units: usize) -> Arc<ChunkCell> {
        if self.free.is_empty() {
            self.add_slab();
        }
        let cell = self.free.pop().expect("a slab was just added");
        // SAFETY: a pooled block belongs to no mutator.
        let chunk = unsafe { cell.get() };
        let slab = self.slabs[chunk.slab()].as_mut().expect("a pooled block's slab exists");
        slab.free -= 1;
        slab.idle = 0;
        chunk.assign(units);
        cell
    }

    fn add_slab(&mut self) {
        let layout = Layout::from_size_align(SLAB_BYTES, SLAB_ALIGN).expect("slab layout");
        // SAFETY: the layout has a nonzero size.
        let raw = unsafe { alloc(layout) };
        let base = NonNull::new(raw).unwrap_or_else(|| panic!("Out of memory: failed to allocate a {SLAB_BYTES}-byte slab"));
        let id = self.slabs.iter().position(Option::is_none).unwrap_or_else(|| {
            self.slabs.push(None);
            self.slabs.len() - 1
        });
        let blocks: Vec<Arc<ChunkCell>> = (0..BLOCKS_PER_SLAB)
            .map(|i| {
                // SAFETY: block `i` lies inside the slab.
                let at = unsafe { NonNull::new_unchecked(base.as_ptr().add(i * CHUNK_BYTES)) };
                Arc::new(ChunkCell::new(Chunk::free_block(at, id)))
            })
            .collect();
        self.free.extend(blocks.iter().rev().cloned());
        self.slabs[id] = Some(Slab { base, blocks, free: BLOCKS_PER_SLAB, idle: 0 });
    }

    /// Takes a swept block with no live object back into the pool.
    pub(crate) fn give_back(&mut self, cell: &Arc<ChunkCell>) {
        // SAFETY: called with every mutator stopped, on an unowned block.
        let chunk = unsafe { cell.get() };
        chunk.release();
        self.slabs[chunk.slab()].as_mut().expect("a block's slab exists").free += 1;
        self.free.push(cell.clone());
    }

    /// Every in-use block, for the collector's sweep.
    pub fn blocks(&self) -> impl Iterator<Item = &Arc<ChunkCell>> {
        self.slabs.iter().flatten().flat_map(|s| s.blocks.iter())
    }

    /// Ends a collection: ages fully free slabs and releases those idle for two collections,
    /// then more, oldest first, while the free pool holds more than `max(4 MiB, live / 4)`. Returns slabs released.
    pub(crate) fn decay(&mut self, live_bytes: usize) -> usize {
        for slab in self.slabs.iter_mut().flatten() {
            if slab.free == BLOCKS_PER_SLAB {
                slab.idle += 1;
            } else {
                slab.idle = 0;
            }
        }
        let cap = MIN_FREE_BYTES.max(live_bytes / 4);
        let mut free_bytes = self.free.len() * CHUNK_BYTES;
        let mut candidates: Vec<(u32, usize)> =
            self.slabs.iter().enumerate().filter_map(|(id, s)| s.as_ref().filter(|s| s.idle > 0).map(|s| (s.idle, id))).collect();
        candidates.sort_by(|a, b| b.cmp(a));
        let mut released = Vec::new();
        for (idle, id) in candidates {
            if idle >= IDLE_COLLECTIONS || free_bytes > cap {
                free_bytes -= SLAB_BYTES;
                released.push(id);
            }
        }
        if released.is_empty() {
            return 0;
        }
        self.free.retain(|c| !released.contains(&{
            // SAFETY: called with every mutator stopped.
            unsafe { c.get() }.slab()
        }));
        for id in &released {
            let slab = self.slabs[*id].take().expect("a candidate slab exists");
            let layout = Layout::from_size_align(SLAB_BYTES, SLAB_ALIGN).expect("slab layout");
            // SAFETY: allocated in `add_slab` with this layout; every block of it is free and unowned.
            unsafe { dealloc(slab.base.as_ptr(), layout) };
        }
        released.len()
    }

    pub(crate) fn slab_count(&self) -> usize {
        self.slabs.iter().flatten().count()
    }

    pub fn free_blocks(&self) -> usize {
        self.free.len()
    }

    /// Blocks serving a size class.
    pub(crate) fn used_blocks(&self) -> usize {
        self.slab_count() * BLOCKS_PER_SLAB - self.free.len()
    }
}

impl Default for BlockPool {
    fn default() -> Self {
        Self::new()
    }
}
