//! Region memory: a per-task stack of segments with a mark per open region.

use std::alloc::{alloc, dealloc, Layout};
use std::mem::size_of;
use std::ptr::NonNull;
use std::sync::Mutex;

use isa::value::{region_state, ObjectHeader, TypeDescriptor, Value, MAX_REGION_DEPTH, MAX_REGION_SLOTS};

const SEGMENT_SIZES: [usize; 3] = [4 * 1024, 16 * 1024, 64 * 1024];

const POOL_BYTES_PER_SIZE: usize = 256 * 1024;

const POOL_IDLE_COLLECTIONS: u32 = 2;

const OBJECT_ALIGN: usize = 16;

fn size_class(index: usize) -> usize {
    index.min(SEGMENT_SIZES.len() - 1)
}

fn segment_layout(class: usize) -> Layout {
    Layout::from_size_align(SEGMENT_SIZES[class], OBJECT_ALIGN).expect("segment layout")
}

struct Segment {
    ptr: NonNull<u8>,
    class: usize,
    used: usize,
}

// SAFETY: a segment is plain memory owned by one stack (or the pool) at a time.
unsafe impl Send for Segment {}

impl Segment {
    fn capacity(&self) -> usize {
        SEGMENT_SIZES[self.class]
    }
}

struct SegmentPool {
    free: [Vec<(NonNull<u8>, u32)>; SEGMENT_SIZES.len()],
}

// SAFETY: the pointers are unowned memory, touched only under the pool's mutex.
unsafe impl Send for SegmentPool {}

static POOL: Mutex<SegmentPool> = Mutex::new(SegmentPool { free: [Vec::new(), Vec::new(), Vec::new()] });

fn take_segment(class: usize) -> Segment {
    let pooled = POOL.lock().unwrap().free[class].pop();
    let ptr = match pooled {
        Some((ptr, _)) => ptr,
        None => {
            // SAFETY: the layout has a nonzero size.
            let raw = unsafe { alloc(segment_layout(class)) };
            NonNull::new(raw).unwrap_or_else(|| panic!("Out of memory: failed to allocate a {}-byte region segment", SEGMENT_SIZES[class]))
        }
    };
    Segment { ptr, class, used: 0 }
}

fn give_segment(segment: Segment) {
    let mut pool = POOL.lock().unwrap();
    let kept = &mut pool.free[segment.class];
    if (kept.len() + 1) * SEGMENT_SIZES[segment.class] <= POOL_BYTES_PER_SIZE {
        kept.push((segment.ptr, 0));
        return;
    }
    drop(pool);
    // SAFETY: allocated in `take_segment` with this class's layout, and no object in it is reachable any more.
    unsafe { dealloc(segment.ptr.as_ptr(), segment_layout(segment.class)) };
}

/// Ages every pooled segment by one collection and frees those idle for [`POOL_IDLE_COLLECTIONS`].
pub(crate) fn decay_segments() {
    let mut pool = POOL.lock().unwrap();
    for (class, kept) in pool.free.iter_mut().enumerate() {
        for entry in kept.iter_mut() {
            entry.1 += 1;
        }
        kept.retain(|&(ptr, idle)| {
            if idle < POOL_IDLE_COLLECTIONS {
                return true;
            }
            // SAFETY: allocated in `take_segment` with this class's layout; nothing uses a pooled segment.
            unsafe { dealloc(ptr.as_ptr(), segment_layout(class)) };
            false
        });
    }
}

/// Bytes of free segments in the pool, for tests and `--mem-stats`.
#[cfg(test)]
pub(crate) fn pooled_segment_bytes() -> usize {
    let pool = POOL.lock().unwrap();
    pool.free.iter().enumerate().map(|(class, kept)| kept.len() * SEGMENT_SIZES[class]).sum()
}

/// Upper bounds of the fill buckets in [`RegionStats`], in bytes; the last bucket is everything above.
pub const FILL_BUCKETS: [usize; 5] = [256, 1024, 4096, 16 * 1024, 64 * 1024];

/// Upper bounds of the object-size buckets in [`RegionStats`], in slots; the last bucket is everything above.
pub const OBJECT_BUCKETS: [usize; 6] = [2, 4, 8, 16, 32, 63];

