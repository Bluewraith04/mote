//! Instruction-counted safepoints: straight-line bytecode with no backward jump still reaches a poll.

use gc::gc::GCController;
use gc::plan::GCConfig;
use isa::encoding::{
    encode_callintrinsic, encode_jc, encode_ju, encode_newobj, encode_newstr, encode_none, encode_r2,
    encode_r3, encode_ri, encode_scopeexit, encode_setfield,
};
use isa::opcode::Opcode::*;
use isa::value::{TypeDescriptor, Value};
use runtime::handlers::sched_intrinsics::TASK_CANCEL_INTRINSIC;
use runtime::{CodeObject, Runtime, SAFEPOINT_INSTRUCTION_INTERVAL};

fn zero_globals(rt: &Runtime) {
    for g in rt.globals.lock().unwrap().iter_mut() {
        *g = Value::small_int(0);
    }
}

fn spawn_closure(code_idx: u16, handle_reg: u8) -> Vec<isa::encoding::Instruction> {
    vec![
        encode_newobj(0, 0),
        encode_ri(LOADI, 1, code_idx),
        encode_setfield(0, 0, 1),
        encode_r2(SPAWN, handle_reg, 0),
    ]
}

fn straight_line_filler(count: u32) -> Vec<isa::encoding::Instruction> {
    (0..count).map(|_| encode_ri(LOADI, 0, 0)).collect()
}

#[test]
fn cancellation_reaches_a_task_with_no_loop_and_no_call() {
    const FILLER: u32 = SAFEPOINT_INSTRUCTION_INTERVAL * 50;

    let mut child_instrs = straight_line_filler(FILLER);
    child_instrs.push(encode_none(HALT));
    let child_code = CodeObject::new(child_instrs, vec![], 8, 0);

    let mut instrs = vec![encode_none(SCOPEENTER)];
    instrs.extend(spawn_closure(1, 2));
    instrs.push(encode_callintrinsic(3, TASK_CANCEL_INTRINSIC, 2));
    instrs.push(encode_scopeexit(4));
    instrs.push(encode_none(HALT));
    let main_code = CodeObject::new(instrs, vec![], 8, 0);

    let rt = Runtime::with_types(
        vec![main_code, child_code],
        vec![TypeDescriptor::function_type(0)],
    );

    let main = rt.acquire_task(0, 8, None, 0);
    let finished = rt
        .run_main_parallel(main, 2)
        .expect("the cancelled child's fault is unobserved, but that's reported via r4, not Err");

    let fault = finished.registers[4]
        .as_heap_string()
        .expect(
            "r4 holds the unobserved fault's message, not null — if this is null, the child \
             ran to completion uninterrupted, meaning the instruction-counted safepoint never \
             fired inside a loop-free, call-free code object",
        );
    assert!(fault.contains("cancelled"), "expected a cancellation fault, got: {fault}");
}

fn gc_churn_and_retain_straight_line(count: u32, gidx: u16) -> CodeObject {
    let mut instrs = vec![encode_newstr(5, 0)];
    instrs.extend((0..count).map(|_| encode_newstr(4, 0)));
    instrs.push(encode_r3(ADD, 6, 5, 5));
    instrs.push(encode_ri(SETGLOBAL, 6, gidx));
    instrs.push(encode_none(HALT));
    CodeObject::new(instrs, vec![], 8, 0).with_string_table(vec!["hello".into()])
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

#[test]
fn a_real_gc_pause_correctly_scans_a_task_with_no_loop_and_no_call() {
    const N_LOOPERS: usize = 4;
    const LOOP_ITERS: u16 = 3_000;
    const STRAIGHT_LINE_CHURN: u32 = SAFEPOINT_INSTRUCTION_INTERVAL * 20;

    let straight_line_idx = 1u16;
    let mut instrs = vec![encode_none(SCOPEENTER)];
    instrs.extend(spawn_closure(straight_line_idx, 2));
    for i in 0..N_LOOPERS {
        instrs.extend(spawn_closure(straight_line_idx + 1 + i as u16, (3 + i) as u8));
    }
    let fault_reg = (3 + N_LOOPERS) as u8;
    instrs.push(encode_scopeexit(fault_reg));
    instrs.push(encode_none(HALT));
    let main_code = CodeObject::new(instrs, vec![], (fault_reg + 1) as u16, 0);

    let mut code_objects = vec![main_code];
    code_objects.push(gc_churn_and_retain_straight_line(STRAIGHT_LINE_CHURN, 0));
    for i in 0..N_LOOPERS {
        code_objects.push(gc_churn_and_retain_loop(LOOP_ITERS, (i + 1) as u16));
    }

    let mut rt = Runtime::with_types(code_objects, vec![TypeDescriptor::function_type(0)]);
    rt.set_global_count(N_LOOPERS + 1);
    zero_globals(&rt);
    rt.set_heap(Box::new(GCController::new(
        GCConfig::mark_sweep().with_threshold(512),
    )));

    let main = rt.acquire_task(0, (fault_reg + 1) as usize, None, 0);
    let finished = rt
        .run_main_parallel(main, N_LOOPERS + 1)
        .expect("every child completes without faulting, under a real installed GC");

    assert!(
        finished.registers[fault_reg as usize].is_null(),
        "no child faulted, so SCOPEEXIT reports null, not a fault message"
    );

    assert_eq!(
        rt.globals.lock().unwrap()[0].as_heap_string().as_deref(),
        Some("hellohello"),
        "the loop-free, call-free child's retained root (r5) must have survived every \
         concurrent collection that ran during its unrolled churn — a wrong or garbled string \
         here means a pause landed without ever seeing this task check in, i.e. the \
         instruction-counted safepoint didn't fire"
    );
    for i in 0..N_LOOPERS {
        assert_eq!(
            rt.globals.lock().unwrap()[i + 1].as_heap_string().as_deref(),
            Some("hellohello"),
            "looping child {i}'s retained root must also have survived"
        );
    }

    let stats = rt.gc_stats().expect("a collector was installed");
    assert!(
        stats.collections > 1,
        "expected the 512-byte threshold + churn from {} children to force multiple real \
         collections, got {}",
        N_LOOPERS + 1,
        stats.collections
    );
}
