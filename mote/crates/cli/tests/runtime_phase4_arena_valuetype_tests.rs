use contracts::Heap;
use std::ptr::NonNull;
use gc::gc::GCController;
use gc::plan::GCConfig;
use runtime::arena::ArenaStack;
use runtime::{CodeObject, Runtime};
use isa::encoding::*;
use isa::opcode::Opcode;
use isa::value::{TypeDescriptor, TypeRegistry, Value};

#[test]
fn test_arena_allocator_unit() {
    let mut arena = ArenaStack::new();
    let mut type_desc = TypeDescriptor::new(1, vec![Some("x".into()), Some("y".into())]);
    let type_ptr = NonNull::new(&mut type_desc).unwrap();

    arena.enter_arena();
    let obj1 = arena.alloc_object(type_ptr).unwrap();
    let obj2 = arena.alloc_object(type_ptr).unwrap();

    assert_ne!(obj1.as_ptr(), obj2.as_ptr());
    assert_eq!(obj2.as_ptr() as usize - obj1.as_ptr() as usize, 48);

    arena.exit_arena().unwrap();
    let mut left = 0;
    arena.for_each_object(|_| left += 1);
    assert_eq!(left, 0);
}

#[test]
fn test_nested_arena_scoping() {
    let mut stack = ArenaStack::new();
    let mut type_desc = TypeDescriptor::new(1, vec![Some("val".into())]);
    let type_ptr = NonNull::new(&mut type_desc).unwrap();

    stack.enter_arena();
    let outer_obj = stack.alloc_object(type_ptr).unwrap();

    stack.enter_arena();
    let inner_obj = stack.alloc_object(type_ptr).unwrap();

    assert_ne!(outer_obj.as_ptr(), inner_obj.as_ptr());
    assert_eq!(stack.depth(), 2);

    assert!(stack.exit_arena().is_ok());
    assert_eq!(stack.depth(), 1);

    assert_eq!(stack.depth(), 1);

    assert!(stack.exit_arena().is_ok());
    assert_eq!(stack.depth(), 0);
}

#[test]
fn test_arena_as_gc_root_source() {
    let gc = GCController::new(GCConfig::mark_sweep());
    let mut node_type = TypeDescriptor::with_pointer_mask(
        1,
        vec![Some("child".into())],
        0b1,
    );
    let type_ptr = NonNull::new(&mut node_type).unwrap();

    let rt = Runtime::new(vec![CodeObject::new(vec![], vec![], 8, 0)]);
    let mut task = runtime::TaskContext::entry(&rt);

    let heap_child = gc.alloc_object(type_ptr);

    task.arenas.enter_arena();
    let arena_parent = task.arenas.alloc_object(type_ptr).unwrap();

    unsafe {
        (*arena_parent.as_ptr()).set_field(0, Value::boxed(heap_child));
    }

    task.registers[0] = Value::boxed(arena_parent);

    gc.collect(&mut runtime::RuntimeRoots::new(&rt, &mut task));

    assert_eq!(gc.live_objects_count(), 1);
    unsafe {
        assert_eq!((*arena_parent.as_ptr()).get_field(0).as_object_ptr(), Some(heap_child));
    }

    task.arenas.exit_arena().unwrap();
    task.registers[0] = Value::null();

    gc.collect(&mut runtime::RuntimeRoots::new(&rt, &mut task));
    assert_eq!(gc.live_objects_count(), 0);
}

#[test]
fn test_pointer_discipline_heap_cannot_point_to_arena() {
    let mut type_desc = TypeDescriptor::new(1, vec![Some("ref".into())]);
    let type_ptr = NonNull::new(&mut type_desc).unwrap();

    let gc = GCController::new(GCConfig::mark_sweep());
    let heap_obj = gc.alloc_object(type_ptr);

    let mut arena = ArenaStack::new();
    arena.enter_arena();
    let arena_obj = arena.alloc_object(type_ptr).unwrap();

    let mut registers = vec![
        Value::boxed(heap_obj),
        Value::boxed(arena_obj),
    ];

    let result = runtime::handlers::three_reg::setfield(&mut registers, 0, 0, 1);

    assert!(result.unwrap_err().contains("would outlive"));
    unsafe { assert_eq!((*heap_obj.as_ptr()).get_field(0).as_object_ptr(), None, "nothing was stored") };
}

