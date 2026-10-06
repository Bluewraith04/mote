//! `EQ` / `NE` compare heap strings by content and `ADD` concatenates them, at the bytecode level.

use isa::encoding::*;
use isa::opcode::Opcode;
use runtime::{CodeObject, Runtime, TaskContext, VmStatus};

fn run_two_strings(op: Opcode, a: &str, b: &str) -> (Runtime, TaskContext) {
    let code = CodeObject::new(
        vec![
            encode_newstr(0, 0),
            encode_newstr(1, 1),
            encode_r3(op, 2, 0, 1),
            encode_r2(Opcode::RET, 2, 0),
        ],
        vec![],
        4,
        0,
    )
    .with_string_table(vec![a.into(), b.into()]);
    let mut rt = Runtime::new(vec![code]);
    let task = rt.run_entry().unwrap();
    assert_eq!(rt.status(), VmStatus::Halted);
    (rt, task)
}

#[test]
fn eq_compares_heap_strings_by_content_not_pointer() {
    let (_rt, task) = run_two_strings(Opcode::EQ, "hello", "hello");
    assert_eq!(task.registers[2].as_bool(), Some(true));

    let (_rt, task) = run_two_strings(Opcode::EQ, "hello", "world");
    assert_eq!(task.registers[2].as_bool(), Some(false));

    let (_rt, task) = run_two_strings(Opcode::NE, "hello", "hello");
    assert_eq!(task.registers[2].as_bool(), Some(false));
}

#[test]
fn add_concatenates_two_heap_strings() {
    let (_rt, task) = run_two_strings(Opcode::ADD, "foo", "bar");
    assert_eq!(task.registers[2].as_heap_string().as_deref(), Some("foobar"));

    let (_rt, task) = run_two_strings(Opcode::ADD, "", "x");
    assert_eq!(task.registers[2].as_heap_string().as_deref(), Some("x"));
}
