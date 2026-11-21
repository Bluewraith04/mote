use super::tests::{code, lowered};

/// The blocks of `f` in `source`, which must lower with nothing unsupported.
fn blocks(source: &str) -> String {
    let (compiled, lowered) = lowered(source);
    assert_eq!(lowered.unsupported, []);
    code(&compiled, &lowered, "f")
}

#[test]
fn an_if_chain_joins_its_branches() {
    let text = blocks("fn f(a: Int) -> Int {\n var r = 0\n if a < 3 { r = 1 } else if a < 5 { r = 2 } else { r = 3 }\n return r\n}");
    let expected = "  b0:\n    r1 = copy 0\n    r2 = lt r0, 3\n    branch r2, b1, b3\n  b1:\n    r1 = copy 1\n    jump b2\n  b2:\n    r4 = copy r1\n    return r4\n  b3:\n    r3 = lt r0, 5\n    branch r3, b4, b6\n  b4:\n    r1 = copy 2\n    jump b5\n  b5:\n    jump b2\n  b6:\n    r1 = copy 3\n    jump b5";
    assert_eq!(text, expected);
}

#[test]
fn a_branch_that_returns_does_not_jump_to_the_join() {
    assert_eq!(blocks("fn f(a: Bool) -> Int {\n if a { return 1 }\n return 2\n}"), "  b0:\n    branch r0, b1, b2\n  b1:\n    return 1\n  b2:\n    return 2");
    assert_eq!(blocks("fn f(a: Bool) -> Int {\n if a { return 1 } else { return 2 }\n}"), "  b0:\n    branch r0, b1, b3\n  b1:\n    return 1\n  b2:\n    unreachable\n  b3:\n    return 2");
}

#[test]
fn and_or_and_not_become_branches() {
    let text = blocks("fn f(a: Bool, b: Bool, c: Bool) -> Int {\n if a && (b || !c) { return 1 }\n return 2\n}");
    assert_eq!(text, "  b0:\n    branch r0, b3, b2\n  b1:\n    return 1\n  b2:\n    return 2\n  b3:\n    branch r1, b1, b4\n  b4:\n    branch r2, b2, b1");
}

#[test]
fn and_as_a_value_sets_a_register_on_each_path() {
    let text = blocks("fn f(a: Bool, b: Bool) -> Bool {\n let x = a && b\n return x\n}");
    assert_eq!(text, "  b0:\n    branch r0, b4, b2\n  b1:\n    r2 = copy true\n    jump b3\n  b2:\n    r2 = copy false\n    jump b3\n  b3:\n    r3 = copy r2\n    return r3\n  b4:\n    branch r1, b1, b2");
}

#[test]
fn an_if_value_copies_each_branch_into_one_register() {
    let text = blocks("fn f(a: Bool) -> Int {\n let x = if a { 1 } else { 2 }\n return x\n}");
    assert_eq!(text, "  b0:\n    branch r0, b1, b2\n  b1:\n    r1 = copy 1\n    jump b3\n  b2:\n    r1 = copy 2\n    jump b3\n  b3:\n    r2 = copy r1\n    return r2");
}

#[test]
fn break_and_continue_jump_to_the_exit_and_the_header() {
    let text = blocks("fn f(n: Int) -> Int {\n var i = 0\n var s = 0\n while i < n {\n i += 1\n if i == 3 { continue }\n if i > 8 { break }\n s += i\n }\n return s\n}");
    let expected = "  b0:\n    r1 = copy 0\n    r2 = copy 0\n    jump b1\n  b1:\n    r3 = copy r1\n    r4 = lt r3, r0\n    branch r4, b2, b3\n  b2:\n    r5 = copy r1\n    r6 = add r5, 1\n    r1 = copy r6\n    r7 = copy r1\n    r8 = eq r7, 3\n    branch r8, b4, b5\n  b3:\n    r14 = copy r2\n    return r14\n  b4:\n    jump b1\n  b5:\n    r9 = copy r1\n    r10 = gt r9, 8\n    branch r10, b6, b7\n  b6:\n    jump b3\n  b7:\n    r11 = copy r2\n    r12 = copy r1\n    r13 = add r11, r12\n    r2 = copy r13\n    jump b1";
    assert_eq!(text, expected);
}

#[test]
fn a_range_loop_counts_a_register_and_checks_the_end() {
    let text = blocks("fn f(n: Int) -> Int {\n var s = 0\n for i in 0..n { s += i }\n return s\n}");
    let expected = "  b0:\n    r1 = copy 0\n    r2 = copy 0\n    r3 = copy r0\n    jump b1\n  b1:\n    r4 = lt r2, r3\n    branch r4, b2, b4\n  b2:\n    r5 = copy r2\n    r6 = copy r1\n    r7 = add r6, r5\n    r1 = copy r7\n    jump b3\n  b3:\n    r2 = add r2, 1\n    jump b1\n  b4:\n    r8 = copy r1\n    return r8";
    assert_eq!(text, expected);
}

