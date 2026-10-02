//! The stop-the-world handshake pauses every worker and scans every checked-out task's roots under a real collector.

use gc::gc::GCController;
use gc::plan::GCConfig;
use isa::encoding::{
    encode_ju, encode_jc, encode_newstr, encode_none, encode_r3, encode_ri, encode_scopeexit,
};
use isa::opcode::Opcode::*;
use isa::value::{TypeDescriptor, Value};
use runtime::{CodeObject, Runtime};

fn zero_globals(rt: &Runtime) {
    for g in rt.globals.lock().unwrap().iter_mut() {
        *g = Value::small_int(0);
    }
}

fn gc_churn_and_retain_loop(n: u16, gidx: u16) -> CodeObject {
    CodeObject::new(
        vec![
            encode_newstr(5, 0),
            encode_ri(LOADI, 0, 0),
            encode_ri(LOADI, 1, n),
            encode_ri(LOADI, 2, 1),
            encode_r3(LT, 3, 0, 1),
            encode_jc(JMPIFNOT, 3, 4),
            encode_newstr(4, 0),
            encode_r3(ADD, 0, 0, 2),
            encode_ju(JMP, -4),
            encode_r3(ADD, 6, 5, 5),
            encode_ri(SETGLOBAL, 6, gidx),
            encode_none(HALT),
        ],
        vec![],
        8,
        0,
    )
    .with_string_table(vec!["hello".into()])
}

fn spawn_closure(code_idx: u16, handle_reg: u8) -> Vec<isa::encoding::Instruction> {
    use isa::encoding::{encode_newobj, encode_r2, encode_setfield};
    vec![
        encode_newobj(0, 0),
        encode_ri(LOADI, 1, code_idx),
        encode_setfield(0, 0, 1),
        encode_r2(SPAWN, handle_reg, 0),
    ]
}

#[test]
fn stop_the_world_pause_survives_real_concurrent_collections_across_workers() {
    const N: usize = 8;
    const ITERS: u16 = 3_000;

    let mut instrs = vec![encode_none(SCOPEENTER)];
    for i in 0..N {
        instrs.extend(spawn_closure((i + 1) as u16, (2 + i) as u8));
    }
    let fault_reg = (2 + N) as u8;
    instrs.push(encode_scopeexit(fault_reg));
    instrs.push(encode_none(HALT));
    let main_code = CodeObject::new(instrs, vec![], (fault_reg + 1) as u16, 0);

    let mut code_objects = vec![main_code];
    for i in 0..N {
        code_objects.push(gc_churn_and_retain_loop(ITERS, i as u16));
    }

    let mut rt = Runtime::with_types(code_objects, vec![TypeDescriptor::function_type(0)]);
    rt.set_global_count(N);
    zero_globals(&rt);
    rt.set_heap(Box::new(GCController::new(
        GCConfig::mark_sweep().with_threshold(512),
    )));

    let main = rt.acquire_task(0, (fault_reg + 1) as usize, None, 0);
    let finished = rt
        .run_main_parallel(main, N)
        .expect("every child completes its loop without faulting, under a real installed GC");

    assert!(
        finished.registers[fault_reg as usize].is_null(),
        "no child faulted, so SCOPEEXIT reports null, not a fault message"
    );

    for i in 0..N {
        assert_eq!(
            rt.globals.lock().unwrap()[i].as_heap_string().as_deref(),
            Some("hellohello"),
            "child {i}'s retained root (r5) must have survived every concurrent collection \
             that ran while it — or some *other* worker — was mid-loop; a wrong or garbled \
             string here means the stop-the-world barrier missed a checked-out task's roots"
        );
    }

    let stats = rt.gc_stats().expect("a collector was installed");
    assert!(
        stats.collections > 1,
        "expected the 512-byte threshold + {N} workers' churn to force multiple real \
         collections during the run, got {}",
        stats.collections
    );
}