/// What regions did, for `--mem-stats`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegionStats {
    pub scopes_entered: u64,
    pub objects: u64,
    /// Bytes in use summed over every region at its exit, and the most any one region held.
    pub bytes: u64,
    pub peak_fill: usize,
    pub peak_depth: usize,
    /// The most segment bytes one task held at once.
    pub peak_segment_bytes: usize,
    /// Allocations placed on the heap because regions were nested past the depth an object records.
    pub too_deep: u64,
    /// Regions by their fill at exit, per [`FILL_BUCKETS`].
    pub fill_histogram: [u64; FILL_BUCKETS.len() + 1],
    /// Objects by slot count, per [`OBJECT_BUCKETS`].
    pub object_histogram: [u64; OBJECT_BUCKETS.len() + 1],
}

impl RegionStats {
    pub fn merge(&mut self, other: &RegionStats) {
        self.scopes_entered += other.scopes_entered;
        self.objects += other.objects;
        self.bytes += other.bytes;
        self.peak_fill = self.peak_fill.max(other.peak_fill);
        self.peak_depth = self.peak_depth.max(other.peak_depth);
        self.peak_segment_bytes = self.peak_segment_bytes.max(other.peak_segment_bytes);
        self.too_deep += other.too_deep;
        for (a, b) in self.fill_histogram.iter_mut().zip(other.fill_histogram) {
            *a += b;
        }
        for (a, b) in self.object_histogram.iter_mut().zip(other.object_histogram) {
            *a += b;
        }
    }
}

fn bucket(bounds: &[usize], value: usize) -> usize {
    bounds.iter().position(|&b| value <= b).unwrap_or(bounds.len())
}

#[derive(Clone, Copy)]
struct Mark {
    segments: usize,
    offset: usize,
    total: usize,
}

/// A task's regions: segments filled front to back, and a mark per open region. Owns no memory until the first allocation.
#[derive(Default)]
pub struct ArenaStack {
    segments: Vec<Segment>,
    marks: Vec<Mark>,
    spare: Option<Segment>,
    used_total: usize,
    segment_bytes: usize,
    stats: RegionStats,
}

