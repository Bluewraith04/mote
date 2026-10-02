//! The one root enumerator: walks everything live and reports it through `RootSource`.

use crate::{CodeObject, Runtime, TaskContext};
#[cfg(test)]
use crate::Frame;
use contracts::{RootSource, ValueSlot};
use isa::value::{ObjectHeader, Value};
use std::ptr::NonNull;

/// The `RootSource` for one collection: the shared `Runtime` plus the coordinator's running task.
pub struct RuntimeRoots<'a> {
    rt: &'a Runtime,
    running: &'a mut TaskContext,
}

impl<'a> RuntimeRoots<'a> {
    pub fn new(rt: &'a Runtime, running: &'a mut TaskContext) -> Self {
        Self { rt, running }
    }
}

impl RootSource for RuntimeRoots<'_> {
    fn visit_roots(&mut self, visitor: &mut dyn FnMut(ValueSlot)) {
        scan_roots(self.rt, self.running, visitor);
    }
}

#[inline(always)]
fn visit_synthetic_root(visitor: &mut dyn FnMut(ValueSlot), ptr: NonNull<ObjectHeader>) {
    let mut v = Value::boxed(ptr);
    let slot = ValueSlot::new(&mut v);
    visitor(slot);
}

/// Enumerates all live `ObjectPtr` root slots within a single call frame.
#[cfg(test)]
pub(crate) fn enumerate_frame_roots(
    registers: &mut [Value],
    frame: &Frame,
    code_objects: &[CodeObject],
    visitor: &mut dyn FnMut(ValueSlot),
) {
    let reg_count = code_objects
        .get(frame.caller_code)
        .map(|c| c.register_count as usize)
        .unwrap_or(256);

    let start = frame.base;
    let end = (frame.base + reg_count).min(registers.len());

    for register in &mut registers[start..end] {
        let slot = ValueSlot::new(register);
        if slot.is_boxed_object() {
            visitor(slot);
        }
    }
}

/// Enumerates the live root slots of one task and nulls the dead registers above them.
pub(crate) fn enumerate_all_roots(
    task: &mut TaskContext,
    code_objects: &[CodeObject],
    visitor: &mut dyn FnMut(ValueSlot),
) {
    let extent = task.live_register_extent(code_objects);
    for idx in 0..extent {
        let slot = ValueSlot::new(&mut task.registers[idx]);
        if slot.is_boxed_object() {
            visitor(slot);
        }
    }
    task.clear_dead_registers(code_objects);
}

/// Reports globals, every queued/blocked/detached task, other workers' self-reported roots, and `running`.
pub(crate) fn scan_roots(rt: &Runtime, running: &mut TaskContext, visitor: &mut dyn FnMut(ValueSlot)) {
    scan_task_roots(running, &rt.code_objects, visitor);
    {
        let mut sched = rt.sched.lock().unwrap();
        for task in sched.run_queue.iter_mut() {
            scan_task_roots(task, &rt.code_objects, visitor);
        }
        for task in sched.blocked.values_mut() {
            scan_task_roots(task, &rt.code_objects, visitor);
        }

        for ptr in sched.detached.iter().copied() {
            visit_synthetic_root(visitor, ptr);
        }

        for ptr in rt.safepoint.take_reported_roots() {
            visit_synthetic_root(visitor, ptr);
        }
    }

    rt.gen_regions.for_each_heap_pointer(|ptr| visit_synthetic_root(visitor, ptr));

    for val in rt.globals.lock().unwrap().iter_mut() {
        let slot = ValueSlot::new(val);
        if slot.is_boxed_object() {
            visitor(slot);
        }
    }
}

