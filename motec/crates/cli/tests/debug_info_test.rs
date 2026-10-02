//! Span tables and source files ride in the compiled program, and `--release` strips them.

use compiler::{CompiledProgram, Compiler};

const SOURCE: &str = "fn add(a: Int, b: Int) -> Int {\n    return a + b\n}\nprintln(add(1, 2))\n";

#[test]
fn debug_builds_carry_spans_and_release_builds_do_not() {
    compiler::span::set_release(false);
    let debug = Compiler::compile(SOURCE, "sum.mote").unwrap();
    let file = debug.sources.iter().find(|s| s.path == "sum.mote").expect("the program source is carried");
    assert_eq!(file.text, SOURCE);

    let add = debug.code_objects.iter().find(|c| c.spans.iter().any(|s| s.line == 2 && s.len == 5)).expect("a span for `a + b`");
    let span = add.spans.iter().find(|s| s.line == 2 && s.len == 5).unwrap();
    assert_eq!(&SOURCE[span.offset as usize..(span.offset + span.len) as usize], "a + b");
    let pc = span.start_pc as usize;
    assert_eq!(add.span_at(pc).map(|s| s.len), Some(1), "the innermost span at the operand load is the operand");
    let op_pc = (span.end_pc - 1) as usize;
    assert_eq!(add.span_at(op_pc).map(|s| s.len), Some(5), "the operator instruction maps to the whole expression");

    let round = CompiledProgram::from_bytes(&debug.to_bytes()).unwrap();
    assert_eq!(round.sources, debug.sources);
    assert_eq!(round.code_objects[0].spans, debug.code_objects[0].spans);

    compiler::span::set_release(true);
    let release = Compiler::compile(SOURCE, "sum.mote").unwrap();
    compiler::span::set_release(false);
    assert!(release.sources.is_empty());
    assert!(release.code_objects.iter().all(|c| c.spans.is_empty()));
}
