use std::alloc::{alloc, dealloc, Layout};
use std::collections::{HashMap, HashSet};
use std::mem::size_of;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use crate::allocator::ObjectAllocator;
use crate::chunk::{units_for, ChunkCell, MAX_UNITS};
use crate::slab::BlockPool;
use crate::object_model::{get_object_size, GC_MARK_BIT, GC_OLD_GEN_BIT, GC_PINNED_BIT, HEADER_SIZE, MIN_ALIGNMENT};
use crate::plan::{GCConfig, PlanType};
use crate::scanning::scan_object;
use contracts::{Heap, HeapStats, Mutator, OutOfMemory, Released, RootSource, ValueSlot, RELEASE_GENERATOR};
use isa::value::{ObjectHeader, TypeDescriptor, Value};

struct ObjectList(HashSet<NonNull<ObjectHeader>>);

// SAFETY: the pointers are heap objects owned by the collector, touched only under the list's mutex.
unsafe impl Send for ObjectList {}

struct MarkStack(Vec<NonNull<ObjectHeader>>);

// SAFETY: the pointers are only dereferenced by the collector during a pause; the stack is empty between collections.
unsafe impl Send for MarkStack {}

const MARK_STACK_KEEP: usize = 4 * 1024 * 1024;

const RELEASE_TRIGGER_MIN: usize = 128;

struct Releasable {
    obj: NonNull<ObjectHeader>,
    type_ptr: usize,
    kind: u8,
    key: i64,
}

// SAFETY: `obj` is only dereferenced by the collector, with every mutator stopped.
unsafe impl Send for Releasable {}

struct Shared {
    releasables: Mutex<Vec<Releasable>>,
    released: Mutex<Vec<Released>>,
    release_trigger: AtomicUsize,
    tracked: Mutex<ObjectList>,
    locals: Mutex<Vec<Arc<Mutex<ObjectList>>>>,
    pool: Mutex<BlockPool>,
    peak_slabs: AtomicUsize,
    available: Mutex<Vec<Vec<Arc<ChunkCell>>>>,
    pending_bytes: AtomicUsize,
    live_bytes: AtomicUsize,
    max_heap: usize,
    pressure: AtomicBool,
    total_allocated_bytes: AtomicUsize,
    total_freed_bytes: AtomicUsize,
    collections_count: AtomicUsize,
    publish_at: usize,
    peak_live_bytes: AtomicUsize,
    peak_in_use_bytes: AtomicUsize,
    large_allocations: AtomicUsize,
    large_bytes: AtomicUsize,
    pause_total_ns: AtomicU64,
    pause_max_ns: AtomicU64,
    size_histogram: Vec<AtomicU64>,
    mark_stack: Mutex<MarkStack>,
}

impl Shared {
    fn check_request(&self, size: usize) {
        if size > self.max_heap {
            let message = format!("out of memory: a {} allocation passes the heap limit of {}", human_bytes(size), human_bytes(self.max_heap));
            std::panic::resume_unwind(Box::new(OutOfMemory(message)));
        }
        let heap = self.live_bytes.load(Ordering::Relaxed).saturating_add(self.pending_bytes.load(Ordering::Relaxed));
        if heap.saturating_add(size) > self.max_heap {
            self.pressure.store(true, Ordering::Relaxed);
        }
    }
}

fn human_bytes(n: usize) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut value = n as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

struct GcMutator {
    shared: Arc<Shared>,
    large: Arc<Mutex<ObjectList>>,
    current: Vec<Option<Arc<ChunkCell>>>,
    unpublished: usize,
    histogram: [u64; MAX_UNITS + 2],
}

impl GcMutator {
    fn new(shared: Arc<Shared>) -> Self {
        let large = Arc::new(Mutex::new(ObjectList(HashSet::new())));
        shared.locals.lock().unwrap().push(large.clone());
        GcMutator { shared, large, current: vec![None; MAX_UNITS + 1], unpublished: 0, histogram: [0; MAX_UNITS + 2] }
    }

    fn publish(&mut self) {
        if self.unpublished > 0 {
            self.shared.pending_bytes.fetch_add(self.unpublished, Ordering::Relaxed);
            self.shared.total_allocated_bytes.fetch_add(self.unpublished, Ordering::Relaxed);
            self.unpublished = 0;
            for (total, n) in self.shared.size_histogram.iter().zip(self.histogram.iter_mut()) {
                if *n > 0 {
                    total.fetch_add(std::mem::take(n), Ordering::Relaxed);
                }
            }
        }
    }

    fn refill(&mut self, units: usize) {
        if let Some(full) = self.current[units].take() {
            // SAFETY: this mutator owns `full`.
            unsafe { full.get() }.set_owned(false);
        }
        let reused = self.shared.available.lock().unwrap()[units].pop();
        let chunk = reused.unwrap_or_else(|| {
            let mut pool = self.shared.pool.lock().unwrap();
            let chunk = pool.take(units);
            self.shared.peak_slabs.fetch_max(pool.slab_count(), Ordering::Relaxed);
            chunk
        });
        // SAFETY: just taken from the shared pool (or created), so no other mutator owns it.
        unsafe { chunk.get() }.set_owned(true);
        self.current[units] = Some(chunk);
    }

    fn alloc_large(&mut self, type_ptr: NonNull<TypeDescriptor>, slot_count: usize) -> NonNull<ObjectHeader> {
        let total_size = HEADER_SIZE.saturating_add(slot_count.saturating_mul(size_of::<Value>()));
        self.shared.check_request(total_size);
        let layout = Layout::from_size_align(total_size, MIN_ALIGNMENT)
            .expect("Invalid layout for object allocation");

        // SAFETY: `layout` has a non-zero size; the result is checked for null below.
        let raw_ptr = unsafe { alloc(layout) };
        if raw_ptr.is_null() {
            panic!("Out of memory: failed to allocate {} bytes", total_size);
        }
        // SAFETY: `raw_ptr` is a fresh allocation of `total_size` bytes, enough for the header and `slot_count` slots.
        let obj = unsafe { init_object(raw_ptr, type_ptr, slot_count) };
        self.large.lock().unwrap().0.insert(obj);
        self.unpublished += total_size;
        self.shared.large_allocations.fetch_add(1, Ordering::Relaxed);
        self.shared.large_bytes.fetch_add(total_size, Ordering::Relaxed);
        self.histogram[MAX_UNITS + 1] += 1;
        if total_size >= 64 * 1024 {
            contracts::mem_trace!("large object {} B", total_size);
        }
        obj
    }
}

