//! Handlers run against a `FakeMachine`: no `Runtime`, no collector.

use std::cell::RefCell;
use std::ptr::NonNull;

use isa::encoding::{encode_r3, encode_ri};
use isa::opcode::Opcode;
use isa::value::{ObjectHeader, TypeDescriptor, Value};

use super::{arena, object, strings};
use crate::alloc::BumpAllocator;
use crate::machine::{CodeTable, ObjectStore, TypeTable};
use crate::{CodeObject, Runtime, TaskContext, VmStatus};

struct FakeMachine {
    code: Vec<CodeObject>,
    types: Vec<TypeDescriptor>,
    string_type: TypeDescriptor,
    heap: RefCell<BumpAllocator>,
    allocs: RefCell<Vec<usize>>,
}

impl FakeMachine {
    fn new(types: Vec<TypeDescriptor>) -> Self {
        FakeMachine {
            code: vec![CodeObject::new(vec![], vec![], 8, 0).with_string_table(vec!["hi".into()])],
            types,
            string_type: TypeDescriptor::string_type(),
            heap: RefCell::new(BumpAllocator::default()),
            allocs: RefCell::new(Vec::new()),
        }
    }

    fn task(&self) -> TaskContext {
        TaskContext::entry(&Runtime::new(self.code.clone()))
    }
}

impl CodeTable for FakeMachine {
    fn code_objects(&self) -> &[CodeObject] {
        &self.code
    }
}

impl TypeTable for FakeMachine {
    fn type_descriptors(&self) -> &[TypeDescriptor] {
        &self.types
    }

    fn string_type(&self) -> &TypeDescriptor {
        &self.string_type
    }
}

impl ObjectStore for FakeMachine {
    fn alloc_object(&self, type_ptr: NonNull<TypeDescriptor>, slot_count: usize) -> NonNull<ObjectHeader> {
        self.allocs.borrow_mut().push(slot_count);
        self.heap.borrow_mut().alloc_object(type_ptr, slot_count)
    }
}

fn ri(op: Opcode, a: u8, bx: u16) -> u32 {
    encode_ri(op, a, bx) >> 8
}

fn r3(op: Opcode, a: u8, b: u8, c: u8) -> u32 {
    encode_r3(op, a, b, c) >> 8
}

fn point() -> TypeDescriptor {
    TypeDescriptor::new(1, vec![Some("x".into()), Some("y".into())])
}

#[test]
fn test_newobj_allocates_the_descriptors_slot_count_into_the_register() {
    let m = FakeMachine::new(vec![point()]);
    let mut task = m.task();
    let status = object::newobj(&m, &mut task, ri(Opcode::NEWOBJ, 2, 0)).unwrap();
    assert!(matches!(status, VmStatus::Running));
    assert_eq!(*m.allocs.borrow(), vec![2]);
    assert!(task.registers[2].as_object_ptr().is_some());
}

#[test]
fn test_newobj_rejects_an_unknown_type_index_without_allocating() {
    let m = FakeMachine::new(vec![point()]);
    let mut task = m.task();
    let err = object::newobj(&m, &mut task, ri(Opcode::NEWOBJ, 0, 5)).unwrap_err();
    assert!(err.contains("Invalid type descriptor index"));
    assert!(m.allocs.borrow().is_empty());
}

#[test]
fn test_setfield_stores_the_value_and_reads_it_back() {
    let m = FakeMachine::new(vec![point()]);
    let mut task = m.task();
    object::newobj(&m, &mut task, ri(Opcode::NEWOBJ, 0, 0)).unwrap();
    task.registers[1] = Value::small_int(7);
    object::setfield(&mut task, r3(Opcode::SETFIELD, 0, 1, 1)).unwrap();
    let obj = task.registers[0].as_object_ptr().unwrap();
    assert_eq!(unsafe { obj.as_ref().get_field(1) }.as_int(), Some(7));
}

#[test]
fn test_setfield_on_a_shared_object_is_a_fault() {
    let m = FakeMachine::new(vec![point()]);
    let mut task = m.task();
    object::newobj(&m, &mut task, ri(Opcode::NEWOBJ, 0, 0)).unwrap();
    isa::seal::seal_deep(task.registers[0].as_object_ptr().unwrap());
    let err = object::setfield(&mut task, r3(Opcode::SETFIELD, 0, 0, 1)).unwrap_err();
    assert!(err.contains("read-only"));
}

#[test]
fn test_newstr_allocates_a_string_holding_the_table_entry() {
    let m = FakeMachine::new(vec![]);
    let mut task = m.task();
    strings::newstr(&m, &mut task, ri(Opcode::NEWSTR, 3, 0)).unwrap();
    assert_eq!(*m.allocs.borrow(), vec![isa::value::string_slot_count(2)]);
    assert_eq!(task.registers[3].as_heap_string().as_deref(), Some("hi"));
}

#[test]
fn test_newstr_with_a_missing_table_entry_is_an_error() {
    let m = FakeMachine::new(vec![]);
    let mut task = m.task();
    assert!(strings::newstr(&m, &mut task, ri(Opcode::NEWSTR, 0, 9)).is_err());
}

#[test]
fn test_arena_alloc_rejects_an_unknown_type_index() {
    let m = FakeMachine::new(vec![point()]);
    let mut task = m.task();
    assert!(arena::alloc(&m, &mut task, ri(Opcode::ARENAALLOC, 0, 4)).is_err());
}
