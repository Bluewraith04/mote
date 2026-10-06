use std::sync::Arc;

use runtime::{CodeObject, NativeCtx, Runtime};
use ffi::native_call::NativeFunctionRegistry;
use isa::encoding::*;
use isa::opcode::Opcode;
use isa::value::{Value, BACKING_TYPE_ID};

fn install(rt: &mut Runtime, registry: NativeFunctionRegistry) {
    rt.set_native_dispatcher(Arc::new(move |fn_idx, heap: &mut dyn NativeCtx, args| {
        registry.call(fn_idx, args, heap)
    }));
}

#[test]
fn test_native_call_roundtrip() {
    let registry = NativeFunctionRegistry::new();

    let func_idx = registry.register(
        "add_two",
        |ctx| {
            let a = ctx.arg(0).and_then(|v| v.as_int()).unwrap_or(0);
            let b = ctx.arg(1).and_then(|v| v.as_int()).unwrap_or(0);
            Ok(Value::int(a + b))
        },
    );
    assert_eq!(func_idx, 0);

    let code = vec![
        encode_ri(Opcode::LOADI, 0, 30),
        encode_ri(Opcode::LOADI, 1, 70),
        encode_r3(Opcode::CALLNATIVE, 2, 0, 0),
        encode_r2(Opcode::RET, 2, 0),
    ];

    let mut rt = Runtime::new(vec![CodeObject::new(code, vec![], 4, 0)]);
    install(&mut rt, registry);

    let task = rt.run_entry().unwrap();
    let status = rt.status();
    assert_eq!(status, runtime::VmStatus::Halted);
    assert_eq!(task.registers[0].as_int(), Some(100));
}

#[test]
fn test_native_allocates_a_backing_through_the_seam() {
    let registry = NativeFunctionRegistry::new();
    registry.register(
        "make_singleton",
        |ctx| {
            let elem = ctx.arg(0).unwrap_or(Value::null());
            let backing = ctx.heap.alloc_backing(4)?;
            ctx.heap.set_slot(backing, 1, elem)?;
            Ok(backing)
        },
    );

    let code = vec![
        encode_ri(Opcode::LOADI, 0, 42),
        encode_r3(Opcode::CALLNATIVE, 1, 0, 0),
        encode_r2(Opcode::RET, 1, 0),
    ];
    let mut rt = Runtime::new(vec![CodeObject::new(code, vec![], 4, 0)]);
    install(&mut rt, registry);
    let task = rt.run_entry().unwrap();
    assert_eq!(rt.status(), runtime::VmStatus::Halted);

    let backing = task.registers[1];
    let ptr = backing.as_object_ptr().expect("backing is a heap object");
    unsafe {
        let header = ptr.as_ref();
        assert_eq!(header.type_ptr.as_ref().id, BACKING_TYPE_ID);
        assert_eq!(header.get_field(0).as_uint(), Some(4));
        assert_eq!(header.get_field(1).as_int(), Some(42));
    }
}

#[test]
fn test_seam_rejects_non_intrinsic_id() {
    let registry = NativeFunctionRegistry::new();
    registry.register(
        "bad_alloc",
        |ctx| ctx.heap.alloc_header(0x1234, 2),
    );
    let code = vec![
        encode_r3(Opcode::CALLNATIVE, 0, 0, 0),
        encode_r2(Opcode::RET, 0, 0),
    ];
    let mut rt = Runtime::new(vec![CodeObject::new(code, vec![], 4, 0)]);
    install(&mut rt, registry);
    let err = rt.run_entry().unwrap_err();
    assert!(err.contains("intrinsic"), "got: {err}");
}

#[test]
fn test_callnativew_resolves_by_name_whatever_the_registry_order() {
    let registry = NativeFunctionRegistry::new();
    registry.register("filler", |_| Ok(Value::int(-1)));
    registry.register("add_two", |ctx| {
        let a = ctx.arg(0).and_then(|v| v.as_int()).unwrap_or(0);
        let b = ctx.arg(1).and_then(|v| v.as_int()).unwrap_or(0);
        Ok(Value::int(a + b))
    });

    let code = vec![
        encode_ri(Opcode::LOADI, 1, 30),
        encode_ri(Opcode::LOADI, 2, 70),
        encode_callnativew(0, 0),
        encode_r2(Opcode::RET, 0, 0),
    ];
    let mut rt = Runtime::new(vec![CodeObject::new(code, vec![], 4, 0)]);
    rt.set_native_resolver(Arc::new({
        let registry = registry.clone();
        move |name| registry.get_by_name(name)
    }));
    install(&mut rt, registry);
    rt.set_native_table(&["add_two".to_string()]).unwrap();
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(100));
}

#[test]
fn test_callnativew_outside_the_table_is_an_error() {
    let code = vec![encode_callnativew(0, 5), encode_r2(Opcode::RET, 0, 0)];
    let mut rt = Runtime::new(vec![CodeObject::new(code, vec![], 2, 0)]);
    let err = rt.run_entry().unwrap_err();
    assert!(err.contains("native table"), "{err}");
}