/// # Safety
/// `raw` must be writable for an `ObjectHeader` plus `slot_count` slots.
unsafe fn init_object(raw: *mut u8, type_ptr: NonNull<TypeDescriptor>, slot_count: usize) -> NonNull<ObjectHeader> {
    let header_ptr = raw as *mut ObjectHeader;
    unsafe {
        std::ptr::write(header_ptr, ObjectHeader { type_ptr, gc_state: AtomicUsize::new(0) });
        for i in 0..slot_count {
            (*header_ptr).set_field(i, Value::null());
        }
        NonNull::new_unchecked(header_ptr)
    }
}

impl Mutator for GcMutator {
    fn alloc(&mut self, type_ptr: NonNull<TypeDescriptor>, slot_count: usize) -> NonNull<ObjectHeader> {
        let units = units_for(slot_count);
        let obj = if units > MAX_UNITS {
            self.alloc_large(type_ptr, slot_count)
        } else {
            let block = loop {
                if let Some(chunk) = &self.current[units] {
                    // SAFETY: this mutator owns `chunk`.
                    if let Some(block) = unsafe { chunk.get() }.take() {
                        break block;
                    }
                }
                self.refill(units);
            };
            self.unpublished += units * crate::chunk::BLOCK_UNIT;
            self.histogram[units] += 1;
            // SAFETY: the block holds `units` units, at least the header plus `slot_count` slots.
            unsafe { init_object(block.as_ptr(), type_ptr, slot_count) }
        };
        if self.unpublished >= self.shared.publish_at {
            self.publish();
        }
        obj
    }

    fn release_on_collect(&mut self, obj: NonNull<ObjectHeader>, kind: u8, key: i64) {
        // SAFETY: the caller passes a live object.
        let type_ptr = unsafe { obj.as_ref().type_ptr.as_ptr() as usize };
        let mut list = self.shared.releasables.lock().unwrap();
        list.push(Releasable { obj, type_ptr, kind, key });
        if list.len() >= self.shared.release_trigger.load(Ordering::Relaxed) {
            self.shared.pressure.store(true, Ordering::Relaxed);
        }
    }

    fn release(&mut self, obj: NonNull<ObjectHeader>) {
        // SAFETY: the caller passes a live object.
        let state = unsafe { obj.as_ref().gc_state.load(Ordering::SeqCst) };
        let size = get_object_size(obj);
        if size <= MAX_UNITS * crate::chunk::BLOCK_UNIT || state & GC_PINNED_BIT != 0 || isa::seal::is_sealed(obj) {
            return;
        }
        if !self.take_large(obj) {
            return;
        }
        let layout = Layout::from_size_align(size, MIN_ALIGNMENT).expect("Invalid layout for object release");
        // SAFETY: `obj` was allocated by `alloc_large` with this layout, and the caller says nothing points at it.
        unsafe { dealloc(obj.as_ptr() as *mut u8, layout) };
        self.shared.total_freed_bytes.fetch_add(size, Ordering::Relaxed);
        if state & GC_OLD_GEN_BIT != 0 {
            sub_saturating(&self.shared.live_bytes, size);
        } else {
            let from_local = self.unpublished.min(size);
            self.unpublished -= from_local;
            sub_saturating(&self.shared.pending_bytes, size - from_local);
        }
    }
}

fn sub_saturating(counter: &AtomicUsize, n: usize) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| Some(v.saturating_sub(n)));
}

impl GcMutator {
    fn take_large(&self, obj: NonNull<ObjectHeader>) -> bool {
        if self.large.lock().unwrap().0.remove(&obj) {
            return true;
        }
        if self.shared.tracked.lock().unwrap().0.remove(&obj) {
            return true;
        }
        let locals = self.shared.locals.lock().unwrap();
        locals.iter().any(|local| local.lock().unwrap().0.remove(&obj))
    }
}

impl Drop for GcMutator {
    fn drop(&mut self) {
        self.publish();
        for (units, slot) in self.current.iter_mut().enumerate() {
            if let Some(chunk) = slot.take() {
                // SAFETY: this mutator owned `chunk` until now.
                let c = unsafe { chunk.get() };
                c.set_owned(false);
                if c.has_room() {
                    self.shared.available.lock().unwrap()[units].push(chunk);
                }
            }
        }
    }
}

/// GC Controller implementing `NoGC` and `MarkSweep`: a basic full-heap mark/sweep with an
/// in-place old-gen promotion bit.
pub struct GCController {
    pub config: GCConfig,
    shared: Arc<Shared>,
    direct: Mutex<GcMutator>,
}

impl GCController {
    pub fn new(config: GCConfig) -> Self {
        let shared = Arc::new(Shared {
            releasables: Mutex::new(Vec::new()),
            released: Mutex::new(Vec::new()),
            release_trigger: AtomicUsize::new(RELEASE_TRIGGER_MIN),
            tracked: Mutex::new(ObjectList(HashSet::new())),
            locals: Mutex::new(Vec::new()),
            pool: Mutex::new(BlockPool::new()),
            peak_slabs: AtomicUsize::new(0),
            available: Mutex::new(vec![Vec::new(); MAX_UNITS + 1]),
            pending_bytes: AtomicUsize::new(0),
            live_bytes: AtomicUsize::new(0),
            max_heap: config.max_heap_size,
            pressure: AtomicBool::new(false),
            total_allocated_bytes: AtomicUsize::new(0),
            total_freed_bytes: AtomicUsize::new(0),
            collections_count: AtomicUsize::new(0),
            publish_at: (config.collection_threshold / 8).clamp(1, 32 * 1024),
            peak_live_bytes: AtomicUsize::new(0),
            peak_in_use_bytes: AtomicUsize::new(0),
            large_allocations: AtomicUsize::new(0),
            large_bytes: AtomicUsize::new(0),
            pause_total_ns: AtomicU64::new(0),
            pause_max_ns: AtomicU64::new(0),
            size_histogram: (0..=MAX_UNITS + 1).map(|_| AtomicU64::new(0)).collect(),
            mark_stack: Mutex::new(MarkStack(Vec::new())),
        });
        let direct = Mutex::new(GcMutator::new(shared.clone()));
        Self { config, shared, direct }
    }

