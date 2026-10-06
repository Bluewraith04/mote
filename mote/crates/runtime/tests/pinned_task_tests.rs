//! Pinned tasks: `TASK_PIN_INTRINSIC` moves a task to the thread that called `run_main_parallel`.

use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

use isa::encoding::{encode_callintrinsic, encode_callnative, encode_newobj, encode_none, encode_r2, encode_ri, encode_scopeexit, encode_setfield};
use isa::opcode::Opcode::*;
use isa::value::{TypeDescriptor, Value};
use runtime::handlers::sched_intrinsics::{PIN_OFF, PIN_ON, PIN_QUERY, TASK_PIN_INTRINSIC};
use runtime::{CodeObject, NativeOutcome, Runtime};

type Seen = Arc<Mutex<Vec<(i64, ThreadId, bool)>>>;

fn spawn_closure(code_idx: u16, handle_reg: u8) -> Vec<isa::encoding::Instruction> {
    vec![
        encode_newobj(0, 0),
        encode_ri(LOADI, 1, code_idx),
        encode_setfield(0, 0, 1),
        encode_r2(SPAWN, handle_reg, 0),
    ]
}

/// Records `(tag, thread, on_home_thread)` for every call of native 0, with the tag in its first argument.
fn recorder(rt: &mut Runtime) -> Seen {
    let seen: Seen = Arc::default();
    let log = seen.clone();
    rt.set_native_dispatcher(Arc::new(move |_, ctx, args| {
        log.lock().unwrap().push((args[0].as_int().unwrap(), std::thread::current().id(), ctx.on_home_thread()));
        NativeOutcome::Done(Value::null())
    }));
    seen
}

fn pin_mode(mode: i64) -> Vec<isa::encoding::Instruction> {
    vec![encode_ri(LOADI, 1, mode as u16), encode_callintrinsic(0, TASK_PIN_INTRINSIC, 1)]
}

fn note(tag: u16) -> Vec<isa::encoding::Instruction> {
    vec![encode_ri(LOADI, 3, tag), encode_callnative(2, 0, 3)]
}

fn child(parts: Vec<Vec<isa::encoding::Instruction>>) -> CodeObject {
    let mut code: Vec<_> = parts.into_iter().flatten().collect();
    code.push(encode_none(HALT));
    CodeObject::new(code, vec![], 8, 0)
}

fn main_spawning(children: u16) -> CodeObject {
    let mut code = vec![encode_none(SCOPEENTER)];
    for i in 0..children {
        code.extend(spawn_closure(i + 1, 2 + i as u8));
    }
    code.push(encode_scopeexit(10));
    code.push(encode_none(HALT));
    CodeObject::new(code, vec![], 12, 0)
}

fn run(codes: Vec<CodeObject>, workers: usize) -> (Seen, Result<runtime::TaskContext, String>) {
    let mut rt = Runtime::with_types(codes, vec![TypeDescriptor::function_type(0)]);
    let seen = recorder(&mut rt);
    let main = rt.acquire_task(0, 12, None, 0);
    let result = rt.run_main_parallel(main, workers);
    (seen, result)
}

fn thread_of(seen: &Seen, tag: i64) -> (ThreadId, bool) {
    let seen = seen.lock().unwrap();
    let (_, thread, home) = seen.iter().find(|(t, _, _)| *t == tag).unwrap_or_else(|| panic!("native tagged {tag} never ran"));
    (*thread, *home)
}

#[test]
fn a_pinned_task_runs_on_the_calling_thread_and_an_unpinned_one_does_not() {
    let pinned = child(vec![pin_mode(PIN_ON), note(10), pin_mode(PIN_OFF), note(11)]);
    let free = child(vec![note(20)]);
    let (seen, result) = run(vec![main_spawning(2), pinned, free], 2);
    result.expect("the run finishes");
    let me = std::thread::current().id();
    assert_eq!(thread_of(&seen, 10), (me, true));
    assert!(!thread_of(&seen, 11).1);
    assert_ne!(thread_of(&seen, 11).0, me);
    assert!(!thread_of(&seen, 20).1);
    assert_ne!(thread_of(&seen, 20).0, me);
}

#[test]
fn a_pinned_task_stays_on_the_calling_thread_across_many_yields() {
    let mut spin = vec![encode_ri(LOADI, 4, 0), encode_ri(LOADI, 5, 1)];
    spin.extend((0..40_000).map(|_| encode_ri(LOADI, 6, 0)));
    let pinned = child(vec![pin_mode(PIN_ON), spin, note(10)]);
    let busy = child(vec![(0..40_000).map(|_| encode_ri(LOADI, 6, 0)).collect(), note(20)]);
    let (seen, result) = run(vec![main_spawning(3), pinned.clone(), pinned, busy], 2);
    result.expect("the run finishes");
    let me = std::thread::current().id();
    let tens: Vec<_> = seen.lock().unwrap().iter().filter(|(t, _, _)| *t == 10).map(|(_, th, h)| (*th, *h)).collect();
    assert_eq!(tens, vec![(me, true), (me, true)]);
}

#[test]
fn pinning_when_already_pinned_changes_nothing_and_the_query_reports_it() {
    let both = child(vec![pin_mode(PIN_ON), pin_mode(PIN_ON), pin_mode(PIN_QUERY), note(10)]);
    let (seen, result) = run(vec![main_spawning(1), both], 1);
    result.expect("the run finishes");
    assert_eq!(thread_of(&seen, 10), (std::thread::current().id(), true));
}

#[test]
fn main_can_pin_itself() {
    let mut code = pin_mode(PIN_ON);
    code.extend(note(10));
    code.push(encode_none(HALT));
    let (seen, result) = run(vec![CodeObject::new(code, vec![], 12, 0)], 2);
    result.expect("the run finishes");
    assert_eq!(thread_of(&seen, 10), (std::thread::current().id(), true));
}

#[test]
fn a_pinned_task_with_every_other_task_done_finishes_without_a_false_deadlock() {
    let pinned = child(vec![pin_mode(PIN_ON), note(10)]);
    for _ in 0..50 {
        let (_, result) = run(vec![main_spawning(1), pinned.clone()], 3);
        result.expect("no deadlock reported");
    }
}
