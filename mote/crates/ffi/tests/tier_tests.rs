//! The runtime tiers share one native order; a program is attached to the smallest tier that has its natives.

use contracts::{FakeNativeCtx, NativeOutcome, NativeRegistry};
use ffi::builtins::{full_registry, lean_registry, registry, tier_of, TIER_FULL, TIER_GUI, TIER_LEAN};

fn table(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

#[test]
fn every_tier_gives_a_native_the_same_index() {
    let (gui, full, lean) = (registry(), full_registry(), lean_registry());
    for index in 0u16.. {
        let Some(entry) = gui.get(index) else { break };
        assert_eq!(full.get(index).unwrap().name, entry.name);
        assert_eq!(lean.get(index).unwrap().name, entry.name);
        assert_eq!(lean.resolve(&entry.name), Some(index));
    }
}

#[test]
fn a_program_needs_the_smallest_tier_with_its_natives() {
    assert_eq!(tier_of(&table(&["println", "len", "str_split"])), TIER_LEAN);
    assert_eq!(tier_of(&table(&["println", "net_tcp_connect_tls"])), TIER_FULL);
    assert_eq!(tier_of(&table(&["net_tcp_connect_tls", "tls_self_signed"])), TIER_FULL);
    assert_eq!(tier_of(&table(&["net_tcp_connect_tls", "ui_open"])), TIER_GUI);
    assert_eq!(tier_of(&[]), TIER_LEAN);
}

#[test]
fn a_native_above_the_tier_answers_an_error() {
    let lean = lean_registry();
    let call = |reg: &ffi::native_call::NativeFunctionRegistry, name: &str| {
        let id = reg.resolve(name).unwrap();
        reg.entry(id).unwrap().callable.call(&mut FakeNativeCtx::default(), &[])
    };
    for name in ["net_tcp_connect_tls", "tls_self_signed", "ui_open"] {
        assert!(matches!(call(&lean, name), NativeOutcome::Fail(_)), "{name}");
    }
    assert!(matches!(call(&full_registry(), "ui_open"), NativeOutcome::Fail(_)));
}
