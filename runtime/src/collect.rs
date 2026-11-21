//! Collection at a safepoint: the roots are the globals and the frames of each mutator's stack.

use std::collections::BTreeMap;

use mote_abi::{FrameDescriptor, Region, Slot, TaskContext};
use mote_heap::{Marker, Roots};
use mote_scheduler::TaskHandle;

/// Where a mutator waits: the frame pointer and the return address of its call into the runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Top {
    pub fp: usize,
    pub pc: usize,
}

/// A place in static data that holds references, by address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobalRoot {
    pub address: usize,
    pub slots: Vec<Slot>,
}

/// What the collector knows of compiled code: the descriptor of each call site by its return address, and the static places that hold references.
#[derive(Debug, Default)]
pub struct CodeMap {
    frames: BTreeMap<usize, FrameDescriptor>,
    roots: Vec<GlobalRoot>,
    functions: BTreeMap<usize, FunctionInfo>,
}

/// A compiled function as a backtrace names it.
#[derive(Debug)]
pub struct FunctionInfo {
    pub name: String,
    pub size: usize,
    /// The offset from the function's start where a position begins, ascending, and what it stands for.
    pub rows: Vec<(u32, u32)>,
}

/// What a return address lies in: the function's name and the position of the call, or `NONE` when it has none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Located {
    pub function: String,
    pub position: u32,
}

impl Located {
    /// The position of a call that no row covers.
    pub const NONE: u32 = u32::MAX;
}

impl CodeMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Names the function that starts at the address.
    pub fn add_function(&mut self, start: usize, info: FunctionInfo) {
        self.functions.insert(start, info);
    }

    /// The function a return address lies in, with the position of the row at or before the call.
    pub fn locate(&self, return_address: usize) -> Option<Located> {
        let call = return_address.checked_sub(1)?;
        let (&start, info) = self.functions.range(..=call).next_back()?;
        let offset = call - start;
        if offset >= info.size {
            return None;
        }
        let row = info.rows.partition_point(|(from, _)| *from as usize <= offset);
        let position = row.checked_sub(1).map_or(Located::NONE, |row| info.rows[row].1);
        Some(Located { function: info.name.clone(), position })
    }

    /// Describes the frame of a function that is suspended in a call whose return address this is.
    pub fn add_frame(&mut self, return_address: usize, frame: FrameDescriptor) {
        self.frames.insert(return_address, frame);
    }

    pub fn add_root(&mut self, root: GlobalRoot) {
        self.roots.push(root);
    }

    pub fn frame(&self, return_address: usize) -> Option<&FrameDescriptor> {
        self.frames.get(&return_address)
    }

    pub fn frames(&self) -> usize {
        self.frames.len()
    }

    /// Every descriptor, by return address.
    pub fn descriptors(&self) -> impl Iterator<Item = (usize, &FrameDescriptor)> {
        self.frames.iter().map(|(&address, frame)| (address, frame))
    }

    pub fn roots(&self) -> &[GlobalRoot] {
        &self.roots
    }
}

/// Where the words of a stack are read: an address `a` of the stack is read at `a + delta`, and only inside `low..high`.
#[derive(Clone, Copy)]
struct View {
    low: usize,
    high: usize,
    delta: isize,
}

impl View {
    /// The stack as it lies in memory.
    const LIVE: View = View { low: 0, high: usize::MAX, delta: 0 };

    fn holds(self, address: usize, bytes: usize) -> bool {
        address >= self.low && address.checked_add(bytes).is_some_and(|end| end <= self.high)
    }

    fn at(self, address: usize) -> usize {
        address.wrapping_add_signed(self.delta)
    }
}

/// Marks what the frames of a stack hold, from the one suspended at `top` to the first return address that is not in the map.
///
/// # Safety
/// `top` is the state of a mutator parked in a call from code in the map, whose frames each keep a frame record, and the stack is readable through `view`.
unsafe fn walk(code: &CodeMap, top: Top, view: View, marker: &mut Marker<'_>) {
    let (mut fp, mut pc) = (top.fp, top.pc);
    while let Some(frame) = code.frame(pc) {
        let base = fp - frame.frame_size as usize;
        if !view.holds(base, frame.frame_size as usize + 16) {
            break;
        }
        marker.mark_frame(view.at(base), frame);
        // SAFETY: a frame of the map has its caller's frame pointer at `fp` and the return address after it, both inside the view.
        let (next_fp, next_pc) = unsafe { (*(view.at(fp) as *const usize), *(view.at(fp + 8) as *const usize)) };
        if next_fp <= fp {
            break;
        }
        (fp, pc) = (next_fp, next_pc);
    }
}

/// The roots of one collection: the globals, the objects the task table holds, and the stacks and regions of the mutators it found.
pub(crate) struct Visit<'a> {
    pub code: Option<&'a CodeMap>,
    /// Each stack by its top; a task blocked in the runtime comes with its scheduler handle, which says whether a compactor holds its bytes.
    pub stacks: Vec<(Top, Option<TaskHandle>)>,
    pub objects: Vec<usize>,
    pub contexts: Vec<usize>,
}

