//! The GC traces `header → backing → element` for objects allocated through the intrinsic seam and sizes them on sweep.

use gc::gc::GCController;
use gc::plan::GCConfig;
use contracts::Heap;
use runtime::{CodeObject, GcHook, NativeCtx, IntrinsicTypeTable, VmIntrinsicCtx, Runtime};
use isa::value::{Value, BACKING_TYPE_ID, LIST_TYPE_ID, MAP_TYPE_ID};

#[test]
fn intrinsic_objects_are_traced_and_swept() {
    let rt = Runtime::new(vec![CodeObject::new(vec![], vec![], 8, 0)]);
    let mut task = runtime::TaskContext::entry(&rt);
    let gc: GcHook = Box::new(GCController::new(GCConfig::mark_sweep().with_threshold(1)));

    let (list, backing, elem) = {
        let mut m = gc.mutator();
        let mut ctx = VmIntrinsicCtx {
            mutator: m.as_mut(),
            heap: gc.as_ref(),
            types: &rt.intrinsic_types,
            string_type: rt.string_type.as_ref(),
            platform: None,
        };
        let list = ctx.alloc_header(LIST_TYPE_ID, 2).unwrap();
        let backing = ctx.alloc_backing(4).unwrap();
        let elem = ctx.alloc_header(LIST_TYPE_ID, 2).unwrap();
        ctx.set_slot(list, 1, backing).unwrap();
        ctx.set_slot(backing, 1, elem).unwrap();
        (list, backing, elem)
    };

    assert_eq!(gc.stats().live_objects, 3);

    task.registers[0] = list;
    gc.collect(&mut runtime::RuntimeRoots::new(&rt, &mut task));
    assert_eq!(gc.stats().live_objects, 3, "header → backing → element all traced");

    unsafe {
        let b = backing.as_object_ptr().unwrap();
        assert_eq!(b.as_ref().type_ptr.as_ref().id, BACKING_TYPE_ID);
        assert_eq!(b.as_ref().get_field(1).as_object_ptr(), elem.as_object_ptr());
    }

    task.registers[0] = Value::null();
    gc.collect(&mut runtime::RuntimeRoots::new(&rt, &mut task));
    assert_eq!(gc.stats().live_objects, 0);
    assert!(gc.stats().bytes_freed > 0);
}

#[test]
fn map_backing_traces_keys_and_values_and_sizes_on_sweep() {
    let rt = Runtime::new(vec![CodeObject::new(vec![], vec![], 8, 0)]);
    let mut task = runtime::TaskContext::entry(&rt);
    let gc: GcHook = Box::new(GCController::new(GCConfig::mark_sweep().with_threshold(1)));

    let (map, kobj, vobj) = {
        let mut m = gc.mutator();
        let mut ctx = VmIntrinsicCtx {
            mutator: m.as_mut(),
            heap: gc.as_ref(),
            types: &rt.intrinsic_types,
            string_type: rt.string_type.as_ref(),
            platform: None,
        };
        let map = ctx.alloc_header(MAP_TYPE_ID, 2).unwrap();
        let backing = ctx.alloc_map_backing(4).unwrap();
        let kobj = ctx.alloc_header(MAP_TYPE_ID, 2).unwrap();
        let vobj = ctx.alloc_header(MAP_TYPE_ID, 2).unwrap();
        ctx.set_slot(map, 1, backing).unwrap();
        ctx.set_slot(backing, 1 + 2 * 2, kobj).unwrap();
        ctx.set_slot(backing, 2 + 2 * 2, vobj).unwrap();
        (map, kobj, vobj)
    };

    assert_eq!(gc.stats().live_objects, 4);
    task.registers[0] = Value::boxed(map.as_object_ptr().unwrap());
    gc.collect(&mut runtime::RuntimeRoots::new(&rt, &mut task));
    assert_eq!(gc.stats().live_objects, 4, "map header → backing → key & value all traced");
    let _ = (kobj, vobj);

    task.registers[0] = Value::null();
    gc.collect(&mut runtime::RuntimeRoots::new(&rt, &mut task));
    assert_eq!(gc.stats().live_objects, 0);
}

#[test]
fn leaking_heap_default_allocates_intrinsics_when_no_gc_installed() {
    let types = IntrinsicTypeTable::new();
    let heap = runtime::alloc::LeakingHeap::default();
    let mut m = heap.mutator();
    let string_type = isa::value::TypeDescriptor::string_type();
    let mut ctx = VmIntrinsicCtx {
        mutator: m.as_mut(),
        heap: &heap,
        types: &types,
        string_type: &string_type,
        platform: None,
    };

    let backing = ctx.alloc_backing(2).unwrap();
    ctx.set_slot(backing, 1, Value::int(7)).unwrap();
    ctx.set_slot(backing, 2, Value::int(9)).unwrap();

    assert_eq!(ctx.get_slot(backing, 0).unwrap().as_uint(), Some(2));
    assert_eq!(ctx.get_slot(backing, 1).unwrap().as_int(), Some(7));
    assert_eq!(ctx.get_slot(backing, 2).unwrap().as_int(), Some(9));

    assert!(ctx.alloc_header(0xDEAD, 2).is_err(), "non-intrinsic id rejected");

    let bb = ctx.alloc_bytes_backing(8).unwrap();
    ctx.write_bytes(bb, 0, b"hello").unwrap();
    ctx.write_bytes(bb, 5, b"!").unwrap();
    assert_eq!(ctx.read_bytes(bb, 0, 6).unwrap(), b"hello!");
    assert!(ctx.write_bytes(bb, 6, b"toolong").is_err(), "overflow rejected");

    let s = ctx.alloc_string(b"decoded").unwrap();
    assert_eq!(s.as_heap_string().as_deref(), Some("decoded"));
}