    pub fn live_objects_count(&self) -> usize {
        let large = self.shared.tracked.lock().unwrap().0.len()
            + self.shared.locals.lock().unwrap().iter().map(|l| l.lock().unwrap().0.len()).sum::<usize>();
        let small: usize = self.shared.pool.lock().unwrap().blocks().map(|c| c.live()).sum();
        large + small
    }

    pub fn collections_count(&self) -> usize {
        self.shared.collections_count.load(Ordering::Relaxed)
    }

    pub fn total_freed_bytes(&self) -> usize {
        self.shared.total_freed_bytes.load(Ordering::Relaxed)
    }

    /// Allocates an object on the GC-managed heap and tracks it. The slot count
    /// comes from the type descriptor's declared fields.
    pub fn alloc_object(&self, type_ptr: NonNull<TypeDescriptor>) -> NonNull<ObjectHeader> {
        // SAFETY: the caller passes a live type descriptor.
        let field_count = unsafe { type_ptr.as_ref().slots as usize };
        self.alloc_object_with_slots(type_ptr, field_count)
    }

    /// Allocates an object with an explicit slot count — needed for heap strings
    /// and any other object whose size is not fixed by its descriptor.
    pub fn alloc_object_with_slots(
        &self,
        type_ptr: NonNull<TypeDescriptor>,
        field_count: usize,
    ) -> NonNull<ObjectHeader> {
        self.direct.lock().unwrap().alloc(type_ptr, field_count)
    }

    fn trigger_bytes(&self) -> usize {
        let live = self.shared.live_bytes.load(Ordering::Relaxed);
        self.config.collection_threshold.max(live.min(self.shared.max_heap.saturating_sub(live)))
    }

    pub fn should_collect(&self) -> bool {
        self.config.plan_type != PlanType::NoGC
            && (self.shared.pending_bytes.load(Ordering::Relaxed) >= self.trigger_bytes() || self.shared.pressure.load(Ordering::Relaxed))
    }

    fn collect_from(&self, roots: &mut dyn RootSource) {
        if self.config.plan_type == PlanType::NoGC {
            return;
        }

        self.shared.collections_count.fetch_add(1, Ordering::Relaxed);
        let started = std::time::Instant::now();
        let in_use = self.shared.live_bytes.load(Ordering::Relaxed) + self.shared.pending_bytes.load(Ordering::Relaxed);
        self.shared.peak_in_use_bytes.fetch_max(in_use, Ordering::Relaxed);

        let mut tracked = self.shared.tracked.lock().unwrap();
        {
            let mut locals = self.shared.locals.lock().unwrap();
            for local in locals.iter() {
                tracked.0.extend(local.lock().unwrap().0.drain());
            }
            locals.retain(|l| Arc::strong_count(l) > 1);
        }

        let mut mark_stack = std::mem::take(&mut self.shared.mark_stack.lock().unwrap().0);
        roots.visit_roots(&mut |slot: ValueSlot| {
            if let Some(obj_ptr) = slot.load_object_ptr() {
                mark_stack.push(obj_ptr);
            }
        });

        let roots_at = started.elapsed();

        let mut marked_objects = 0usize;
        let mut stack_peak = 0usize;
        let mut live_handles: HashMap<(usize, i64), NonNull<ObjectHeader>> = HashMap::new();
        while let Some(obj_ptr) = mark_stack.pop() {
            // SAFETY: the mark stack holds only live objects.
            let gc_state = &unsafe { obj_ptr.as_ref() }.gc_state;
            let before = gc_state.load(Ordering::Relaxed);
            if before & (GC_MARK_BIT | isa::value::REGION_BIT) != 0 {
                continue;
            }
            gc_state.store(before | GC_MARK_BIT, Ordering::Relaxed);
            if contracts::heap::MEM_TRACE {
                marked_objects += 1;
                stack_peak = stack_peak.max(mark_stack.len());
            }
            if before & isa::value::RELEASE_BIT != 0 {
                // SAFETY: a marked object is live; a handle object holds its id in slot 0.
                let (type_ptr, id) = unsafe { (obj_ptr.as_ref().type_ptr.as_ptr() as usize, obj_ptr.as_ref().get_field(0).as_int()) };
                if let Some(id) = id {
                    live_handles.insert((type_ptr, id), obj_ptr);
                }
            }
            scan_object(obj_ptr, &mut |child_slot: ValueSlot| {
                if let Some(child_ptr) = child_slot.load_object_ptr() {
                    mark_stack.push(child_ptr);
                }
            });
        }

        mark_stack.shrink_to(MARK_STACK_KEEP);
        self.shared.mark_stack.lock().unwrap().0 = mark_stack;

        {
            let mut releasables = self.shared.releasables.lock().unwrap();
            let mut released = self.shared.released.lock().unwrap();
            releasables.retain_mut(|e| {
                // SAFETY: the object is still allocated: sweeping has not started.
                if unsafe { e.obj.as_ref().gc_state.load(Ordering::SeqCst) } & GC_MARK_BIT != 0 {
                    return true;
                }
                if e.kind != RELEASE_GENERATOR {
                    if let Some(&copy) = live_handles.get(&(e.type_ptr, e.key)) {
                        e.obj = copy;
                        return true;
                    }
                }
                released.push(Released { kind: e.kind, key: e.key });
                false
            });
            self.shared.release_trigger.store(RELEASE_TRIGGER_MIN.max(releasables.len() * 2), Ordering::Relaxed);
        }

        let marked_at = started.elapsed();

        let mut freed = 0;
        let mut live = 0;
        tracked.0.retain(|&obj_ptr| {
            // SAFETY: every tracked object is live until swept here.
            let gc_state = unsafe { obj_ptr.as_ref().gc_state.load(Ordering::SeqCst) };
            if (gc_state & GC_MARK_BIT) != 0 {
                // SAFETY: as above.
                unsafe {
                    obj_ptr.as_ref().gc_state.store(gc_state & !GC_MARK_BIT | GC_OLD_GEN_BIT, Ordering::Relaxed);
                }
                live += get_object_size(obj_ptr);
                return true;
            }
            let size = get_object_size(obj_ptr);
            let layout = Layout::from_size_align(size, MIN_ALIGNMENT).unwrap();
            // SAFETY: the object was allocated with this size and alignment, and no mutator runs during the sweep.
            unsafe {
                dealloc(obj_ptr.as_ptr() as *mut u8, layout);
            }
            freed += size;
            false
        });

        {
            let mut pool = self.shared.pool.lock().unwrap();
            let mut available = self.shared.available.lock().unwrap();
            available.iter_mut().for_each(Vec::clear);
            let mut emptied = Vec::new();
            for cell in pool.blocks() {
                // SAFETY: every mutator is stopped, so the collector may touch any chunk.
                let chunk = unsafe { cell.get() };
                if chunk.is_free() {
                    continue;
                }
                freed += chunk.sweep();
                live += chunk.live() * chunk.block_size();
                if chunk.is_owned() {
                    continue;
                }
                if chunk.live() == 0 {
                    emptied.push(cell.clone());
                } else if chunk.has_room() {
                    available[chunk.units()].push(cell.clone());
                }
            }
            for cell in &emptied {
                pool.give_back(cell);
            }
            pool.decay(live);
        }

        self.shared.total_freed_bytes.fetch_add(freed, Ordering::Relaxed);
        self.shared.live_bytes.store(live, Ordering::Relaxed);
        self.shared.pending_bytes.store(0, Ordering::Relaxed);
        self.shared.pressure.store(false, Ordering::Relaxed);

        self.shared.peak_live_bytes.fetch_max(live, Ordering::Relaxed);
        let pause = started.elapsed().as_nanos() as u64;
        self.shared.pause_total_ns.fetch_add(pause, Ordering::Relaxed);
        self.shared.pause_max_ns.fetch_max(pause, Ordering::Relaxed);
        contracts::mem_trace!(
            "gc #{} {:.2} ms (roots {:.2}, mark {:.2} for {} objects, stack peak {}, sweep {:.2}): in use {} B, live {} B, freed {} B",
            self.shared.collections_count.load(Ordering::Relaxed),
            pause as f64 / 1e6,
            roots_at.as_secs_f64() * 1e3,
            (marked_at - roots_at).as_secs_f64() * 1e3,
            marked_objects,
            stack_peak,
            (started.elapsed() - marked_at).as_secs_f64() * 1e3,
            in_use,
            live,
            freed
        );
    }
}

