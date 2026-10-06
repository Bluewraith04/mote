//! Event sources end to end on the fake platform: a native opens a timer source,
//! a task receives its ticks, and the runtime closes it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use contracts::{EventPayload, NativeCtx, NativeFn, NativeOutcome, Overflow, SourceRequest};
use ffi::native_call::NativeFunctionRegistry;
use isa::encoding::*;
use isa::opcode::Opcode;
use isa::value::Value;
use platform::FakePlatform;
use runtime::handlers::sched_intrinsics::CHANNEL_RECV_INTRINSIC;
use runtime::{CodeObject, Runtime, TaskContext};

struct OpenTicker;

fn decode(_ctx: &mut dyn NativeCtx, payload: EventPayload) -> Result<Value, String> {
    match payload {
        EventPayload::Int(n) => Ok(Value::int(n)),
        other => Err(format!("unexpected payload {other:?}")),
    }
}

impl NativeFn for OpenTicker {
    fn call(&self, _cx: &mut dyn NativeCtx, _args: &[Value]) -> NativeOutcome {
        NativeOutcome::OpenSource {
            request: SourceRequest::Timer { period_nanos: 1_000 },
            capacity: 4,
            overflow: Overflow::Pause,
            decode,
        }
    }
}

fn run(code: Vec<u32>, drive: impl Fn(&FakePlatform) + Send + 'static) -> (Runtime, TaskContext, Arc<FakePlatform>) {
    let registry = NativeFunctionRegistry::new();
    registry.register_fn("open_ticker", Some(0), Arc::new(OpenTicker));
    let fake = Arc::new(FakePlatform::new(1));

    let mut rt = Runtime::new(vec![CodeObject::new(code, vec![], 10, 0)]);
    rt.set_native_resolver(Arc::new({
        let registry = registry.clone();
        move |name| registry.get_by_name(name)
    }));
    let dispatch = registry.clone();
    rt.set_native_dispatcher(Arc::new(move |id, ctx: &mut dyn NativeCtx, args| dispatch.call(id, args, ctx)));
    rt.set_native_table(&["open_ticker".to_string()]).unwrap();
    rt.set_platform(fake.clone());

    let done = Arc::new(AtomicBool::new(false));
    let driver = {
        let (fake, done) = (fake.clone(), done.clone());
        std::thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                drive(&fake);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        })
    };
    let task = rt.run_entry().unwrap();
    done.store(true, Ordering::SeqCst);
    driver.join().unwrap();
    (rt, task, fake)
}

fn recv() -> [u32; 2] {
    [encode_r2(Opcode::MOVE, 3, 0), encode_callintrinsic(5, CHANNEL_RECV_INTRINSIC, 3)]
}

#[test]
fn test_a_task_receives_timer_ticks_in_order() {
    let mut code = vec![encode_callnativew(0, 0), encode_ri(Opcode::LOADI, 7, 0)];
    for _ in 0..3 {
        code.extend(recv());
        code.push(encode_r3(Opcode::ADD, 7, 7, 5));
    }
    code.push(encode_r2(Opcode::MOVE, 0, 7));
    code.push(encode_r2(Opcode::RET, 0, 0));

    let (_rt, task, _) = run(code, |fake| fake.advance(1_000));
    assert_eq!(task.registers[0].as_int(), Some(1 + 2 + 3));
}

#[test]
fn test_an_ended_source_ends_the_channel_after_its_events() {
    let mut code = vec![encode_callnativew(0, 0)];
    code.extend(recv());
    code.extend(recv());
    code.push(encode_r2(Opcode::MOVE, 0, 4));
    code.push(encode_r2(Opcode::RET, 0, 0));

    let (_rt, task, _) = run(code, |fake| {
        if fake.sources_opened() == 1 {
            fake.emit(0, EventPayload::Int(9));
            fake.end_source(0);
        }
    });
    assert_eq!(task.registers[0].as_int(), Some(0));
}

#[test]
fn test_a_channel_dropped_at_exit_closes_its_source() {
    let mut code = vec![encode_callnativew(0, 0)];
    code.extend(recv());
    code.push(encode_r2(Opcode::RET, 0, 0));

    let (rt, task, fake) = run(code, |fake| fake.advance(1_000));
    drop(task);
    drop(rt);
    assert!(fake.source_closed(0));
}
