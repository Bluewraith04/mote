//! Hand-assembled generator programs: resume/yield/done, calls inside a body, the
//! running-generator fault, and cancellation observed inside a body.

use std::sync::atomic::Ordering;
use std::time::Duration;

use isa::encoding::*;
use isa::opcode::Opcode::*;

use crate::{CodeObject, Runtime};

fn run(code: Vec<CodeObject>) -> Result<crate::TaskContext, String> {
    Runtime::new(code).run_entry()
}

fn count_program(body_yield: Vec<u32>) -> Vec<CodeObject> {
    let mut gen_code = vec![
        encode_none(MKGEN),
        encode_ri(LOADI, 1, 0),
        encode_r3(LT, 2, 1, 0),
        encode_jc(JMPIFNOT, 2, 5 + (body_yield.len() as i16 - 1)),
    ];
    gen_code.extend(body_yield);
    gen_code.push(encode_ri(LOADI, 3, 1));
    gen_code.push(encode_r3(ADD, 1, 1, 3));
    let back = -(gen_code.len() as i32 - 2);
    gen_code.push(encode_ju(JMP, back));
    gen_code.push(encode_ret(0));

    let main = vec![
        encode_ri(LOADI, 2, 3),
        encode_call(1, 1),
        encode_ri(LOADI, 4, 0),
        encode_r3(RESUME, 2, 1, 3),
        encode_jc(JMPIFNOT, 3, 3),
        encode_r3(ADD, 4, 4, 2),
        encode_ju(JMP, -3),
        encode_r3(RESUME, 2, 1, 5),
        encode_ri(LOADI, 6, 1000),
        encode_r3(MUL, 5, 5, 6),
        encode_r3(ADD, 4, 4, 5),
        encode_ret(4),
    ];
    let double = vec![encode_r3(ADD, 1, 0, 0), encode_ret(1)];
    vec![
        CodeObject::new(main, vec![], 8, 0),
        CodeObject::new(gen_code, vec![], 8, 1),
        CodeObject::new(double, vec![], 3, 1),
    ]
}

#[test]
fn test_resume_runs_a_body_to_each_yield_then_reports_done() {
    let task = run(count_program(vec![encode_r2(YIELD, 1, 0)])).unwrap();
    assert_eq!(task.registers[0].as_int(), Some(3));
}

#[test]
fn test_a_body_can_call_functions_between_yields() {
    let body = vec![encode_r2(MOVE, 6, 1), encode_call(5, 2), encode_r2(YIELD, 5, 0)];
    let task = run(count_program(body)).unwrap();
    assert_eq!(task.registers[0].as_int(), Some(6));
}

#[test]
fn test_resuming_a_running_generator_is_a_fault() {
    let gen_code = vec![
        encode_none(MKGEN),
        encode_getglobal(1, 0),
        encode_r3(RESUME, 2, 1, 3),
        encode_ret(0),
    ];
    let main = vec![
        encode_call(1, 1),
        encode_setglobal(1, 0),
        encode_r3(RESUME, 2, 1, 3),
        encode_ret(2),
    ];
    let result = run(vec![
        CodeObject::new(main, vec![], 6, 0),
        CodeObject::new(gen_code, vec![], 6, 0),
    ]);
    let err = result.unwrap_err();
    assert!(err.contains("already running"), "got: {err}");
}

#[test]
fn test_resume_on_a_non_generator_is_a_fault() {
    let main = vec![encode_ri(LOADI, 1, 7), encode_r3(RESUME, 2, 1, 3), encode_ret(2)];
    let err = run(vec![CodeObject::new(main, vec![], 6, 0)]).unwrap_err();
    assert!(err.contains("not a generator"), "got: {err}");
}

#[test]
fn test_yield_outside_a_generator_body_is_a_fault() {
    let main = vec![encode_ri(LOADI, 1, 7), encode_r2(YIELD, 1, 0), encode_ret(1)];
    let err = run(vec![CodeObject::new(main, vec![], 4, 0)]).unwrap_err();
    assert!(err.contains("outside a generator"), "got: {err}");
}

struct CancelFlag(std::sync::Arc<crate::sched::TaskShared>);
unsafe impl Send for CancelFlag {}

#[test]
fn test_cancellation_is_observed_inside_a_generator_body() {
    let gen_code = vec![encode_none(MKGEN), encode_none(NOP), encode_ju(JMP, -1)];
    let main = vec![encode_call(0, 1), encode_r3(RESUME, 1, 0, 2), encode_ret(1)];
    let (flag_tx, flag_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut rt = Runtime::new(vec![
            CodeObject::new(main, vec![], 4, 0),
            CodeObject::new(gen_code, vec![], 4, 0),
        ]);
        let task = crate::TaskContext::entry(&rt);
        flag_tx.send(CancelFlag(task.shared.clone())).unwrap();
        done_tx.send(rt.run_task(task).map(|_| ())).unwrap();
    });
    let flag = flag_rx.recv().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    flag.0.cancelled.store(true, Ordering::SeqCst);
    let result = done_rx.recv_timeout(Duration::from_secs(30)).expect("the cancelled body stops at its safepoint");
    assert!(result.unwrap_err().contains("cancelled"));
}
