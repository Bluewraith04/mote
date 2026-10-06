//! `Runtime::run_main_parallel` running hand-assembled tasks on several OS threads.

use isa::encoding::{
    encode_callintrinsic, encode_ju, encode_jc, encode_newobj, encode_none, encode_r2, encode_r3,
    encode_ri, encode_scopeexit, encode_setfield,
};
use isa::opcode::Opcode::*;
use isa::value::{TypeDescriptor, Value};
use runtime::handlers::sched_intrinsics::{
    SEAL_CLAIM, SEAL_IN_PLACE, SHARED_BEGIN_INTRINSIC, SHARED_COMMIT_INTRINSIC, SHARED_NEW_INTRINSIC, SHARED_SLOT_VERSION,
    TASK_CANCEL_INTRINSIC,
};
use runtime::{CodeObject, Runtime};

fn zero_globals(rt: &Runtime) {
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

fn spawn_closure(code_idx: u16, handle_reg: u8) -> Vec<isa::encoding::Instruction> {
    vec![
        encode_newobj(0, 0),
        encode_ri(LOADI, 1, code_idx),
        encode_setfield(0, 0, 1),
        encode_r2(SPAWN, handle_reg, 0),
    ]
}

#[test]
fn spawned_children_run_correctly_across_multiple_real_os_workers() {
    const N: usize = 8;
    const ITERS: u16 = 20_000;

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
        code_objects.push(incr_loop(ITERS, i as u16));
    }

    let mut rt = Runtime::with_types(code_objects, vec![TypeDescriptor::function_type(0)]);
    rt.set_global_count(N);
    zero_globals(&rt);

    let main = rt.acquire_task(0, (fault_reg + 1) as usize, None, 0);
    let finished = rt
        .run_main_parallel(main, 4)
        .expect("all children complete their loop without faulting");

    for i in 0..N {
        assert_eq!(
            rt.globals.lock().unwrap()[i].as_int(),
            Some(ITERS as i64),
            "global {i} (child {i}'s own counter) reached the full count"
        );
    }
    assert!(
        finished.registers[fault_reg as usize].is_null(),
        "no child faulted, so SCOPEEXIT reports null, not a fault message"
    );
}

#[test]
fn cancelling_a_task_running_on_a_different_real_worker_actually_stops_it() {
    const BIG: u16 = u16::MAX;
    let child_code = incr_loop(BIG, 0);

    let mut instrs = vec![encode_none(SCOPEENTER)];
    instrs.extend(spawn_closure(1, 2));
    instrs.push(encode_callintrinsic(3, TASK_CANCEL_INTRINSIC, 2));
    instrs.push(encode_scopeexit(4));
    instrs.push(encode_none(HALT));
    let main_code = CodeObject::new(instrs, vec![], 8, 0);

    let mut rt = Runtime::with_types(
        vec![main_code, child_code],
        vec![TypeDescriptor::function_type(0)],
    );
    rt.set_global_count(1);
    zero_globals(&rt);

    let main = rt.acquire_task(0, 8, None, 0);
    let finished = rt
        .run_main_parallel(main, 2)
        .expect("the cancelled child's fault is unobserved, but that's reported via r4, not Err");

    let fault = finished.registers[4]
        .as_heap_string()
        .expect("r4 holds the unobserved fault's message, not null");
    assert!(fault.contains("cancelled"), "expected a cancellation fault, got: {fault}");

    let progress = rt.globals.lock().unwrap()[0].as_int().unwrap_or(-1);
    assert!(
        progress < BIG as i64,
        "child should have been stopped well short of {BIG} iterations, got {progress}"
    );
}

fn cell_incr_loop(n: u16, cell_global_idx: u16) -> CodeObject {
    CodeObject::new(
        vec![
            encode_ri(LOADI, 0, 0),
            encode_ri(LOADI, 1, n),
            encode_ri(LOADI, 2, 1),
            encode_r3(LT, 3, 0, 1),
            encode_jc(JMPIFNOT, 3, 9),
            encode_ri(GETGLOBAL, 6, cell_global_idx),
            encode_ri(LOADI, 7, 1),
            encode_callintrinsic(8, SHARED_BEGIN_INTRINSIC, 6),
            encode_r3(ADD, 7, 8, 2),
            encode_ri(LOADI, 8, SEAL_CLAIM as u16),
            encode_callintrinsic(10, SHARED_COMMIT_INTRINSIC, 6),
            encode_r3(ADD, 0, 0, 2),
            encode_ju(JMP, -9),
            encode_none(HALT),
        ],
        vec![],
        12,
        0,
    )
}

#[test]
fn cell_writes_from_real_parallel_workers_lose_nothing() {
    const N: usize = 4;
    const ITERS: u16 = 5_000;

    let mut instrs = vec![
        encode_ri(LOADI, 0, 0),
        encode_ri(LOADI, 1, SEAL_IN_PLACE as u16),
        encode_callintrinsic(1, SHARED_NEW_INTRINSIC, 0),
        encode_ri(SETGLOBAL, 1, 0),
        encode_none(SCOPEENTER),
    ];
    for i in 0..N {
        instrs.extend(spawn_closure((i + 1) as u16, (2 + i) as u8));
    }
    let fault_reg = (2 + N) as u8;
    let handle_reg = fault_reg + 1;
    let result_reg = handle_reg + 1;
    instrs.push(encode_scopeexit(fault_reg));
    instrs.push(encode_ri(GETGLOBAL, handle_reg, 0));
    instrs.push(encode_r3(GETFIELD, result_reg, handle_reg, SHARED_SLOT_VERSION as u8));
    instrs.push(encode_none(HALT));
    let main_code = CodeObject::new(instrs, vec![], (result_reg + 1) as u16, 0);

    let mut code_objects = vec![main_code];
    for _ in 0..N {
        code_objects.push(cell_incr_loop(ITERS, 0));
    }

    let mut rt = Runtime::with_types(code_objects, vec![TypeDescriptor::function_type(0)]);
    rt.set_global_count(1);
    zero_globals(&rt);

    let main = rt.acquire_task(0, (result_reg + 1) as usize, None, 0);
    let finished = rt
        .run_main_parallel(main, N)
        .expect("every child completes its loop without faulting");

    assert_eq!(
        finished.registers[result_reg as usize].as_int(),
        Some((N as i64) * (ITERS as i64)),
        "every worker's increments should land"
    );
}
