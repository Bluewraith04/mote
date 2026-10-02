//! The cooperative scheduler, exercised with hand-built `TaskContext`s.

use gc::gc::GCController;
use gc::plan::GCConfig;
use isa::encoding::{
    encode_jc, encode_ju, encode_newobj, encode_none, encode_r2, encode_r3, encode_ri,
    encode_scopeexit, encode_setfield,
};
use isa::opcode::Opcode::*;
use isa::value::{TypeDescriptor, Value};
use runtime::{CodeObject, Runtime};

fn zero_globals(rt: &mut Runtime) {
    for g in rt.globals.lock().unwrap().iter_mut() {
        *g = Value::small_int(0);
    }
}

fn incr_loop(n: u16, gidx: u16) -> CodeObject {
    CodeObject::new(
        vec![
            encode_ri(LOADI, 0, 0),
            encode_ri(LOADI, 1, n),
            encode_ri(LOADI, 2, 1),
            encode_r3(LT, 3, 0, 1),
            encode_jc(JMPIFNOT, 3, 6),
            encode_ri(GETGLOBAL, 4, gidx),
            encode_r3(ADD, 4, 4, 2),
            encode_ri(SETGLOBAL, 4, gidx),
            encode_r3(ADD, 0, 0, 2),
            encode_ju(JMP, -6),
            encode_none(HALT),
        ],
        vec![],
        8,
        0,
    )
}

#[test]
fn two_tasks_both_run_to_completion_and_are_recycled() {
    let mut rt = Runtime::new(vec![incr_loop(5000, 0), incr_loop(5000, 1)]);
    rt.set_global_count(2);
    zero_globals(&mut rt);

    let a = rt.acquire_task(0, 8, None, 0);
    let b = rt.acquire_task(1, 8, None, 0);
    rt.schedule(a);
    rt.schedule(b);

    rt.run_scheduled().unwrap();

    assert_eq!(rt.globals.lock().unwrap()[0].as_int(), Some(5000), "task A finished its loop");
    assert_eq!(rt.globals.lock().unwrap()[1].as_int(), Some(5000), "task B finished its loop");
    assert!(
        rt.context_switches() > 0,
        "5000 back-edges each vs a 2000 budget — the scheduler must have round-robined"
    );
    assert_eq!(rt.pooled_tasks(), 2, "both finished contexts recycled");
}

#[test]
fn a_lone_task_never_yields() {
    let mut rt = Runtime::new(vec![incr_loop(3000, 0)]);
    rt.set_global_count(1);
    zero_globals(&mut rt);

    let t = rt.acquire_task(0, 8, None, 0);
    rt.schedule(t);
    rt.run_scheduled().unwrap();

    assert_eq!(rt.globals.lock().unwrap()[0].as_int(), Some(3000));
    assert_eq!(
        rt.context_switches(),
        0,
        "empty run queue during its safepoints — budget never spent"
    );
    assert_eq!(rt.pooled_tasks(), 1);
}

#[test]
fn the_free_pool_is_reused_and_task_ids_advance() {
    let mut rt = Runtime::new(vec![incr_loop(100, 0)]);
    rt.set_global_count(1);
    zero_globals(&mut rt);

    let t1 = rt.acquire_task(0, 8, None, 0);
    assert_eq!(t1.task_id, 1);
    rt.schedule(t1);
    rt.run_scheduled().unwrap();
    assert_eq!(rt.pooled_tasks(), 1);

    let t2 = rt.acquire_task(0, 8, None, 0);
    assert_eq!(t2.task_id, 2);
    assert_eq!(rt.pooled_tasks(), 0);
    assert!(t2.registers.capacity() >= 16, "recycled register file kept its capacity");

    rt.schedule(t2);
    rt.run_scheduled().unwrap();
    assert_eq!(rt.globals.lock().unwrap()[0].as_int(), Some(200), "second run added another 100");
    assert_eq!(rt.pooled_tasks(), 1);
}

