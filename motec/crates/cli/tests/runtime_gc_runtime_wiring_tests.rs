//! The `Runtime` and `GCController` seam: `NEWOBJ` allocating through the collector, `safepoint_poll` driving collection and `SETFIELD` reaching the write barrier.

use gc::gc::GCController;
use gc::plan::GCConfig;
use isa::encoding::*;
use isa::opcode::Opcode;
use isa::value::{TypeDescriptor, Value};
use runtime::{CodeObject, Runtime, VmStatus};

fn allocating_loop(count: u16) -> CodeObject {
    CodeObject::new(
        vec![
            encode_ri(Opcode::LOADI, 0, 0),
            encode_ri(Opcode::LOADI, 1, count),
            encode_r3(Opcode::GE, 2, 0, 1),
            encode_jc(Opcode::JMPIF, 2, 7),
            encode_ri(Opcode::NEWOBJ, 3, 0),
            encode_ri(Opcode::LOADI, 4, 1),
            encode_r3(Opcode::ADD, 0, 0, 4),
            encode_r3(Opcode::SETFIELD, 3, 0, 0),
            encode_r3(Opcode::SETFIELD, 3, 1, 4),
            encode_ju(Opcode::JMP, -7),
            encode_none(Opcode::HALT),
        ],
        vec![],
        8,
        0,
    )
}

fn one_two_field_type() -> Vec<TypeDescriptor> {
    vec![TypeDescriptor::new(0, vec![Some("a".into()), Some("b".into())])]
}

#[test]
fn newobj_routes_through_the_collector_and_safepoints_reclaim() {
    let mut rt = Runtime::with_types(vec![allocating_loop(400)], one_two_field_type());
    rt.set_heap(Box::new(GCController::new(
        GCConfig::mark_sweep().with_threshold(512),
    )));

    let task = rt.run_entry().unwrap();
    assert_eq!(rt.status(), VmStatus::Halted);
    assert_eq!(task.registers[0].as_int(), Some(400));

    let s = rt.gc_stats().expect("a GC is installed");
    assert!(s.collections >= 10, "expected many collections, got {s:?}");
    assert!(s.bytes_freed > 0, "expected reclaimed bytes, got {s:?}");
    assert!(
        s.bytes_allocated >= 400 * (std::mem::size_of::<Value>() * 2),
        "not all allocations routed through the GC: {s:?}"
    );
    assert!(
        s.live_objects <= 24,
        "live set grows with total allocations — reclamation is not happening: {s:?}"
    );
}

#[test]
fn newstr_allocates_through_the_gc_and_strings_are_sized_and_reclaimed() {
    let code = CodeObject::new(
        vec![
            encode_ri(Opcode::LOADI, 0, 0),
            encode_ri(Opcode::LOADI, 1, 300),
            encode_r3(Opcode::GE, 2, 0, 1),
            encode_jc(Opcode::JMPIF, 2, 6),
            encode_newstr(3, 0),
            encode_ri(Opcode::NEWOBJ, 4, 0),
            encode_ri(Opcode::LOADI, 5, 1),
            encode_r3(Opcode::ADD, 0, 0, 5),
            encode_ju(Opcode::JMP, -6),
            encode_none(Opcode::HALT),
        ],
        vec![],
        8,
        0,
    )
    .with_string_table(vec!["a moderately long throwaway string, well over 7 bytes".into()]);

    let mut rt = Runtime::with_types(vec![code], one_two_field_type());
    rt.set_heap(Box::new(GCController::new(
        GCConfig::mark_sweep().with_threshold(512),
    )));

    let task = rt.run_entry().unwrap();
    assert_eq!(rt.status(), VmStatus::Halted);
    assert_eq!(task.registers[0].as_int(), Some(300));

    let s = rt.gc_stats().expect("a GC is installed");
    assert!(s.collections >= 5, "no collections ran: {s:?}");
    assert!(s.bytes_freed > 0, "strings + objects were never reclaimed: {s:?}");
    assert!(s.live_objects <= 32, "garbage strings not reclaimed: {s:?}");
}

