use contracts::Heap;
use std::ptr::NonNull;
use gc::gc::GCController;
use gc::plan::GCConfig;
use runtime::{CodeObject, Runtime};
use isa::encoding::{encode_jc, encode_ju, encode_r2, encode_r3, encode_ri};
use isa::opcode::Opcode;
use isa::value::{TypeDescriptor, Value};

#[test]
fn test_gc_alloc_and_trace_across_small_programs() {
    let mut type_desc = TypeDescriptor::new(
        1,
        vec![Some("a".into()), Some("b".into())],
    );
    let type_ptr = NonNull::new(&mut type_desc).unwrap();

    let gc = GCController::new(GCConfig::mark_sweep());

    {
        let obj = gc.alloc_object(type_ptr);
        unsafe {
            (*obj.as_ptr()).set_field(0, Value::small_int(10));
            (*obj.as_ptr()).set_field(1, Value::small_int(20));
        }

        let code = vec![
            encode_ri(Opcode::LOADI, 0, 10),
            encode_ri(Opcode::LOADI, 1, 20),
            encode_r3(Opcode::ADD, 2, 0, 1),
            encode_r2(Opcode::RET, 2, 0),
        ];

        let mut rt = Runtime::new(vec![CodeObject::new(code, vec![], 4, 0)]);
        let task = rt.run_entry().unwrap();
        assert_eq!(task.registers[0].as_int(), Some(30));
    }

    {
        let code = vec![
            encode_ri(Opcode::LOADI, 0, 0),
            encode_ri(Opcode::LOADI, 1, 10),
            encode_ri(Opcode::LOADI, 2, 1),
            encode_r3(Opcode::LT, 3, 0, 1),
            encode_jc(Opcode::JMPIFNOT, 3, 3),
            encode_r3(Opcode::ADD, 0, 0, 2),
            encode_ju(Opcode::JMP, -3),
            encode_r2(Opcode::RET, 0, 0),
        ];

        let mut rt = Runtime::new(vec![CodeObject::new(code, vec![], 4, 0)]);
        let task = rt.run_entry().unwrap();
        assert_eq!(task.registers[0].as_int(), Some(10));
    }

    {
        let obj1 = gc.alloc_object(type_ptr);
        let obj2 = gc.alloc_object(type_ptr);

        unsafe {
            (*obj1.as_ptr()).set_field(0, Value::small_int(100));
            (*obj1.as_ptr()).set_field(1, Value::boxed(obj2));
            (*obj2.as_ptr()).set_field(0, Value::small_int(200));
        }

        unsafe {
            assert_eq!((*obj1.as_ptr()).get_field(0).as_int(), Some(100));
            let child = (*obj1.as_ptr()).get_field(1).as_object_ptr().unwrap();
            assert_eq!((*child.as_ptr()).get_field(0).as_int(), Some(200));
        }
    }
}

#[test]
fn test_allocation_stress_and_fuzz_harness() {
    let mut type_node = TypeDescriptor::with_pointer_mask(
        1,
        vec![Some("val".into()), Some("next".into())],
        0b10,
    );
    let type_ptr = NonNull::new(&mut type_node).unwrap();

    let config = GCConfig::mark_sweep().with_threshold(1024);
    let gc = GCController::new(config);

    let rt = Runtime::new(vec![CodeObject::new(vec![], vec![], 16, 0)]);
    let mut task = runtime::TaskContext::entry(&rt);

    let root_a = gc.alloc_object(type_ptr);
    let root_b = gc.alloc_object(type_ptr);
    task.registers[0] = Value::boxed(root_a);
    task.registers[1] = Value::boxed(root_b);

    let mut state: u64 = 0x12345678;
    let mut pseudo_rand = || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        (state >> 33) as usize
    };

    let mut current_tail = root_a;

    for i in 0..2000 {
        let action = pseudo_rand() % 3;
        match action {
            0 => {
                let new_node = gc.alloc_object(type_ptr);
                unsafe {
                    (*new_node.as_ptr()).set_field(0, Value::small_int(i as i64));
                    (*current_tail.as_ptr()).set_field(1, Value::boxed(new_node));
                }
                current_tail = new_node;
            }
            1 => {
                let garbage = gc.alloc_object(type_ptr);
                unsafe {
                    (*garbage.as_ptr()).set_field(0, Value::small_int(i as i64));
                }
            }
            _ => {
                let fresh = gc.alloc_object(type_ptr);
                unsafe {
                    (*fresh.as_ptr()).set_field(0, Value::small_int(999));
                }
                task.registers[1] = Value::boxed(fresh);
            }
        }

        if gc.should_collect() {
            gc.collect(&mut runtime::RuntimeRoots::new(&rt, &mut task));
        }
    }

    gc.collect(&mut runtime::RuntimeRoots::new(&rt, &mut task));

    assert!(gc.collections_count() > 5);
    assert!(gc.total_freed_bytes() > 0);

    let mut count = 0;
    let mut curr = Some(root_a);
    while let Some(node) = curr {
        count += 1;
        curr = unsafe { (*node.as_ptr()).get_field(1).as_object_ptr() };
    }
    assert!(count > 10, "Chained nodes must survive collections");
}