#[test]
fn the_free_pool_is_capped_and_recycled_registers_shrink() {
    let mut rt = Runtime::new(vec![incr_loop(10, 0)]);
    rt.set_global_count(1);
    zero_globals(&mut rt);

    for _ in 0..300 {
        let mut t = rt.acquire_task(0, 8, None, 0);
        t.registers.resize(100_000, Value::null());
        rt.schedule(t);
    }
    rt.run_scheduled().unwrap();

    assert_eq!(rt.pooled_tasks(), 256, "the pool keeps at most 256 contexts");
    let t = rt.acquire_task(0, 8, None, 0);
    assert!(t.registers.capacity() <= 4096, "a recycled register file keeps at most 4096 slots");
}

#[test]
fn gc_scans_the_registers_of_queued_tasks() {
    let churn = CodeObject::new(
        vec![
            encode_ri(LOADI, 0, 0),
            encode_ri(LOADI, 1, 4000),
            encode_ri(LOADI, 2, 1),
            encode_r3(LT, 3, 0, 1),
            encode_jc(JMPIFNOT, 3, 5),
            isa::encoding::encode_newstr(4, 0),
            encode_ri(SETGLOBAL, 4, 0),
            encode_r3(ADD, 0, 0, 2),
            encode_ju(JMP, -5),
            encode_none(HALT),
        ],
        vec![],
        8,
        0,
    )
    .with_string_table(vec!["hello".into()]);

    let holder = CodeObject::new(
        vec![
            isa::encoding::encode_newstr(5, 0),
            encode_ri(LOADI, 0, 0),
            encode_ri(LOADI, 1, 4000),
            encode_ri(LOADI, 2, 1),
            encode_r3(LT, 3, 0, 1),
            encode_jc(JMPIFNOT, 3, 6),
            encode_ri(GETGLOBAL, 4, 1),
            encode_r3(ADD, 4, 4, 2),
            encode_ri(SETGLOBAL, 4, 1),
            encode_r3(ADD, 0, 0, 2),
            encode_ju(JMP, -6),
            encode_r3(ADD, 6, 5, 5),
            encode_ri(SETGLOBAL, 6, 2),
            encode_none(HALT),
        ],
        vec![],
        10,
        0,
    )
    .with_string_table(vec!["hello".into()]);

    let mut rt = Runtime::new(vec![churn, holder]);
    rt.set_global_count(3);
    zero_globals(&mut rt);
    rt.set_heap(Box::new(GCController::new(
        GCConfig::mark_sweep().with_threshold(4096),
    )));

    let a = rt.acquire_task(0, 8, None, 0);
    let b = rt.acquire_task(1, 10, None, 0);
    rt.schedule(a);
    rt.schedule(b);
    rt.run_scheduled().unwrap();

    assert_eq!(rt.globals.lock().unwrap()[1].as_int(), Some(4000), "holder finished its loop");
    assert_eq!(
        rt.globals.lock().unwrap()[2].as_heap_string().as_deref(),
        Some("hellohello"),
        "r5 stayed live through collections while the task was queued"
    );
    assert!(rt.gc_stats().unwrap().collections > 0, "the collector actually ran");
}

#[test]
fn a_child_fault_surfaces_at_the_scope_boundary() {
    let parent = CodeObject::new(
        vec![
            encode_none(SCOPEENTER),
            encode_newobj(0, 0),
            encode_ri(LOADI, 1, 1),
            encode_setfield(0, 0, 1),
            encode_r2(SPAWN, 2, 0),
            encode_scopeexit(3),
            encode_none(HALT),
        ],
        vec![],
        8,
        0,
    );
    let child = CodeObject::new(
        vec![
            encode_ri(LOADI, 0, 5),
            encode_r3(CALLV, 1, 0, 0),
            encode_none(HALT),
        ],
        vec![],
        4,
        0,
    );

    let mut rt = Runtime::with_types(vec![parent, child], vec![TypeDescriptor::function_type(0)]);
    let main = rt.acquire_task(0, 8, None, 0);
    let main = rt.run_main(main).expect("SCOPEEXIT reports the fault via r3, not an Err");
    let msg = main.registers[3]
        .as_heap_string()
        .expect("r3 holds the unobserved fault's message, not null");
    assert!(msg.contains("not a callable value"), "carries the child's error: {msg}");
}