#[test]
fn newstr_content_survives_a_gc_cycle() {
    let text = "the-survivor-string-payload-value";
    let code = CodeObject::new(
        vec![
            encode_newstr(6, 0),
            encode_ri(Opcode::LOADI, 0, 0),
            encode_ri(Opcode::LOADI, 1, 250),
            encode_r3(Opcode::GE, 2, 0, 1),
            encode_jc(Opcode::JMPIF, 2, 6),
            encode_newstr(3, 1),
            encode_ri(Opcode::NEWOBJ, 4, 0),
            encode_ri(Opcode::LOADI, 5, 1),
            encode_r3(Opcode::ADD, 0, 0, 5),
            encode_ju(Opcode::JMP, -6),
            encode_none(Opcode::HALT),
        ],
        vec![],
        8,
        0,
    )
    .with_string_table(vec![text.into(), "different garbage string contents here".into()]);

    let mut rt = Runtime::with_types(vec![code], one_two_field_type());
    rt.set_heap(Box::new(GCController::new(
        GCConfig::mark_sweep().with_threshold(256),
    )));
    let task = rt.run_entry().unwrap();
    assert_eq!(rt.status(), VmStatus::Halted);

    assert!(rt.gc_stats().unwrap().collections >= 5);
    assert_eq!(task.registers[6].as_heap_string().as_deref(), Some(text));
}

#[test]
fn without_a_gc_newobj_falls_back_to_the_bump_allocator() {
    let mut rt = Runtime::with_types(vec![allocating_loop(64)], one_two_field_type());
    let task = rt.run_entry().unwrap();
    assert_eq!(rt.status(), VmStatus::Halted);
    assert_eq!(task.registers[0].as_int(), Some(64));
    assert!(rt.gc_stats().is_none());
}

#[test]
fn a_reachable_chain_survives_collection() {
    let code = CodeObject::new(
        vec![
            encode_ri(Opcode::NEWOBJ, 5, 0),
            encode_ri(Opcode::NEWOBJ, 6, 0),
            encode_r3(Opcode::SETFIELD, 5, 0, 6),
            encode_ri(Opcode::LOADI, 0, 0),
            encode_ri(Opcode::LOADI, 1, 200),
            encode_r3(Opcode::GE, 2, 0, 1),
            encode_jc(Opcode::JMPIF, 2, 6),
            encode_ri(Opcode::NEWOBJ, 7, 0),
            encode_ri(Opcode::LOADI, 4, 1),
            encode_r3(Opcode::ADD, 0, 0, 4),
            encode_r2(Opcode::MOVE, 7, 5),
            encode_ju(Opcode::JMP, -6),
            encode_none(Opcode::HALT),
        ],
        vec![],
        8,
        0,
    );
    let ty = vec![TypeDescriptor::with_pointer_mask(
        0,
        vec![Some("next".into()), Some("scalar".into())],
        0b01,
    )];

    let mut rt = Runtime::with_types(vec![code], ty);
    rt.set_heap(Box::new(GCController::new(
        GCConfig::mark_sweep().with_threshold(256),
    )));
    let task = rt.run_entry().unwrap();
    assert_eq!(rt.status(), VmStatus::Halted);

    let s = rt.gc_stats().unwrap();
    assert!(s.collections >= 5, "no collections ran: {s:?}");

    let obj_a = task.registers[5].as_object_ptr().expect("r5 still holds obj_a");
    let obj_b = task.registers[6].as_object_ptr().expect("r6 still holds obj_b");
    let linked = unsafe { obj_a.as_ref().get_field(0) };
    assert_eq!(
        linked.as_object_ptr(),
        Some(obj_b),
        "the obj_a -> obj_b link did not survive collection"
    );
    assert!(s.live_objects <= 20, "garbage not reclaimed: {s:?}");
}