impl ArenaStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocates an object in the innermost region, initialising its header and null fields.
    pub fn alloc_object(&mut self, type_desc: NonNull<TypeDescriptor>) -> Result<NonNull<ObjectHeader>, String> {
        if self.marks.is_empty() {
            return Err("No active arena scope: call ENTERARENA first".to_string());
        }
        if self.too_deep() {
            return Err(format!("A region holds objects only {MAX_REGION_DEPTH} levels deep"));
        }
        // SAFETY: the caller passes a live type descriptor.
        let field_count = unsafe { type_desc.as_ref().slots as usize };
        if field_count > MAX_REGION_SLOTS {
            return Err(format!("A region holds objects of at most {MAX_REGION_SLOTS} slots, not {field_count}"));
        }
        let bytes = size_of::<ObjectHeader>() + field_count * size_of::<Value>();
        self.stats.objects += 1;
        self.stats.object_histogram[bucket(&OBJECT_BUCKETS, field_count)] += 1;
        let fits = self.segments.last().is_some_and(|s| s.used + bytes <= s.capacity());
        if !fits {
            self.grow();
        }
        let segment = self.segments.last_mut().expect("grow added a segment");
        // SAFETY: the bound check above keeps `bytes` inside the segment, and `used` stays a multiple of 16.
        let obj_ptr = unsafe { segment.ptr.as_ptr().add(segment.used) as *mut ObjectHeader };
        segment.used += bytes;
        self.used_total += bytes;
        // SAFETY: `obj_ptr` points at `bytes` writable bytes reserved above.
        unsafe {
            (*obj_ptr).type_ptr = type_desc;
            (*obj_ptr).gc_state = std::sync::atomic::AtomicUsize::new(region_state(self.marks.len()));
            for f in 0..field_count {
                (*obj_ptr).set_field(f, Value::null());
            }
            Ok(NonNull::new_unchecked(obj_ptr))
        }
    }

    /// Whether the innermost region is deeper than an object's depth can record; the caller allocates on the heap.
    pub fn too_deep(&self) -> bool {
        self.marks.len() > MAX_REGION_DEPTH
    }

    /// Records one allocation that went to the heap because regions were nested too deep.
    pub(crate) fn count_too_deep(&mut self) {
        self.stats.too_deep += 1;
    }

    fn grow(&mut self) {
        let class = size_class(self.segments.len());
        let segment = match self.spare.take() {
            Some(spare) if spare.class == class => spare,
            other => {
                if let Some(spare) = other {
                    give_segment(spare);
                }
                take_segment(class)
            }
        };
        self.segment_bytes += segment.capacity();
        self.stats.peak_segment_bytes = self.stats.peak_segment_bytes.max(self.segment_bytes);
        self.segments.push(segment);
    }

    /// The measurements so far, leaving the counters at zero.
    pub(crate) fn take_stats(&mut self) -> RegionStats {
        std::mem::take(&mut self.stats)
    }

    /// Adds another stack's measurements to this one's.
    pub(crate) fn absorb_stats(&mut self, other: &RegionStats) {
        self.stats.merge(other);
    }

    /// Opens a region; returns the depth. Takes no memory until an object is allocated.
    pub fn enter_arena(&mut self) -> usize {
        let offset = self.segments.last().map_or(0, |s| s.used);
        self.marks.push(Mark { segments: self.segments.len(), offset, total: self.used_total });
        self.stats.scopes_entered += 1;
        self.stats.peak_depth = self.stats.peak_depth.max(self.marks.len());
        self.marks.len()
    }

    /// Closes the innermost region: everything allocated since its mark is freed at once.
    pub fn exit_arena(&mut self) -> Result<(), String> {
        let Some(mark) = self.marks.pop() else {
            return Err("Cannot exit arena: arena stack is empty".to_string());
        };
        let fill = self.used_total - mark.total;
        self.stats.bytes += fill as u64;
        self.stats.peak_fill = self.stats.peak_fill.max(fill);
        self.stats.fill_histogram[bucket(&FILL_BUCKETS, fill)] += 1;
        contracts::mem_trace!("region closed with {} B", fill);
        self.rewind(mark);
        Ok(())
    }

    fn rewind(&mut self, mark: Mark) {
        while self.segments.len() > mark.segments {
            let segment = self.segments.pop().expect("length checked");
            self.segment_bytes -= segment.capacity();
            self.release(segment);
        }
        if let Some(last) = self.segments.last_mut() {
            poison(last, mark.offset);
            last.used = mark.offset;
        }
        self.used_total = mark.total;
    }

    fn release(&mut self, mut segment: Segment) {
        poison(&mut segment, 0);
        segment.used = 0;
        match self.spare {
            None => self.spare = Some(segment),
            Some(_) => give_segment(segment),
        }
    }

    /// Bytes of segments this task holds; they count toward its stack limit.
    pub(crate) fn segment_bytes(&self) -> usize {
        self.segment_bytes
    }

    /// Calls `f` for each live region object, oldest first: each segment is walked object by object up to its fill.
    pub fn for_each_object(&self, mut f: impl FnMut(NonNull<ObjectHeader>)) {
        for segment in &self.segments {
            let mut at = 0;
            while at < segment.used {
                // SAFETY: `at` is the start of an object written by `alloc_object`, below the segment's fill.
                let header = unsafe { NonNull::new_unchecked(segment.ptr.as_ptr().add(at) as *mut ObjectHeader) };
                let fields = unsafe { header.as_ref().type_ptr.as_ref().slots as usize };
                f(header);
                at += size_of::<ObjectHeader>() + fields * size_of::<Value>();
            }
        }
    }

    /// Calls `f` for each heap object a live region object points to.
    pub(crate) fn for_each_heap_pointer(&self, mut f: impl FnMut(NonNull<ObjectHeader>)) {
        self.for_each_object(|object| {
            // SAFETY: `for_each_object` yields initialised objects, and `i` stays below their slot count.
            let header = unsafe { object.as_ref() };
            let slots = unsafe { header.type_ptr.as_ref().slots as usize };
            for i in 0..slots {
                if let Some(ptr) = unsafe { header.get_field(i) }.as_object_ptr() {
                    f(ptr);
                }
            }
        });
    }

    pub fn depth(&self) -> usize {
        self.marks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    /// Empties the stack when the scheduler recycles a `TaskContext`: every segment goes back to the pool.
    pub fn clear(&mut self) {
        self.marks.clear();
        for segment in self.segments.drain(..) {
            give_segment(segment);
        }
        if let Some(spare) = self.spare.take() {
            give_segment(spare);
        }
        self.used_total = 0;
        self.segment_bytes = 0;
    }
}

impl Drop for ArenaStack {
    fn drop(&mut self) {
        self.clear();
    }
}

/// The region stacks of suspended generators, by key: a generator keeps its regions across a `yield`.
#[derive(Default)]
pub struct GeneratorRegions {
    stacks: Mutex<std::collections::HashMap<i64, Box<ArenaStack>>>,
    next_key: std::sync::atomic::AtomicI64,
}

