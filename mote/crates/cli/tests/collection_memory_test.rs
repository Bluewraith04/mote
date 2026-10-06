//! Collections grow gently, shrink when emptied, and free the buffer they replace.

use contracts::{NativeCtx, NativeOutcome, NativeRegistry};
use ffi::builtins::registry;
use gc::gc::GCController;
use gc::plan::GCConfig;
use isa::value::{Value, BACKING_CAP_SLOT, HEADER_BACKING_SLOT};
use runtime::{CodeObject, GcHook, Runtime, VmIntrinsicCtx};

fn with_heap(body: impl FnOnce(&mut VmIntrinsicCtx<'_>, &GcHook)) {
    let rt = Runtime::new(vec![CodeObject::new(vec![], vec![], 8, 0)]);
    let gc: GcHook = Box::new(GCController::new(GCConfig::mark_sweep().with_threshold(1 << 30)));
    let mut mutator = gc.mutator();
    let mut ctx = VmIntrinsicCtx {
        mutator: mutator.as_mut(),
        heap: gc.as_ref(),
        types: &rt.intrinsic_types,
        string_type: rt.string_type.as_ref(),
        platform: None,
    };
    body(&mut ctx, &gc);
}

thread_local! {
    static REGISTRY: ffi::NativeFunctionRegistry = registry();
}

fn call(ctx: &mut VmIntrinsicCtx<'_>, name: &str, args: &[Value]) -> Value {
    REGISTRY.with(|reg| {
        let id = reg.resolve(name).unwrap_or_else(|| panic!("{name} is not registered"));
        match reg.call(id, args, ctx) {
            NativeOutcome::Done(v) => v,
            NativeOutcome::Fail(e) => panic!("{name} failed: {}", e.message),
            _ => panic!("{name} did not finish"),
        }
    })
}

fn capacity(ctx: &VmIntrinsicCtx<'_>, coll: Value) -> usize {
    let backing = ctx.get_slot(coll, HEADER_BACKING_SLOT).unwrap();
    ctx.get_slot(backing, BACKING_CAP_SLOT).unwrap().as_uint().unwrap() as usize
}

fn fill(ctx: &mut VmIntrinsicCtx<'_>, list: Value, n: i64) {
    for i in 0..n {
        call(ctx, "coll_push", &[list, Value::int(i)]);
    }
}

#[test]
fn growing_a_list_frees_each_replaced_backing() {
    with_heap(|ctx, gc| {
        let list = call(ctx, "list_new", &[Value::int(0)]);
        fill(ctx, list, 5000);
        let live = gc.stats().live_objects;
        assert!(live <= 8, "the header, the backing, and a few small early backings; got {live}");
    });
}

#[test]
fn popping_a_list_empty_shrinks_its_backing() {
    with_heap(|ctx, _| {
        let list = call(ctx, "list_new", &[Value::int(0)]);
        fill(ctx, list, 4096);
        assert_eq!(capacity(ctx, list), 4096);

        for _ in 0..3996 {
            call(ctx, "list_pop", &[list]);
        }
        assert!(capacity(ctx, list) <= 512, "100 elements no longer need 4096 slots");
        for i in 0..100 {
            assert_eq!(call(ctx, "coll_get", &[list, Value::int(i)]).as_int(), Some(i), "element {i} survived the shrinks");
        }

        for _ in 0..100 {
            call(ctx, "list_pop", &[list]);
        }
        assert!(capacity(ctx, list) <= 64);
        fill(ctx, list, 10);
        assert_eq!(call(ctx, "coll_get", &[list, Value::int(9)]).as_int(), Some(9));
    });
}

#[test]
fn a_backing_past_one_mebibyte_grows_by_half() {
    with_heap(|ctx, _| {
        let list = call(ctx, "list_new", &[Value::int(65536)]);
        fill(ctx, list, 65536);
        assert_eq!(capacity(ctx, list), 65536);
        fill(ctx, list, 1);
        assert_eq!(capacity(ctx, list), 98304, "1 MiB of slots grows to 1.5 MiB, not 2");

        let bytes = call(ctx, "bytes_zeros", &[Value::int(1 << 20)]);
        call(ctx, "coll_push", &[bytes, Value::int(7)]);
        assert_eq!(capacity(ctx, bytes), (1 << 20) + (1 << 19));
    });
}

#[test]
fn clearing_a_large_list_returns_it_to_a_small_backing() {
    with_heap(|ctx, _| {
        let list = call(ctx, "list_new", &[Value::int(0)]);
        fill(ctx, list, 4096);
        call(ctx, "coll_clear", &[list]);
        assert_eq!(capacity(ctx, list), 4);
        assert_eq!(call(ctx, "len", &[list]).as_int(), Some(0));
        fill(ctx, list, 6);
        assert_eq!(call(ctx, "coll_get", &[list, Value::int(5)]).as_int(), Some(5));
    });
}

#[test]
fn clearing_large_bytes_returns_them_to_a_small_backing() {
    with_heap(|ctx, _| {
        let bytes = call(ctx, "bytes_zeros", &[Value::int(4096)]);
        call(ctx, "coll_clear", &[bytes]);
        assert_eq!(capacity(ctx, bytes), 16);
        assert_eq!(call(ctx, "len", &[bytes]).as_int(), Some(0));
    });
}

#[test]
fn clearing_a_map_forgets_every_key() {
    with_heap(|ctx, _| {
        let map = call(ctx, "map_new", &[Value::int(0)]);
        for i in 0..20 {
            call(ctx, "coll_set", &[map, Value::int(i), Value::int(i)]);
        }
        call(ctx, "coll_clear", &[map]);
        for i in 0..20 {
            assert_eq!(call(ctx, "coll_contains", &[map, Value::int(i)]).as_bool(), Some(false), "key {i} outlived clear()");
        }
    });
}

#[test]
fn clearing_a_large_map_returns_it_to_a_small_table() {
    with_heap(|ctx, _| {
        let map = call(ctx, "map_new", &[Value::int(0)]);
        for i in 0..200 {
            call(ctx, "coll_set", &[map, Value::int(i), Value::int(i)]);
        }
        assert!(capacity(ctx, map) >= 256);
        call(ctx, "coll_clear", &[map]);
        assert_eq!(capacity(ctx, map), 8);
        call(ctx, "coll_set", &[map, Value::int(3), Value::int(4)]);
        assert_eq!(call(ctx, "coll_get", &[map, Value::int(3)]).as_int(), Some(4));
    });
}

#[test]
fn removing_from_a_map_shrinks_its_table_and_keeps_the_rest() {
    with_heap(|ctx, _| {
        let map = call(ctx, "map_new", &[Value::int(0)]);
        for i in 0..300 {
            call(ctx, "coll_set", &[map, Value::int(i), Value::int(i * 2)]);
        }
        let grown = capacity(ctx, map);
        for i in 10..300 {
            assert_eq!(call(ctx, "coll_remove", &[map, Value::int(i)]).as_bool(), Some(true));
        }
        assert!(capacity(ctx, map) < grown / 4, "10 entries no longer need {grown} buckets");
        for i in 0..10 {
            assert_eq!(call(ctx, "coll_get", &[map, Value::int(i)]).as_int(), Some(i * 2), "key {i} survived the shrinks");
        }
        assert_eq!(call(ctx, "len", &[map]).as_int(), Some(10));
    });
}

#[test]
fn removing_from_a_set_shrinks_its_table_and_keeps_the_rest() {
    with_heap(|ctx, _| {
        let set = call(ctx, "set_new", &[Value::int(0)]);
        for i in 0..300 {
            call(ctx, "set_add", &[set, Value::int(i)]);
        }
        let grown = capacity(ctx, set);
        for i in 10..300 {
            call(ctx, "coll_remove", &[set, Value::int(i)]);
        }
        assert!(capacity(ctx, set) < grown / 4);
        for i in 0..10 {
            assert_eq!(call(ctx, "coll_contains", &[set, Value::int(i)]).as_bool(), Some(true), "member {i} survived the shrinks");
        }
    });
}