impl Roots for Visit<'_> {
    fn visit(&mut self, marker: &mut Marker<'_>) {
        for &address in &self.objects {
            marker.mark_address(address as u64);
        }
        let Some(code) = self.code else { return };
        for root in code.roots() {
            marker.mark_value(root.address, &root.slots);
        }
        for (top, task) in &self.stacks {
            let top = *top;
            let Some(task) = task else {
                // SAFETY: the mutator is parked at a poll, with its stack in memory.
                unsafe { walk(code, top, View::LIVE, marker) };
                continue;
            };
            task.with_snapshot(|copied| match copied {
                // SAFETY: the task is blocked in the runtime and nothing copies its stack in or out meanwhile.
                None => unsafe { walk(code, top, View::LIVE, marker) },
                Some((sp, bytes)) => {
                    let image: Vec<usize> = bytes.as_chunks::<8>().0.iter().map(|word| usize::from_ne_bytes(*word)).collect();
                    let view = View { low: sp, high: sp + image.len() * 8, delta: (image.as_ptr() as usize as isize).wrapping_sub(sp as isize) };
                    // SAFETY: the image holds the bytes the stack had when its pages were released, and the view keeps the walk inside it.
                    unsafe { walk(code, top, view, marker) };
                }
            });
        }
    }

    fn retire(&mut self) {
        for &address in &self.contexts {
            // SAFETY: each context belongs to a joined mutator, which is parked.
            unsafe { (*(address as *mut TaskContext)).set_region(Region { cursor: 0, limit: 0 }) };
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mote_abi::{Kind, Scalar, TypeId, TypeTable};
    use mote_heap::{Config, Heap, Object};

    use super::*;

    struct Types {
        table: Arc<TypeTable>,
        node: TypeId,
    }

    fn types() -> Types {
        let mut table = TypeTable::new();
        let int = table.add("Int", Kind::Scalar(Scalar::I64));
        let node = table.add("Node", Kind::Class(vec![int, int]));
        Types { table: Arc::new(table), node }
    }

    fn node(heap: &Heap, region: &mut Region, ty: &Types) -> Object {
        heap.alloc(region, ty.node, 0).expect("room in the heap")
    }

    #[test]
    fn collect_a_walk_marks_the_slots_of_each_frame_in_the_map_and_stops_at_the_first_return_address_that_is_not() {
        let ty = types();
        let heap = Heap::new(ty.table.clone(), Config::default());
        let mut region = Region { cursor: 0, limit: 0 };
        let (inner, outer, beyond, dead) = (node(&heap, &mut region, &ty), node(&heap, &mut region, &ty), node(&heap, &mut region, &ty), node(&heap, &mut region, &ty));
        // Two frames of four words, then a record the walk must not reach: [slot, slot, saved fp, return address].
        let mut stack = [0usize; 12];
        let base = stack.as_mut_ptr() as usize;
        stack[0] = inner.address();
        stack[2] = base + 8 * 6;
        stack[3] = 0x2000;
        stack[4] = outer.address();
        stack[6] = base + 8 * 10;
        stack[7] = 0x3000;
        stack[8] = beyond.address();
        let mut code = CodeMap::new();
        code.add_frame(0x1000, FrameDescriptor::new(16, vec![0], vec![]).unwrap());
        code.add_frame(0x2000, FrameDescriptor::new(16, vec![0], vec![]).unwrap());
        let top = Top { fp: base + 8 * 2, pc: 0x1000 };
        std::hint::black_box(&stack);
        let mut visit = Visit { code: Some(&code), stacks: vec![(top, None)], objects: Vec::new(), contexts: Vec::new() };
        let found = heap.collect(&mut visit);
        assert_eq!(found.marked_objects, 2);
        assert!(heap.is_marked(inner) && heap.is_marked(outer) && !heap.is_marked(beyond) && !heap.is_marked(dead));
    }

    #[test]
    fn collect_a_global_root_is_marked_through_its_slots() {
        let ty = types();
        let heap = Heap::new(ty.table.clone(), Config::default());
        let mut region = Region { cursor: 0, limit: 0 };
        let (kept, dead) = (node(&heap, &mut region, &ty), node(&heap, &mut region, &ty));
        let cell = [0usize, kept.address()];
        let mut code = CodeMap::new();
        code.add_root(GlobalRoot { address: cell.as_ptr() as usize, slots: vec![Slot::Ref(8)] });
        let mut visit = Visit { code: Some(&code), stacks: Vec::new(), objects: Vec::new(), contexts: Vec::new() };
        heap.collect(&mut visit);
        assert!(heap.is_marked(kept) && !heap.is_marked(dead));
    }

    #[test]
    fn collect_the_regions_of_the_mutators_are_emptied() {
        let ty = types();
        let heap = Heap::new(ty.table.clone(), Config::default());
        let mut context = TaskContext::default();
        let mut region = context.region();
        node(&heap, &mut region, &ty);
        context.set_region(region);
        assert_ne!(context.region(), Region { cursor: 0, limit: 0 });
        let mut visit = Visit { code: None, stacks: Vec::new(), objects: Vec::new(), contexts: vec![&mut context as *mut TaskContext as usize] };
        heap.collect(&mut visit);
        assert_eq!(context.region(), Region { cursor: 0, limit: 0 });
    }
}