#[test]
fn a_bytes_backing_exposes_a_raw_pointer() {
    let types = IntrinsicTypeTable::new();
    let heap = runtime::alloc::LeakingHeap::default();
    let mut m = heap.mutator();
    let string_type = isa::value::TypeDescriptor::string_type();
    let mut ctx = VmIntrinsicCtx {
        mutator: m.as_mut(),
        heap: &heap,
        types: &types,
        string_type: &string_type,
        platform: None,
    };

    let op = ctx.alloc_bytes_backing(8).unwrap();
    assert_eq!(ctx.read_bytes(op, 0, 8).unwrap(), vec![0u8; 8]);

    ctx.write_bytes(op, 0, b"hellohel").unwrap();
    assert_eq!(ctx.read_bytes(op, 0, 8).unwrap(), b"hellohel");
    assert!(ctx.write_bytes(op, 8, b"!").is_err(), "overflow rejected");

    let raw = ctx.bytes_ptr(op).unwrap();
    unsafe { *raw.add(0) = b'H' };
    assert_eq!(ctx.read_bytes(op, 0, 1).unwrap(), b"H");
    ctx.write_bytes(op, 1, b"i").unwrap();
    assert_eq!(unsafe { *raw.add(1) }, b'i');
}

#[test]
fn a_bytes_backing_is_never_traced_and_is_correctly_sized_on_sweep() {
    let rt = Runtime::new(vec![CodeObject::new(vec![], vec![], 8, 0)]);
    let mut task = runtime::TaskContext::entry(&rt);
    let gc: GcHook = Box::new(GCController::new(GCConfig::mark_sweep().with_threshold(1)));

    let op = {
        let mut m = gc.mutator();
        let mut ctx = VmIntrinsicCtx {
            mutator: m.as_mut(),
            heap: gc.as_ref(),
            types: &rt.intrinsic_types,
            string_type: rt.string_type.as_ref(),
            platform: None,
        };
        let op = ctx.alloc_bytes_backing(16).unwrap();
        ctx.write_bytes(op, 0, &0xDEAD_BEEF_u64.to_ne_bytes()).unwrap();
        ctx.write_bytes(op, 8, &0xFFFF_FFFF_FFFF_F100_u64.to_ne_bytes()).unwrap();
        op
    };

    assert_eq!(gc.stats().live_objects, 1);
    task.registers[0] = op;
    gc.collect(&mut runtime::RuntimeRoots::new(&rt, &mut task));
    assert_eq!(gc.stats().live_objects, 1, "the backing survives while rooted");

    let bytes = {
        let mut m = gc.mutator();
        let ctx = VmIntrinsicCtx {
            mutator: m.as_mut(),
            heap: gc.as_ref(),
            types: &rt.intrinsic_types,
            string_type: rt.string_type.as_ref(),
            platform: None,
        };
        ctx.read_bytes(task.registers[0], 0, 16).unwrap()
    };
    assert_eq!(&bytes[0..8], &0xDEAD_BEEF_u64.to_ne_bytes());
    assert_eq!(&bytes[8..16], &0xFFFF_FFFF_FFFF_F100_u64.to_ne_bytes());

    task.registers[0] = Value::null();
    gc.collect(&mut runtime::RuntimeRoots::new(&rt, &mut task));
    assert_eq!(gc.stats().live_objects, 0);
    assert!(gc.stats().bytes_freed > 0);
}

#[test]
fn native_allocated_string_is_traced_and_swept() {
    let rt = Runtime::new(vec![CodeObject::new(vec![], vec![], 8, 0)]);
    let mut task = runtime::TaskContext::entry(&rt);
    let gc: GcHook = Box::new(GCController::new(GCConfig::mark_sweep().with_threshold(1)));

    let s = {
        let mut m = gc.mutator();
        let mut ctx = VmIntrinsicCtx {
            mutator: m.as_mut(),
            heap: gc.as_ref(),
            types: &rt.intrinsic_types,
            string_type: rt.string_type.as_ref(),
            platform: None,
        };
        ctx.alloc_string(b"the-string-that-must-survive").unwrap()
    };
    task.registers[0] = s;
    gc.collect(&mut runtime::RuntimeRoots::new(&rt, &mut task));
    assert_eq!(gc.stats().live_objects, 1);
    assert_eq!(task.registers[0].as_heap_string().as_deref(), Some("the-string-that-must-survive"));

    task.registers[0] = Value::null();
    gc.collect(&mut runtime::RuntimeRoots::new(&rt, &mut task));
    assert_eq!(gc.stats().live_objects, 0);
}