impl ObjectAllocator for GCController {
    fn alloc_object(&mut self, type_ptr: NonNull<TypeDescriptor>) -> NonNull<ObjectHeader> {
        GCController::alloc_object(self, type_ptr)
    }

    fn allocated_bytes(&self) -> usize {
        self.direct.lock().unwrap().publish();
        self.shared.total_allocated_bytes.load(Ordering::Relaxed)
    }
}

impl Heap for GCController {
    fn mutator(&self) -> Box<dyn Mutator> {
        Box::new(GcMutator::new(self.shared.clone()))
    }

    fn should_collect(&self) -> bool {
        GCController::should_collect(self)
    }

    fn collect(&self, roots: &mut dyn RootSource) {
        self.collect_from(roots)
    }

    fn take_released(&self) -> Vec<Released> {
        std::mem::take(&mut *self.shared.released.lock().unwrap())
    }

    fn limit_exceeded(&self) -> Option<String> {
        let live = self.shared.live_bytes.load(Ordering::Relaxed);
        (live > self.shared.max_heap)
            .then(|| format!("out of memory: heap limit {} reached (live {})", human_bytes(self.shared.max_heap), human_bytes(live)))
    }

    fn stats(&self) -> HeapStats {
        self.direct.lock().unwrap().publish();
        let (slabs, blocks, free_blocks) = {
            let pool = self.shared.pool.lock().unwrap();
            (pool.slab_count(), pool.used_blocks(), pool.free_blocks())
        };
        HeapStats {
            live_objects: self.live_objects_count(),
            collections: self.collections_count(),
            bytes_allocated: self.shared.total_allocated_bytes.load(Ordering::Relaxed),
            bytes_freed: self.total_freed_bytes(),
            live_bytes: self.shared.live_bytes.load(Ordering::Relaxed),
            peak_live_bytes: self.shared.peak_live_bytes.load(Ordering::Relaxed),
            peak_in_use_bytes: self.shared.peak_in_use_bytes.load(Ordering::Relaxed),
            chunks: blocks,
            free_blocks,
            slabs,
            peak_slabs: self.shared.peak_slabs.load(Ordering::Relaxed),
            large_allocations: self.shared.large_allocations.load(Ordering::Relaxed),
            large_bytes: self.shared.large_bytes.load(Ordering::Relaxed),
            pause_total_ns: self.shared.pause_total_ns.load(Ordering::Relaxed),
            pause_max_ns: self.shared.pause_max_ns.load(Ordering::Relaxed),
        }
    }