#[test]
fn an_inclusive_range_stops_at_the_end_without_adding_past_it() {
    let text = blocks("fn f(n: Int) -> Int {\n var s = 0\n for i in 1..=n { s += i }\n return s\n}");
    let expected = "  b0:\n    r1 = copy 0\n    r2 = copy 1\n    r3 = copy r0\n    jump b1\n  b1:\n    r4 = le r2, r3\n    branch r4, b2, b4\n  b2:\n    r5 = copy r2\n    r6 = copy r1\n    r7 = add r6, r5\n    r1 = copy r7\n    jump b3\n  b3:\n    r8 = eq r2, r3\n    branch r8, b4, b5\n  b4:\n    r9 = copy r1\n    return r9\n  b5:\n    r2 = add r2, 1\n    jump b1";
    assert_eq!(text, expected);
}

#[test]
fn a_range_held_in_a_value_reads_its_inclusive_flag() {
    let text = blocks("import { Range } from std.collections\nfn f(r: Range<Int>) -> Int {\n var s = 0\n for i in r { s += i }\n return s\n}");
    let expected = "  b0:\n    r1 = copy 0\n    r2 = field r0.0\n    r3 = field r0.1\n    r4 = field r0.2\n    r5 = copy r2\n    r6 = copy r3\n    jump b1\n  b1:\n    r7 = lt r5, r6\n    r8 = eq r5, r6\n    r9 = and r4, r8\n    r10 = or r7, r9\n    branch r10, b2, b4\n  b2:\n    r11 = copy r5\n    r12 = copy r1\n    r13 = add r12, r11\n    r1 = copy r13\n    jump b3\n  b3:\n    r14 = eq r5, r6\n    r15 = and r4, r14\n    branch r15, b4, b5\n  b4:\n    r16 = copy r1\n    return r16\n  b5:\n    r5 = add r5, 1\n    jump b1";
    assert_eq!(text, expected);
}

#[test]
fn a_list_loop_reads_the_length_each_time_round() {
    let text = blocks("fn f(xs: List<Int>) -> Int {\n var s = 0\n for x in xs { s += x }\n return s\n}");
    let expected = "  b0:\n    r1 = copy 0\n    r2 = copy r0\n    r3 = copy 0\n    jump b1\n  b1:\n    r4 = len r2\n    r5 = lt r3, r4\n    branch r5, b2, b4\n  b2:\n    r6 = index r2[r3]\n    r7 = copy r6\n    r8 = copy r1\n    r9 = add r8, r7\n    r1 = copy r9\n    jump b3\n  b3:\n    r3 = add r3, 1\n    jump b1\n  b4:\n    r10 = copy r1\n    return r10";
    assert_eq!(text, expected);
}

#[test]
fn a_tuple_pattern_in_a_for_takes_the_elements_apart() {
    let text = blocks("fn f(xs: List<(Int, Int)>) -> Int {\n var s = 0\n for (a, b) in xs { s += a * b }\n return s\n}");
    assert!(text.contains("    r6 = index r2[r3]\n    r7 = tuple_field r6.0\n    r8 = copy r7\n    r9 = tuple_field r6.1\n    r10 = copy r9\n"), "{text}");
}

#[test]
fn a_stream_and_a_receiver_loop_call_their_next_intrinsic() {
    let stream = blocks("fn f(xs: Stream<Int>) -> Int {\n var s = 0\n for x in xs { s += x }\n return s\n}");
    assert!(stream.contains("  b1:\n    r4 = call intrinsic stream_next(r2) -> r3\n    r5 = is_some r4\n    branch r5, b2, b3\n  b2:\n    r6 = some_payload r4\n    r7 = copy r6\n"), "{stream}");
    let receiver = blocks("fn f(xs: Receiver<Int>) -> Int {\n var s = 0\n for x in xs { s += x }\n return s\n}");
    assert!(receiver.contains("  b1:\n    r3 = call intrinsic receiver_recv(r2)\n    r4 = is_some r3\n    branch r4, b2, b3\n  b2:\n    r5 = some_payload r3\n    r6 = copy r5\n"), "{receiver}");
}

#[test]
fn coalesce_assignment_writes_only_when_the_place_is_none() {
    let text = blocks("fn f(var a: Int?, b: Int?) {\n a ??= b\n}");
    assert_eq!(text, "  b0:\n    r2 = copy r0\n    r3 = is_some r2\n    branch r3, b2, b1\n  b1:\n    r0 = copy r1\n    jump b2\n  b2:\n    return _");
}