#[test]
fn test_pointer_discipline_inner_region_cannot_be_stored_in_an_outer_one() {
    let mut type_desc = TypeDescriptor::new(1, vec![Some("ref".into())]);
    let type_ptr = NonNull::new(&mut type_desc).unwrap();
    let mut arena = ArenaStack::new();
    arena.enter_arena();
    let outer = arena.alloc_object(type_ptr).unwrap();
    arena.enter_arena();
    let inner = arena.alloc_object(type_ptr).unwrap();
    assert_eq!(unsafe { outer.as_ref() }.region_depth(), 1);
    assert_eq!(unsafe { inner.as_ref() }.region_depth(), 2);

    let mut registers = vec![Value::boxed(outer), Value::boxed(inner)];
    assert!(runtime::handlers::three_reg::setfield(&mut registers, 0, 0, 1).is_err(), "outer <- inner");
    registers.swap(0, 1);
    assert!(runtime::handlers::three_reg::setfield(&mut registers, 0, 0, 1).is_ok(), "inner <- outer");
}

#[test]
fn test_regions_nested_past_the_recordable_depth_are_too_deep() {
    let mut arena = ArenaStack::new();
    for _ in 0..isa::value::MAX_REGION_DEPTH {
        arena.enter_arena();
    }
    assert!(!arena.too_deep());
    arena.enter_arena();
    assert!(arena.too_deep());
}

#[test]
fn test_pointer_discipline_arena_can_point_to_heap() {
    let mut type_desc = TypeDescriptor::new(1, vec![Some("ref".into())]);
    let type_ptr = NonNull::new(&mut type_desc).unwrap();

    let gc = GCController::new(GCConfig::mark_sweep());
    let heap_obj = gc.alloc_object(type_ptr);

    let mut arena = ArenaStack::new();
    arena.enter_arena();
    let arena_obj = arena.alloc_object(type_ptr).unwrap();

    let mut registers = vec![
        Value::boxed(arena_obj),
        Value::boxed(heap_obj),
    ];

    runtime::handlers::three_reg::setfield(&mut registers, 0, 0, 1).unwrap();

    unsafe {
        assert_eq!((*arena_obj.as_ptr()).get_field(0).as_object_ptr(), Some(heap_obj));
    }
}

#[test]
fn test_value_type_structural_stickiness() {
    let mut registry = TypeRegistry::new();

    let point_type = TypeDescriptor::new_value_type(
        10,
        vec![Some("x".into()), Some("y".into())],
        true,
    );
    assert!(registry.register(point_type, Some("Point".into())).is_ok());

    let mut pointer_type = TypeDescriptor::with_pointer_mask(11, vec![Some("ptr".into())], 0b1);
    pointer_type.is_value_type = true;
    assert!(registry.register(pointer_type, Some("BadPoint".into())).is_err());
}

#[test]
fn test_multi_slot_value_type_calling_convention() {

    let sum3_code = vec![
        encode_r3(Opcode::ADD, 3, 0, 1),
        encode_r3(Opcode::ADD, 3, 3, 2),
        encode_r2(Opcode::RET, 3, 0),
    ];

    let main_code = vec![
        encode_ri(Opcode::LOADI, 4, 10),
        encode_ri(Opcode::LOADI, 5, 20),
        encode_ri(Opcode::LOADI, 6, 30),
        encode_call(3, 1),
        encode_r2(Opcode::RET, 3, 0),
    ];

    let code_objects = vec![
        CodeObject::new(main_code, vec![], 8, 0),
        CodeObject::new(sum3_code, vec![], 4, 3),
    ];

    let mut rt = Runtime::new(code_objects);
    let task = rt.run_entry().unwrap();
    let status = rt.status();
    assert_eq!(status, runtime::VmStatus::Halted);
    assert_eq!(task.registers[0].as_int(), Some(60));
}

