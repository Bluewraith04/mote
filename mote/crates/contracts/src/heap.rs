use crate::roots::RootSource;
use isa::value::{ObjectHeader, TypeDescriptor};
use std::ptr::NonNull;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
/// Collector statistics.
pub struct HeapStats {
    pub live_objects: usize,
    pub collections: usize,
    pub bytes_allocated: usize,
    pub bytes_freed: usize,
    /// Bytes that survived the last collection.
    pub live_bytes: usize,
    /// The most live bytes any collection found.
    pub peak_live_bytes: usize,
    /// The most bytes in use (live at the last collection plus allocated since) when a collection began.
    pub peak_in_use_bytes: usize,
    /// Blocks serving a size class.
    pub chunks: usize,
    /// Blocks in the pool, free for any size class.
    pub free_blocks: usize,
    /// Slabs held from the OS now, and at most.
    pub slabs: usize,
    pub peak_slabs: usize,
    /// Objects allocated outside the size classes, and their bytes.
    pub large_allocations: usize,
    pub large_bytes: usize,
    /// Time spent inside collections.
    pub pause_total_ns: u64,
    pub pause_max_ns: u64,
}

/// Whether [`mem_trace!`] prints; set by the `mem-trace` cargo feature.
pub const MEM_TRACE: bool = cfg!(feature = "mem-trace");

#[macro_export]
macro_rules! mem_trace {
    ($($arg:tt)*) => {
        if $crate::heap::MEM_TRACE {
            eprintln!("[mem] {}", format_args!($($arg)*));
        }
    };
}

/// The panic payload a [`Mutator`] unwinds with when one allocation passes the heap limit; the runtime turns it into a task fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutOfMemory(pub String);

/// What a released object owned: a file, a socket or a library handle (the key is its id), or a generator's regions
/// (the key names the stack the runtime holds for it).
pub const RELEASE_FILE: u8 = 1;
pub const RELEASE_SOCKET: u8 = 2;
pub const RELEASE_LIBRARY: u8 = 3;
pub const RELEASE_GENERATOR: u8 = 4;

/// A resource whose owning object the collector found unreachable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Released {
    pub kind: u8,
    pub key: i64,
}

/// Where objects live. Shared by every worker; allocation goes through a per-worker [`Mutator`].
pub trait Heap: Send + Sync {
    /// A new allocation handle for one worker thread.
    fn mutator(&self) -> Box<dyn Mutator>;

    fn should_collect(&self) -> bool;

    /// Called only with every mutator parked at a safepoint.
    fn collect(&self, roots: &mut dyn RootSource);

    /// The fault message when the live heap is past its limit (checked after a collection), else `None`.
    fn limit_exceeded(&self) -> Option<String> {
        None
    }

    fn stats(&self) -> HeapStats {
        HeapStats::default()
    }

    /// Objects allocated so far by block size in 16-byte units (index 2 to 32), with the last index counting large objects.
    fn size_histogram(&self) -> Vec<u64> {
        Vec::new()
    }

    /// The resources whose objects the last collections found unreachable, once each.
    fn take_released(&self) -> Vec<Released> {
        Vec::new()
    }
}

/// One worker's allocation handle; no lock shared with other workers on the fast path.
pub trait Mutator: Send {
    fn alloc(&mut self, type_ptr: NonNull<TypeDescriptor>, slot_count: usize) -> NonNull<ObjectHeader>;

    /// Frees `obj` now instead of at the next collection. The caller guarantees nothing else points at it.
    /// A heap may keep it until the collector runs.
    fn release(&mut self, _obj: NonNull<ObjectHeader>) {}

    /// Reports `key` of kind `RELEASE_*` once `obj` (and, for a handle, every copy of it) is unreachable.
    /// A handle object's `key` is its slot 0 and it carries [`isa::value::RELEASE_BIT`].
    fn release_on_collect(&mut self, _obj: NonNull<ObjectHeader>, _kind: u8, _key: i64) {}
}