    fn size_histogram(&self) -> Vec<u64> {
        self.direct.lock().unwrap().publish();
        self.shared.size_histogram.iter().map(|n| n.load(Ordering::Relaxed)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Roots(Vec<Value>);

    impl RootSource for Roots {
        fn visit_roots(&mut self, visitor: &mut dyn FnMut(ValueSlot)) {
            for v in self.0.iter_mut() {
                let slot = ValueSlot::new(v);
                if slot.is_boxed_object() {
                    visitor(slot);
                }
            }
        }
    }

    #[test]
    fn test_collection_triggering_and_reclamation() {
        let mut type_node = TypeDescriptor::new(
            1,
            vec![Some("val".into()), Some("next".into())],
        );
        let type_ptr = NonNull::new(&mut type_node).unwrap();

        let config = GCConfig::mark_sweep().with_threshold(1024);
        let gc = GCController::new(config);

        let mut vm = Roots(vec![Value::null(); 8]);

        let root_obj = gc.alloc_object(type_ptr);
        vm.0[0] = Value::boxed(root_obj);

        for i in 0..50 {
            let dead_obj = gc.alloc_object(type_ptr);
            unsafe {
                let header = &mut *dead_obj.as_ptr();
                header.set_field(0, Value::small_int(i));
            }
        }

        assert_eq!(gc.live_objects_count(), 51);
        assert!(gc.should_collect());

        gc.collect(&mut vm);

        assert_eq!(gc.collections_count(), 1);
        assert_eq!(gc.live_objects_count(), 1);
        assert!(gc.total_freed_bytes() > 0);

        unsafe {
            let header = root_obj.as_ref();
            assert_eq!(header.type_ptr, type_ptr);
            assert_eq!(header.gc_state.load(Ordering::SeqCst) & GC_OLD_GEN_BIT, GC_OLD_GEN_BIT);
        }
    }

    #[test]
    fn test_generational_promotion_and_chain_retention() {
        let mut type_node = TypeDescriptor::with_pointer_mask(
            1,
            vec![Some("next".into())],
            0b1,
        );
        let type_ptr = NonNull::new(&mut type_node).unwrap();

        let config = GCConfig::mark_sweep().with_threshold(256);
        let gc = GCController::new(config);

        let mut vm = Roots(vec![Value::null(); 8]);

        let obj3 = gc.alloc_object(type_ptr);
        let obj2 = gc.alloc_object(type_ptr);
        let obj1 = gc.alloc_object(type_ptr);

        unsafe {
            (*obj1.as_ptr()).set_field(0, Value::boxed(obj2));
            (*obj2.as_ptr()).set_field(0, Value::boxed(obj3));
        }

        vm.0[1] = Value::boxed(obj1);

        for _ in 0..20 {
            gc.alloc_object(type_ptr);
        }

        gc.collect(&mut vm);

        assert_eq!(gc.live_objects_count(), 3);
        unsafe {
            assert_eq!(obj1.as_ref().gc_state.load(Ordering::SeqCst) & GC_OLD_GEN_BIT, GC_OLD_GEN_BIT);
            assert_eq!(obj2.as_ref().gc_state.load(Ordering::SeqCst) & GC_OLD_GEN_BIT, GC_OLD_GEN_BIT);
            assert_eq!(obj3.as_ref().gc_state.load(Ordering::SeqCst) & GC_OLD_GEN_BIT, GC_OLD_GEN_BIT);
        }
    }

    #[test]
    fn test_reclamation_loop_bounded_memory() {
        let mut type_node = TypeDescriptor::new(
            1,
            vec![Some("data".into())],
        );
        let type_ptr = NonNull::new(&mut type_node).unwrap();

        let config = GCConfig::mark_sweep().with_threshold(512);
        let gc = GCController::new(config);

        let mut vm = Roots(vec![Value::null(); 8]);

        for _ in 0..1000 {
            gc.alloc_object(type_ptr);
            if gc.should_collect() {
                gc.collect(&mut vm);
            }
        }

        gc.collect(&mut vm);

        assert_eq!(gc.live_objects_count(), 0);
        assert!(gc.collections_count() >= 10);
    }

    #[test]
    fn test_mutators_on_many_threads_allocate_without_sharing_a_lock_and_all_get_collected() {
        let mut type_node = TypeDescriptor::new(1, vec![Some("val".into())]);
        let type_addr = &mut type_node as *mut TypeDescriptor as usize;
        let gc = Arc::new(GCController::new(GCConfig::mark_sweep().with_threshold(1 << 30)));

        let kept: Vec<usize> = (0..4)
            .map(|_| {
                let gc = gc.clone();
                std::thread::spawn(move || {
                    let type_ptr = NonNull::new(type_addr as *mut TypeDescriptor).unwrap();
                    let mut m = gc.mutator();
                    let first = m.alloc(type_ptr, 1);
                    for _ in 0..999 {
                        m.alloc(type_ptr, 1);
                    }
                    first.as_ptr() as usize
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect();

        assert_eq!(gc.stats().live_objects, 4000);

        let mut vm = Roots(kept.iter().map(|&a| Value::boxed(NonNull::new(a as *mut ObjectHeader).unwrap())).collect());
        gc.collect(&mut vm);

        assert_eq!(gc.stats().live_objects, 4, "only each thread's kept object survives");
        assert_eq!(gc.collections_count(), 1);
    }

    #[test]
    fn test_freed_blocks_are_reused_so_the_heap_stays_bounded() {
        let mut type_node = TypeDescriptor::new(1, vec![Some("a".into()), Some("b".into())]);
        let type_ptr = NonNull::new(&mut type_node).unwrap();
        let gc = GCController::new(GCConfig::mark_sweep().with_threshold(1 << 30));
        let mut vm = Roots(vec![Value::null(); 2]);

        for _ in 0..100 {
            for _ in 0..1000 {
                gc.alloc_object(type_ptr);
            }
            gc.collect(&mut vm);
        }

        assert_eq!(gc.live_objects_count(), 0);
        assert!(gc.shared.pool.lock().unwrap().used_blocks() <= 3, "100,000 objects fit in a few reused chunks");
    }

    #[test]
    fn test_large_objects_are_freed_individually() {
        let mut type_node = TypeDescriptor::new(1, vec![Some("a".into()); 100]);
        let type_ptr = NonNull::new(&mut type_node).unwrap();
        let gc = GCController::new(GCConfig::mark_sweep().with_threshold(1 << 30));
        let mut vm = Roots(vec![Value::null(); 2]);

        let kept = gc.alloc_object_with_slots(type_ptr, 100);
        vm.0[0] = Value::boxed(kept);
        gc.alloc_object_with_slots(type_ptr, 100);
        assert_eq!(gc.live_objects_count(), 2);

        gc.collect(&mut vm);

        assert_eq!(gc.live_objects_count(), 1);
        assert!(gc.total_freed_bytes() >= HEADER_SIZE + 100 * size_of::<Value>());
    }

    #[test]
    fn test_a_mutator_that_goes_away_hands_its_chunk_to_the_next() {
        let mut type_node = TypeDescriptor::new(1, vec![Some("a".into())]);
        let type_ptr = NonNull::new(&mut type_node).unwrap();
        let gc = GCController::new(GCConfig::mark_sweep().with_threshold(1 << 30));

        let mut first = gc.mutator();
        first.alloc(type_ptr, 1);
        drop(first);
        let mut second = gc.mutator();
        second.alloc(type_ptr, 1);

        assert_eq!(gc.shared.pool.lock().unwrap().used_blocks(), 1);
        assert_eq!(gc.live_objects_count(), 2);
    }

    #[test]
    fn test_live_bytes_counts_small_and_large_survivors() {
        let mut type_node = TypeDescriptor::new(1, vec![Some("a".into()); 100]);
        let type_ptr = NonNull::new(&mut type_node).unwrap();
        let gc = GCController::new(GCConfig::mark_sweep().with_threshold(1 << 30));
        let mut vm = Roots(vec![Value::null(); 3]);

        vm.0[0] = Value::boxed(gc.alloc_object_with_slots(type_ptr, 1));
        vm.0[1] = Value::boxed(gc.alloc_object_with_slots(type_ptr, 100));
        gc.alloc_object_with_slots(type_ptr, 1);
        gc.collect(&mut vm);

        let two_slot_block = 2 * crate::chunk::BLOCK_UNIT;
        assert_eq!(gc.stats().live_bytes, two_slot_block + HEADER_SIZE + 100 * size_of::<Value>());
    }

    #[test]
    fn test_release_frees_a_large_object_now_and_leaves_small_ones_to_the_collector() {
        let mut type_node = TypeDescriptor::new(1, vec![Some("a".into()); 100]);
        let type_ptr = NonNull::new(&mut type_node).unwrap();
        let gc = GCController::new(GCConfig::mark_sweep().with_threshold(1 << 30));
        let mut m = gc.mutator();

        let large = m.alloc(type_ptr, 100);
        let small = m.alloc(type_ptr, 1);
        assert_eq!(gc.live_objects_count(), 2);

        m.release(small);
        assert_eq!(gc.live_objects_count(), 2, "a small object stays until the next collection");
        m.release(large);
        assert_eq!(gc.live_objects_count(), 1);
        assert!(gc.total_freed_bytes() >= HEADER_SIZE + 100 * size_of::<Value>());
    }

    #[test]
    fn test_release_finds_a_survivor_and_a_neighbours_object() {
        let mut type_node = TypeDescriptor::new(1, vec![Some("a".into()); 100]);
        let type_ptr = NonNull::new(&mut type_node).unwrap();
        let gc = GCController::new(GCConfig::mark_sweep().with_threshold(1 << 30));
        let mut first = gc.mutator();
        let mut second = gc.mutator();

        let survivor = first.alloc(type_ptr, 100);
        let neighbours = first.alloc(type_ptr, 100);
        let mut vm = Roots(vec![Value::boxed(survivor), Value::boxed(neighbours)]);
        gc.collect(&mut vm);
        let fresh = first.alloc(type_ptr, 100);
        assert_eq!(gc.live_objects_count(), 3);

        second.release(survivor);
        second.release(fresh);
        assert_eq!(gc.live_objects_count(), 1);
        assert_eq!(gc.stats().live_bytes, HEADER_SIZE + 100 * size_of::<Value>(), "the neighbour's object stays counted");
    }

    #[test]
    fn test_a_request_past_the_limit_unwinds_with_out_of_memory() {
        let mut type_node = TypeDescriptor::new(1, vec![Some("a".into())]);
        let type_ptr = NonNull::new(&mut type_node).unwrap();
        let gc = GCController::new(GCConfig::mark_sweep().with_max_heap(1 << 20));

        let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| gc.alloc_object_with_slots(type_ptr, 1 << 20))).unwrap_err();
        let oom = payload.downcast_ref::<OutOfMemory>().expect("an OutOfMemory payload");
        assert_eq!(oom.0, "out of memory: a 16.0 MiB allocation passes the heap limit of 1.0 MiB");
    }

    #[test]
    fn test_a_large_request_that_may_not_fit_asks_for_a_collection() {
        let mut type_node = TypeDescriptor::new(1, vec![Some("a".into())]);
        let type_ptr = NonNull::new(&mut type_node).unwrap();
        let gc = GCController::new(GCConfig::mark_sweep().with_threshold(1 << 30).with_max_heap(1 << 20));

        gc.alloc_object_with_slots(type_ptr, 40_000);
        gc.stats();
        assert!(!gc.should_collect());
        gc.alloc_object_with_slots(type_ptr, 40_000);
        gc.stats();
        assert!(gc.should_collect(), "two 640 KB objects may not fit under 1 MiB");
    }

    #[test]
    fn test_a_live_heap_past_the_limit_is_reported_after_a_collection() {
        let mut type_node = TypeDescriptor::new(1, vec![Some("a".into())]);
        let type_ptr = NonNull::new(&mut type_node).unwrap();
        let gc = GCController::new(GCConfig::mark_sweep().with_max_heap(4096));
        let mut vm = Roots((0..200).map(|_| Value::boxed(gc.alloc_object(type_ptr))).collect());
        assert_eq!(gc.limit_exceeded(), None, "nothing is measured before a collection");

        gc.collect(&mut vm);

        let message = gc.limit_exceeded().expect("6.3 KiB live against a 4 KiB limit");
        assert_eq!(message, "out of memory: heap limit 4.0 KiB reached (live 6.2 KiB)");
    }

    #[test]
    fn test_the_trigger_leaves_room_under_the_limit() {
        let mut type_node = TypeDescriptor::new(1, vec![Some("a".into())]);
        let type_ptr = NonNull::new(&mut type_node).unwrap();
        let gc = GCController::new(GCConfig::mark_sweep().with_threshold(1024).with_max_heap(100 * 1024));
        let mut vm = Roots((0..2048).map(|_| Value::boxed(gc.alloc_object(type_ptr))).collect());
        gc.collect(&mut vm);
        assert_eq!(gc.stats().live_bytes, 64 * 1024);

        for _ in 0..640 {
            gc.alloc_object(type_ptr);
        }
        gc.stats();
        assert!(!gc.should_collect(), "20 KiB allocated; 36 KiB of room is left");
        for _ in 0..640 {
            gc.alloc_object(type_ptr);
        }
        gc.stats();
        assert!(gc.should_collect(), "40 KiB allocated against 36 KiB of room");
    }

    #[test]
    fn test_the_trigger_grows_with_the_live_heap() {
        let mut type_node = TypeDescriptor::new(1, vec![Some("a".into())]);
        let type_ptr = NonNull::new(&mut type_node).unwrap();
        let gc = GCController::new(GCConfig::mark_sweep().with_threshold(1024));
        let mut vm = Roots((0..128).map(|_| Value::boxed(gc.alloc_object(type_ptr))).collect());
        gc.collect(&mut vm);
        assert_eq!(gc.stats().live_bytes, 128 * 32);

        for _ in 0..64 {
            gc.alloc_object(type_ptr);
        }
        gc.stats();
        assert!(!gc.should_collect(), "2 KiB allocated against 4 KiB live: below the live size");

        for _ in 0..64 {
            gc.alloc_object(type_ptr);
        }
        gc.stats();
        assert!(gc.should_collect(), "4 KiB allocated against 4 KiB live");
    }

    #[test]
    fn test_an_update_working_copy_is_swept_like_any_live_object() {
        let mut type_node = TypeDescriptor::new(1, vec![Some("a".into())]);
        let type_ptr = NonNull::new(&mut type_node).unwrap();
        let gc = GCController::new(GCConfig::mark_sweep().with_threshold(1 << 30));
        let mut vm = Roots(vec![Value::null(); 2]);

        let obj = gc.alloc_object(type_ptr);
        unsafe { obj.as_ref() }.gc_state.fetch_or(isa::seal::WORKING_BIT, Ordering::SeqCst);
        vm.0[0] = Value::boxed(obj);
        for _ in 0..2 {
            gc.collect(&mut vm);
            assert_eq!(gc.stats().live_bytes, 2 * crate::chunk::BLOCK_UNIT, "counted live");
            let state = unsafe { obj.as_ref() }.gc_state.load(Ordering::SeqCst);
            assert_eq!(state & GC_MARK_BIT, 0, "its mark is cleared for the next cycle");
        }
    }

    fn handle(gc: &GCController, m: &mut dyn Mutator, type_ptr: NonNull<TypeDescriptor>, kind: u8, id: i64) -> NonNull<ObjectHeader> {
        let mut obj = m.alloc(type_ptr, 1);
        unsafe {
            obj.as_mut().set_field(0, Value::small_int(id));
            obj.as_ref().gc_state.fetch_or(isa::value::RELEASE_BIT, Ordering::SeqCst);
        }
        m.release_on_collect(obj, kind, id);
        let _ = gc;
        obj
    }

    #[test]
    fn test_an_unreachable_handle_is_reported_once_and_a_live_one_is_not() {
        let mut type_node = TypeDescriptor::new(1, vec![Some("id".into())]);
        let type_ptr = NonNull::new(&mut type_node).unwrap();
        let gc = GCController::new(GCConfig::mark_sweep().with_threshold(1 << 30));
        let mut m = gc.mutator();
        let live = handle(&gc, &mut *m, type_ptr, contracts::RELEASE_FILE, 1);
        handle(&gc, &mut *m, type_ptr, contracts::RELEASE_SOCKET, 2);
        let mut vm = Roots(vec![Value::boxed(live)]);

        gc.collect(&mut vm);
        assert_eq!(gc.take_released(), vec![Released { kind: contracts::RELEASE_SOCKET, key: 2 }]);
        gc.collect(&mut vm);
        assert!(gc.take_released().is_empty(), "each resource is reported once");
        vm.0.clear();
        gc.collect(&mut vm);
        assert_eq!(gc.take_released(), vec![Released { kind: contracts::RELEASE_FILE, key: 1 }]);
    }

    #[test]
    fn test_a_live_copy_of_a_handle_keeps_it_open_until_the_last_copy_dies() {
        let mut type_node = TypeDescriptor::new(1, vec![Some("id".into())]);
        let type_ptr = NonNull::new(&mut type_node).unwrap();
        let gc = GCController::new(GCConfig::mark_sweep().with_threshold(1 << 30));
        let mut m = gc.mutator();
        let original = handle(&gc, &mut *m, type_ptr, contracts::RELEASE_FILE, 9);
        let mut copy = m.alloc(type_ptr, 1);
        unsafe {
            copy.as_mut().set_field(0, Value::small_int(9));
            copy.as_ref().gc_state.fetch_or(isa::value::RELEASE_BIT, Ordering::SeqCst);
        }
        let mut vm = Roots(vec![Value::boxed(copy)]);
        let _ = original;

        gc.collect(&mut vm);
        assert!(gc.take_released().is_empty(), "the copy still holds the handle");
        gc.collect(&mut vm);
        assert!(gc.take_released().is_empty(), "the copy is now the tracked owner");
        vm.0.clear();
        gc.collect(&mut vm);
        assert_eq!(gc.take_released(), vec![Released { kind: contracts::RELEASE_FILE, key: 9 }]);
    }

    #[test]
    fn test_registered_handles_request_a_collection() {
        let mut type_node = TypeDescriptor::new(1, vec![Some("id".into())]);
        let type_ptr = NonNull::new(&mut type_node).unwrap();
        let gc = GCController::new(GCConfig::mark_sweep().with_threshold(1 << 30));
        let mut m = gc.mutator();
        for id in 0..RELEASE_TRIGGER_MIN as i64 - 1 {
            handle(&gc, &mut *m, type_ptr, contracts::RELEASE_FILE, id);
        }
        assert!(!gc.should_collect());
        handle(&gc, &mut *m, type_ptr, contracts::RELEASE_FILE, 1000);
        assert!(gc.should_collect());
    }

    #[test]
    fn test_a_region_object_is_neither_marked_nor_freed() {
        let mut type_node = TypeDescriptor::new(1, vec![Some("a".into())]);
        let type_ptr = NonNull::new(&mut type_node).unwrap();
        let gc = GCController::new(GCConfig::mark_sweep().with_threshold(1 << 30));
        let mut vm = Roots(vec![Value::null(); 2]);

        let mut region_obj = ObjectHeader { type_ptr, gc_state: AtomicUsize::new(isa::value::region_state(1)) };
        let child = gc.alloc_object(type_ptr);
        vm.0[0] = Value::boxed(NonNull::from(&mut region_obj));
        vm.0[1] = Value::boxed(child);
        gc.collect(&mut vm);

        assert_eq!(region_obj.gc_state.load(Ordering::SeqCst), isa::value::region_state(1), "untouched by the collector");
        assert_eq!(gc.live_objects_count(), 1, "the heap child survives through its own root");
    }

    fn fill_blocks(gc: &GCController, type_ptr: NonNull<TypeDescriptor>, slots: usize, blocks: usize) {
        let per_block = crate::chunk::CHUNK_BYTES / (units_for(slots) * crate::chunk::BLOCK_UNIT);
        let mut m = gc.mutator();
        for _ in 0..per_block * blocks {
            m.alloc(type_ptr, slots);
        }
    }

    #[test]
    fn test_an_emptied_block_serves_another_size_class() {
        let mut type_node = TypeDescriptor::new(1, vec![Some("a".into())]);
        let type_ptr = NonNull::new(&mut type_node).unwrap();
        let gc = GCController::new(GCConfig::mark_sweep().with_threshold(1 << 30));
        let mut vm = Roots(vec![Value::null(); 2]);

        fill_blocks(&gc, type_ptr, 1, 16);
        assert_eq!(gc.shared.pool.lock().unwrap().used_blocks(), 16);
        gc.collect(&mut vm);
        assert_eq!(gc.shared.pool.lock().unwrap().used_blocks(), 0, "nothing lived, so every block is free");

        fill_blocks(&gc, type_ptr, 7, 16);
        let pool = gc.shared.pool.lock().unwrap();
        assert_eq!(pool.slab_count(), 1, "the 8-unit class reused the 2-unit class's blocks");
        assert_eq!(pool.used_blocks(), 16);
    }

    #[test]
    fn test_a_free_slab_is_released_after_two_collections() {
        let mut type_node = TypeDescriptor::new(1, vec![Some("a".into())]);
        let type_ptr = NonNull::new(&mut type_node).unwrap();
        let gc = GCController::new(GCConfig::mark_sweep().with_threshold(1 << 30));
        let mut vm = Roots(vec![Value::null(); 2]);

        fill_blocks(&gc, type_ptr, 1, 16);
        gc.collect(&mut vm);
        assert_eq!(gc.shared.pool.lock().unwrap().slab_count(), 1, "one collection free: kept");
        gc.collect(&mut vm);
        assert_eq!(gc.shared.pool.lock().unwrap().slab_count(), 0, "two collections free: released");
        assert_eq!(gc.shared.pool.lock().unwrap().free_blocks(), 0);

        fill_blocks(&gc, type_ptr, 1, 16);
        assert_eq!(gc.shared.pool.lock().unwrap().slab_count(), 1, "allocation maps a new slab");
    }

    #[test]
    fn test_a_slab_in_use_again_is_not_released() {
        let mut type_node = TypeDescriptor::new(1, vec![Some("a".into())]);
        let type_ptr = NonNull::new(&mut type_node).unwrap();
        let gc = GCController::new(GCConfig::mark_sweep().with_threshold(1 << 30));
        let mut vm = Roots(vec![Value::null(); 2]);

        fill_blocks(&gc, type_ptr, 1, 16);
        gc.collect(&mut vm);
        fill_blocks(&gc, type_ptr, 1, 4);
        gc.collect(&mut vm);
        gc.collect(&mut vm);
        assert_eq!(gc.shared.pool.lock().unwrap().slab_count(), 0, "idle only counts collections in a row");

        fill_blocks(&gc, type_ptr, 1, 16);
        gc.collect(&mut vm);
        fill_blocks(&gc, type_ptr, 1, 1);
        gc.collect(&mut vm);
        assert_eq!(gc.shared.pool.lock().unwrap().slab_count(), 1, "a block taken in between resets the count");
    }

    #[test]
    fn test_free_slabs_past_the_cap_are_released_at_once() {
        let mut type_node = TypeDescriptor::new(1, vec![Some("a".into())]);
        let type_ptr = NonNull::new(&mut type_node).unwrap();
        let gc = GCController::new(GCConfig::mark_sweep().with_threshold(1 << 30));
        let mut vm = Roots(vec![Value::null(); 2]);

        fill_blocks(&gc, type_ptr, 1, 256);
        assert_eq!(gc.shared.pool.lock().unwrap().slab_count(), 8);
        gc.collect(&mut vm);
        assert_eq!(gc.shared.pool.lock().unwrap().slab_count(), 4, "released down to the 4 MiB cap after one collection");
        gc.collect(&mut vm);
        assert_eq!(gc.shared.pool.lock().unwrap().slab_count(), 0);
    }
}