#[test]
fn test_cross_tier_four_memory_tiers_composition() {
    let gc = GCController::new(GCConfig::mark_sweep());
    let mut type_desc = TypeDescriptor::with_pointer_mask(
        1,
        vec![Some("child".into()), Some("val".into())],
        0b1,
    );
    let type_ptr = NonNull::new(&mut type_desc).unwrap();

    let stack_val = Value::small_int(42);

    let val_type = Value::inline_pair_i32(100, 200);

    let heap_obj = gc.alloc_object(type_ptr);
    unsafe {
        (*heap_obj.as_ptr()).set_field(1, Value::small_int(999));
    }

    let rt = Runtime::new(vec![CodeObject::new(vec![], vec![], 8, 0)]);
    let mut task = runtime::TaskContext::entry(&rt);
    task.arenas.enter_arena();
    let arena_obj = task.arenas.alloc_object(type_ptr).unwrap();
    unsafe {
        (*arena_obj.as_ptr()).set_field(0, Value::boxed(heap_obj));
        (*arena_obj.as_ptr()).set_field(1, Value::small_int(888));
    }

    task.registers[0] = stack_val;
    task.registers[1] = val_type;
    task.registers[2] = Value::boxed(heap_obj);
    task.registers[3] = Value::boxed(arena_obj);

    gc.collect(&mut runtime::RuntimeRoots::new(&rt, &mut task));

    assert_eq!(task.registers[0].as_int(), Some(42));
    assert_eq!(task.registers[1].as_inline_pair_i32(), Some((100, 200)));
    assert_eq!(task.registers[2].as_object_ptr(), Some(heap_obj));
    assert_eq!(task.registers[3].as_object_ptr(), Some(arena_obj));

    unsafe {
        assert_eq!((*heap_obj.as_ptr()).get_field(1).as_int(), Some(999));
        assert_eq!((*arena_obj.as_ptr()).get_field(0).as_object_ptr(), Some(heap_obj));
        assert_eq!((*arena_obj.as_ptr()).get_field(1).as_int(), Some(888));
    }
}

#[test]
fn test_returning_a_region_value_after_its_region_ended_faults() {
    let leak_code = vec![
        encode_none(Opcode::ENTERARENA),
        encode_ri(Opcode::ARENAALLOC, 0, 0),
        encode_none(Opcode::EXITARENA),
        encode_r2(Opcode::RET, 0, 0),
    ];
    let main_code = vec![encode_call(3, 1), encode_r2(Opcode::RET, 3, 0)];
    let types = vec![TypeDescriptor::new(1, vec![Some("x".into())])];
    let mut rt = Runtime::with_types(vec![CodeObject::new(main_code, vec![], 8, 0), CodeObject::new(leak_code, vec![], 4, 0)], types);
    let err = rt.run_entry().expect_err("the return must fault");
    assert!(format!("{err:?}").contains("would outlive"), "{err:?}");
}

#[test]
fn test_returning_a_region_value_while_its_region_is_open_is_allowed() {
    let code = vec![
        encode_none(Opcode::ENTERARENA),
        encode_ri(Opcode::ARENAALLOC, 0, 0),
        encode_r2(Opcode::RET, 0, 0),
    ];
    let types = vec![TypeDescriptor::new(1, vec![Some("x".into())])];
    let mut rt = Runtime::with_types(vec![CodeObject::new(code, vec![], 4, 0)], types);
    assert!(rt.run_entry().is_ok());
}
