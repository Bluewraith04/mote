//! A Rust panic inside an opcode handler fails the task instead of hanging the run.

use std::sync::mpsc;
use std::time::Duration;

use isa::encoding::{
    encode_newobj, encode_none, encode_r2, encode_r3, encode_ri, encode_scopeexit, encode_setfield,
};
use isa::opcode::Opcode::*;
use isa::value::TypeDescriptor;
use runtime::{CodeObject, Runtime};

fn panicking_code() -> CodeObject {
    CodeObject::new(vec![encode_r3(ADD, 1, 2, 3), encode_none(HALT)], vec![], 8, 0)
}

fn run_with_timeout(codes: Vec<CodeObject>, workers: usize) -> Result<bool, String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let rt = Runtime::with_types(codes, vec![TypeDescriptor::function_type(0)]);
        let main = rt.acquire_task(0, 8, None, 0);
        let r = rt
            .run_main_parallel(main, workers)
            .map(|t| !t.registers[3].is_null());
        let _ = tx.send(r);
    });
    rx.recv_timeout(Duration::from_secs(20))
        .expect("run_main_parallel hung after a worker panic")
}

#[test]
fn panic_in_main_task_is_reported_as_an_error() {
    for workers in [1, 2] {
        let err = run_with_timeout(vec![panicking_code()], workers).unwrap_err();
        assert!(err.contains("internal panic"), "workers={workers}: {err}");
        assert!(err.contains("Unexpected Types in ADD"), "workers={workers}: {err}");
    }
}

#[test]
fn panic_in_a_child_faults_the_child_and_does_not_hang_the_scope() {
    let main_code = CodeObject::new(
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
    for workers in [1, 2] {
        let fault_surfaced = run_with_timeout(vec![main_code.clone(), panicking_code()], workers)
            .unwrap_or_else(|e| panic!("workers={workers}: {e}"));
        assert!(fault_surfaced, "workers={workers}: the child's fault never reached SCOPEEXIT");
    }
}