fn scan_task_roots(
    task: &mut TaskContext,
    code_objects: &[CodeObject],
    visitor: &mut dyn FnMut(ValueSlot),
) {
    enumerate_all_roots(task, code_objects, visitor);

    for frame in &task.call_stack {
        if let Some(ptr) = frame.closure {
            visit_synthetic_root(visitor, ptr);
        }
        if let Some(link) = frame.generator {
            visit_synthetic_root(visitor, link.object);
        }
    }

    if let Some(ptr) = task.handle {
        visit_synthetic_root(visitor, ptr);
    }
    for &ptr in task.held_turns.iter().chain(&task.owned_senders) {
        visit_synthetic_root(visitor, ptr);
    }

    task.arenas.for_each_heap_pointer(|ptr| visit_synthetic_root(visitor, ptr));
    for (_, saved) in &task.arena_saves {
        saved.for_each_heap_pointer(|ptr| visit_synthetic_root(visitor, ptr));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr::NonNull;
    use isa::value::{ObjectHeader, TypeDescriptor, Value};

    #[test]
    fn test_single_frame_root_enumeration() {
        let mut dummy_type = TypeDescriptor::new(10, vec![]);
        let mut obj1 = ObjectHeader {
            type_ptr: NonNull::new(&mut dummy_type).unwrap(),
            gc_state: 0.into(),
        };
        let mut obj2 = ObjectHeader {
            type_ptr: NonNull::new(&mut dummy_type).unwrap(),
            gc_state: 0.into(),
        };

        let ptr1 = NonNull::new(&mut obj1).unwrap();
        let ptr2 = NonNull::new(&mut obj2).unwrap();

        let mut registers = vec![
            Value::small_int(123),
            Value::boxed(ptr1),
            Value::float(3.5),
            Value::true_(),
            Value::boxed(ptr2),
            Value::null(),
        ];

        let code_objects = vec![CodeObject::new(vec![], vec![], 6, 0)];
        let frame = Frame {
            return_pc: 0,
            base: 0,
            caller_code: 0,
            dest_reg: 0,
            closure: None,
            generator: None,
        };

        let mut collected_ptrs = Vec::new();
        enumerate_frame_roots(
            &mut registers,
            &frame,
            &code_objects,
            &mut |slot: ValueSlot| {
                if let Some(p) = slot.load_object_ptr() {
                    collected_ptrs.push(p);
                }
            },
        );

        assert_eq!(collected_ptrs.len(), 2);
        assert_eq!(collected_ptrs[0], ptr1);
        assert_eq!(collected_ptrs[1], ptr2);
    }

    #[test]
    fn test_multi_frame_call_stack_root_enumeration() {
        let mut dummy_type = TypeDescriptor::new(1, vec![]);
        let mut obj_parent = ObjectHeader {
            type_ptr: NonNull::new(&mut dummy_type).unwrap(),
            gc_state: 0.into(),
        };
        let mut obj_child = ObjectHeader {
            type_ptr: NonNull::new(&mut dummy_type).unwrap(),
            gc_state: 0.into(),
        };

        let ptr_parent = NonNull::new(&mut obj_parent).unwrap();
        let ptr_child = NonNull::new(&mut obj_child).unwrap();

        let mut registers = vec![Value::null(); 16];
        registers[0] = Value::small_int(10);
        registers[2] = Value::boxed(ptr_parent);

        registers[8] = Value::small_int(20);
        registers[9] = Value::boxed(ptr_child);

        let code_objects = vec![
            CodeObject::new(vec![], vec![], 8, 0),
            CodeObject::new(vec![], vec![], 8, 1),
        ];

        let call_stack = vec![
            Frame {
                return_pc: 0,
                base: 0,
                caller_code: 0,
                dest_reg: 0,
                closure: None,
                generator: None,
            },
            Frame {
                return_pc: 5,
                base: 8,
                caller_code: 1,
                dest_reg: 1,
                closure: None,
                generator: None,
            },
        ];

        let mut task = TaskContext {
            registers: std::mem::take(&mut registers),
            call_stack,
            current_code: 1,
            ..TaskContext::default()
        };
        let mut collected_slots = Vec::new();
        enumerate_all_roots(&mut task, &code_objects, &mut |slot: ValueSlot| {
            collected_slots.push(slot);
        });

        assert_eq!(collected_slots.len(), 2);
        assert_eq!(collected_slots[0].load_object_ptr(), Some(ptr_parent));
        assert_eq!(collected_slots[1].load_object_ptr(), Some(ptr_child));

        let mut obj_new = ObjectHeader {
            type_ptr: NonNull::new(&mut dummy_type).unwrap(),
            gc_state: 0.into(),
        };
        let ptr_new = NonNull::new(&mut obj_new).unwrap();

        collected_slots[1].store_object_ptr(ptr_new);
        assert!(matches!(task.registers[9], Value::ObjectPtr(_)));
        assert_eq!(task.registers[9].as_object_ptr(), Some(ptr_new));
    }
}