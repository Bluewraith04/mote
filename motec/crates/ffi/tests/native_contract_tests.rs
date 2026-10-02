//! Conformance of the builtin registry to the `NativeFn` contract.

use contracts::{
    FakeNativeCtx, NativeOutcome, NativeRegistry, Platform, PlatformError, PlatformRequest, PlatformResponse,
};
use ffi::builtins::registry;
use isa::value::Value;

struct AcceptAll;

impl Platform for AcceptAll {
    fn execute(&self, _: PlatformRequest) -> Result<PlatformResponse, PlatformError> {
        Ok(PlatformResponse::Unit)
    }
    fn blocking(&self, _: &PlatformRequest) -> bool {
        false
    }
}

fn call(name: &str, args: &[Value]) -> NativeOutcome {
    let reg = registry();
    let id = reg.resolve(name).unwrap_or_else(|| panic!("{name} not registered"));
    reg.entry(id).unwrap().callable.call(&mut FakeNativeCtx::with_platform(AcceptAll), args)
}
#[test]
fn names_resolve_to_entries() {
    let reg = registry();
    let id = reg.resolve("int_min").unwrap();
    assert_eq!(reg.entry(id).unwrap().name, "int_min");
    assert!(reg.resolve("no_such_native").is_none());
    assert!(reg.entry(u16::MAX).is_none());
}

#[test]
fn plain_native_is_done() {
    match call("int_min", &[Value::int(3), Value::int(2)]) {
        NativeOutcome::Done(v) => assert_eq!(v.as_int(), Some(2)),
        _ => panic!("expected Done"),
    }
}

#[test]
fn exit_native_is_exit() {
    assert!(matches!(call("process_exit", &[Value::int(7)]), NativeOutcome::Exit(7)));
}

#[test]
fn platform_native_names_its_request() {
    match call("time_sleep_nanos", &[Value::int(5)]) {
        NativeOutcome::Platform(PlatformRequest::Sleep { .. }, _) => {}
        _ => panic!("expected a Sleep request"),
    }
}

#[test]
fn faulting_native_is_fail() {
    match registry().call(u16::MAX, &[], &mut FakeNativeCtx::new()) {
        NativeOutcome::Fail(e) => assert_eq!(e.kind, 0),
        _ => panic!("expected Fail"),
    }
}