impl GeneratorRegions {
    /// A key no other generator has used.
    pub(crate) fn new_key(&self) -> i64 {
        self.next_key.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
    }

    /// Takes the stack of the generator being resumed.
    pub fn take(&self, key: i64) -> Option<Box<ArenaStack>> {
        self.stacks.lock().unwrap().remove(&key)
    }

    /// Keeps the stack of a generator that yielded.
    pub fn put(&self, key: i64, stack: Box<ArenaStack>) {
        self.stacks.lock().unwrap().insert(key, stack);
    }

    /// Drops the stack of a generator that finished or was collected.
    pub fn release(&self, key: i64) {
        self.stacks.lock().unwrap().remove(&key);
    }

    /// The heap objects live region objects of suspended generators point to.
    pub(crate) fn for_each_heap_pointer(&self, mut f: impl FnMut(NonNull<ObjectHeader>)) {
        for stack in self.stacks.lock().unwrap().values() {
            stack.for_each_heap_pointer(&mut f);
        }
    }

    /// Suspended generators holding regions.
    pub fn len(&self) -> usize {
        self.stacks.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn poison(segment: &mut Segment, from: usize) {
    if !cfg!(debug_assertions) {
        return;
    }
    let mut at = from;
    while at < segment.used {
        // SAFETY: `at` is the start of an object written by `alloc_object`, below the segment's fill, and its descriptor is alive.
        unsafe {
            let header = segment.ptr.as_ptr().add(at) as *mut ObjectHeader;
            let fields = (*header).type_ptr.as_ref().slots as usize;
            let body = at + size_of::<ObjectHeader>();
            std::ptr::write_bytes(segment.ptr.as_ptr().add(body), 0xDD, fields * size_of::<Value>());
            at = body + fields * size_of::<Value>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_slots() -> Box<TypeDescriptor> {
        Box::new(TypeDescriptor::new(1, vec![Some("a".into()), Some("b".into())]))
    }

    #[test]
    fn test_allocation_and_bulk_free() {
        let mut stack = ArenaStack::new();
        let mut desc = two_slots();
        let ty = NonNull::new(&mut *desc).unwrap();
        stack.enter_arena();
        let a = stack.alloc_object(ty).unwrap();
        let b = stack.alloc_object(ty).unwrap();
        assert_ne!(a.as_ptr(), b.as_ptr());
        unsafe {
            (*a.as_ptr()).set_field(0, Value::small_int(100));
            assert_eq!((*a.as_ptr()).get_field(0).as_int(), Some(100));
        }
        assert_eq!(stack.used_total, 96);
        stack.exit_arena().unwrap();
        assert_eq!(stack.used_total, 0);
    }

    #[test]
    fn test_nesting_and_the_empty_stack() {
        let mut stack = ArenaStack::new();
        assert_eq!(stack.depth(), 0);
        stack.enter_arena();
        stack.enter_arena();
        assert_eq!(stack.depth(), 2);
        assert!(stack.exit_arena().is_ok());
        assert!(stack.exit_arena().is_ok());
        assert!(stack.exit_arena().is_err());
    }

    #[test]
    fn test_a_region_owns_no_memory_until_its_first_object() {
        let mut stack = ArenaStack::new();
        stack.enter_arena();
        assert_eq!(stack.segment_bytes(), 0);
        let mut desc = two_slots();
        stack.alloc_object(NonNull::new(&mut *desc).unwrap()).unwrap();
        assert_eq!(stack.segment_bytes(), 4096);
    }

    #[test]
    fn test_allocating_without_a_region_is_an_error() {
        let mut stack = ArenaStack::new();
        let mut desc = two_slots();
        assert!(stack.alloc_object(NonNull::new(&mut *desc).unwrap()).is_err());
    }

    #[test]
    fn test_segments_grow_4_then_16_then_64_kib_and_overflow_never_panics() {
        let mut stack = ArenaStack::new();
        let mut desc = Box::new(TypeDescriptor::new(1, (0..63).map(|i| Some(format!("f{i}"))).collect()));
        let ty = NonNull::new(&mut *desc).unwrap();
        stack.enter_arena();
        for _ in 0..(4 + 16 + 64 + 1) {
            stack.alloc_object(ty).unwrap();
        }
        assert_eq!(stack.segment_bytes(), 4096 + 16 * 1024 + 2 * 64 * 1024);
        assert_eq!(stack.segments.iter().map(|s| s.capacity()).collect::<Vec<_>>(), vec![4096, 16384, 65536, 65536]);
    }

    #[test]
    fn test_an_object_past_the_slot_limit_is_refused() {
        let mut stack = ArenaStack::new();
        let mut desc = Box::new(TypeDescriptor::new(1, (0..64).map(|i| Some(format!("f{i}"))).collect()));
        stack.enter_arena();
        assert!(stack.alloc_object(NonNull::new(&mut *desc).unwrap()).is_err());
    }

    #[test]
    fn test_rewind_frees_segments_above_the_mark_and_reuses_memory() {
        let mut stack = ArenaStack::new();
        let mut desc = two_slots();
        let ty = NonNull::new(&mut *desc).unwrap();
        stack.enter_arena();
        let outer = stack.alloc_object(ty).unwrap();
        stack.enter_arena();
        for _ in 0..200 {
            stack.alloc_object(ty).unwrap();
        }
        assert!(stack.segments.len() > 1);
        stack.exit_arena().unwrap();
        assert_eq!((stack.segments.len(), stack.used_total), (1, 48));
        assert_eq!(stack.segment_bytes(), 4096);
        let next = stack.alloc_object(ty).unwrap();
        assert_eq!(next.as_ptr() as usize, outer.as_ptr() as usize + 48, "the next object follows the outer one");
    }

    #[test]
    fn test_for_each_object_walks_every_segment_up_to_its_fill() {
        let mut stack = ArenaStack::new();
        let mut desc = two_slots();
        let ty = NonNull::new(&mut *desc).unwrap();
        stack.enter_arena();
        let mut made = Vec::new();
        for _ in 0..200 {
            made.push(stack.alloc_object(ty).unwrap());
        }
        let mut seen = Vec::new();
        stack.for_each_object(|o| seen.push(o));
        assert_eq!(seen, made);
        stack.exit_arena().unwrap();
        let mut count = 0;
        stack.for_each_object(|_| count += 1);
        assert_eq!(count, 0);
    }

    #[test]
    fn test_freed_bytes_are_poisoned_in_debug_builds() {
        let mut stack = ArenaStack::new();
        let mut desc = two_slots();
        let ty = NonNull::new(&mut *desc).unwrap();
        stack.enter_arena();
        let obj = stack.alloc_object(ty).unwrap();
        stack.exit_arena().unwrap();
        if cfg!(debug_assertions) {
            assert_eq!(unsafe { *((obj.as_ptr() as *const u8).add(size_of::<ObjectHeader>())) }, 0xDD);
        }
    }

    #[test]
    fn test_clear_returns_everything_and_decay_frees_idle_segments() {
        let mut desc = two_slots();
        let ty = NonNull::new(&mut *desc).unwrap();
        let mut stack = ArenaStack::new();
        stack.enter_arena();
        for _ in 0..200 {
            stack.alloc_object(ty).unwrap();
        }
        stack.clear();
        assert_eq!((stack.depth(), stack.segment_bytes()), (0, 0));
        assert!(pooled_segment_bytes() > 0);
        decay_segments();
        decay_segments();
        assert!(pooled_segment_bytes() <= 3 * POOL_BYTES_PER_SIZE);
    }

    #[test]
    fn test_suspended_generator_stacks_report_their_heap_pointers_until_released() {
        let mut desc = two_slots();
        let ty = NonNull::new(&mut *desc).unwrap();
        let mut heap_object = ObjectHeader { type_ptr: ty, gc_state: 0.into() };
        let heap_ptr = NonNull::new(&mut heap_object).unwrap();
        let mut stack = Box::new(ArenaStack::new());
        stack.enter_arena();
        let obj = stack.alloc_object(ty).unwrap();
        unsafe {
            (*obj.as_ptr()).set_field(0, Value::boxed(heap_ptr));
            (*obj.as_ptr()).set_field(1, Value::small_int(5));
        }
        let regions = GeneratorRegions::default();
        let key = regions.new_key();
        regions.put(key, stack);
        let mut seen = Vec::new();
        regions.for_each_heap_pointer(|p| seen.push(p));
        assert_eq!(seen, vec![heap_ptr]);
        regions.release(key);
        seen.clear();
        regions.for_each_heap_pointer(|p| seen.push(p));
        assert!(seen.is_empty() && regions.is_empty());
    }
}
