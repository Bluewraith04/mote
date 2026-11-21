use super::tests::{code, lowered};

/// The blocks of `f` in `source` must be exactly `expected`, one line each, with nothing unsupported.
fn expect(source: &str, expected: &[&str]) {
    let (compiled, lowered) = lowered(source);
    assert_eq!(lowered.unsupported, []);
    assert_eq!(code(&compiled, &lowered, "f"), expected.join("\n"));
}

#[test]
fn a_match_on_an_enum_switches_on_the_variant_tag() {
    let source = "enum E {\n A(Int)\n B\n C(Int, Int)\n}\nfn f(e: E) -> Int {\n match e {\n A(n) => { return n }\n B => { return 0 }\n C(a, b) => { return a + b }\n }\n}";
    expect(
        source,
        &[
            "  b0:",
            "    r1 = variant_tag r0",
            "    switch r1 [0 => b2, 1 => b3, 2 => b4] else b5",
            "  b1:",
            "    unreachable",
            "  b2:",
            "    r2 = is_variant r0, 0",
            "    branch r2, b6, b3",
            "  b3:",
            "    r5 = is_variant r0, 1",
            "    branch r5, b7, b4",
            "  b4:",
            "    r6 = is_variant r0, 2",
            "    branch r6, b8, b5",
            "  b5:",
            "    unreachable",
            "  b6:",
            "    r3 = variant_field r0, 0.0",
            "    r4 = copy r3",
            "    return r4",
            "  b7:",
            "    return 0",
            "  b8:",
            "    r7 = variant_field r0, 2.0",
            "    r8 = copy r7",
            "    r9 = variant_field r0, 2.1",
            "    r10 = copy r9",
            "    r11 = add r8, r10",
            "    return r11",
        ],
    );
}

#[test]
fn a_match_value_is_copied_into_one_register_by_every_arm() {
    let source = "enum E {\n A(Int)\n B\n}\nfn f(e: E) -> Int {\n let n = match e {\n A(x) => x + 1\n B => 0\n }\n return n\n}";
    expect(
        source,
        &[
            "  b0:",
            "    r2 = variant_tag r0",
            "    switch r2 [0 => b2, 1 => b3] else b4",
            "  b1:",
            "    r8 = copy r1",
            "    return r8",
            "  b2:",
            "    r3 = is_variant r0, 0",
            "    branch r3, b5, b3",
            "  b3:",
            "    r7 = is_variant r0, 1",
            "    branch r7, b6, b4",
            "  b4:",
            "    unreachable",
            "  b5:",
            "    r4 = variant_field r0, 0.0",
            "    r5 = copy r4",
            "    r6 = add r5, 1",
            "    r1 = copy r6",
            "    jump b1",
            "  b6:",
            "    r1 = copy 0",
            "    jump b1",
        ],
    );
}

#[test]
fn a_guard_that_fails_falls_through_to_the_next_arm_and_a_catch_all_is_the_switch_default() {
    let source = "enum E {\n A(Int)\n B\n C\n}\nfn f(e: E) -> Int {\n return match e {\n A(x) if x > 3 => 1\n C => 2\n _ => 3\n }\n}";
    expect(
        source,
        &[
            "  b0:",
            "    r2 = variant_tag r0",
            "    switch r2 [0 => b2, 2 => b3] else b4",
            "  b1:",
            "    return r1",
            "  b2:",
            "    r3 = is_variant r0, 0",
            "    branch r3, b6, b3",
            "  b3:",
            "    r7 = is_variant r0, 2",
            "    branch r7, b8, b4",
            "  b4:",
            "    r1 = copy 3",
            "    jump b1",
            "  b5:",
            "    unreachable",
            "  b6:",
            "    r4 = variant_field r0, 0.0",
            "    r5 = copy r4",
            "    r6 = gt r5, 3",
            "    branch r6, b7, b3",
            "  b7:",
            "    r1 = copy 1",
            "    jump b1",
            "  b8:",
            "    r1 = copy 2",
            "    jump b1",
        ],
    );
}

#[test]
fn literals_ranges_and_or_patterns_test_in_order_without_a_switch() {
    let source = "fn f(n: Int) -> String {\n return match n {\n 0 => \"zero\"\n 1..=9 => \"small\"\n 10 | 20 => \"round\"\n _ => \"big\"\n }\n}";
    expect(
        source,
        &[
            "  b0:",
            "    jump b2",
            "  b1:",
            "    return r1",
            "  b2:",
            "    r2 = eq r0, 0",
            "    branch r2, b7, b3",
            "  b3:",
            "    r3 = ge r0, 1",
            "    branch r3, b8, b4",
            "  b4:",
            "    r5 = eq r0, 10",
            "    branch r5, b12, b11",
            "  b5:",
            "    r1 = copy \"big\"",
            "    jump b1",
            "  b6:",
            "    unreachable",
            "  b7:",
            "    r1 = copy \"zero\"",
            "    jump b1",
            "  b8:",
            "    r4 = le r0, 9",
            "    branch r4, b9, b4",
            "  b9:",
            "    r1 = copy \"small\"",
            "    jump b1",
            "  b10:",
            "    r1 = copy \"round\"",
            "    jump b1",
            "  b11:",
            "    r6 = eq r0, 20",
            "    branch r6, b13, b5",
            "  b12:",
            "    jump b10",
            "  b13:",
            "    jump b10",
        ],
    );
}

#[test]
fn an_at_pattern_binds_the_value_then_tests_the_inner_pattern() {
    let source = "fn f(n: Int) -> Int {\n return match n {\n k @ 1..=5 => k\n _ => 0\n }\n}";
    expect(
        source,
        &[
            "  b0:",
            "    jump b2",
            "  b1:",
            "    return r1",
            "  b2:",
            "    r2 = copy r0",
            "    r3 = ge r0, 1",
            "    branch r3, b5, b3",
            "  b3:",
            "    r1 = copy 0",
            "    jump b1",
            "  b4:",
            "    unreachable",
            "  b5:",
            "    r4 = le r0, 5",
            "    branch r4, b6, b3",
            "  b6:",
            "    r1 = copy r2",
            "    jump b1",
        ],
    );
}

#[test]
fn optional_patterns_test_is_some_and_read_the_payload() {
    let source = "fn f(a: Int?) -> Int {\n return match a {\n Some(n) => n\n None => 0\n }\n}";
    expect(
        source,
        &[
            "  b0:",
            "    jump b2",
            "  b1:",
            "    return r1",
            "  b2:",
            "    r2 = is_some r0",
            "    branch r2, b5, b3",
            "  b3:",
            "    r5 = is_some r0",
            "    branch r5, b4, b6",
            "  b4:",
            "    unreachable",
            "  b5:",
            "    r3 = some_payload r0",
            "    r4 = copy r3",
            "    r1 = copy r4",
            "    jump b1",
            "  b6:",
            "    r1 = copy 0",
            "    jump b1",
        ],
    );
}

#[test]
fn a_tuple_pattern_reads_only_the_parts_it_tests_or_binds() {
    let source = "fn f(p: (Int, Bool)) -> Int {\n match p {\n (0, true) => { return 1 }\n (n, _) => { return n }\n }\n}";
    expect(
        source,
        &[
            "  b0:",
            "    jump b2",
            "  b1:",
            "    unreachable",
            "  b2:",
            "    r1 = tuple_field r0.0",
            "    r2 = eq r1, 0",
            "    branch r2, b5, b3",
            "  b3:",
            "    r5 = tuple_field r0.0",
            "    r6 = copy r5",
            "    return r6",
            "  b4:",
            "    unreachable",
            "  b5:",
            "    r3 = tuple_field r0.1",
            "    r4 = eq r3, true",
            "    branch r4, b6, b3",
            "  b6:",
            "    return 1",
        ],
    );
}

#[test]
fn a_type_pattern_tests_the_member_and_reads_its_payload() {
    let source = "fn f(a: (Int | String)) -> Int {\n return match a {\n n: Int => n\n s: String => 0\n }\n}";
    expect(
        source,
        &[
            "  b0:",
            "    jump b2",
            "  b1:",
            "    return r1",
            "  b2:",
            "    r2 = is_type r0, Int",
            "    branch r2, b5, b3",
            "  b3:",
            "    r5 = is_type r0, String",
            "    branch r5, b6, b4",
            "  b4:",
            "    unreachable",
            "  b5:",
            "    r3 = union_payload r0, Int",
            "    r4 = copy r3",
            "    r1 = copy r4",
            "    jump b1",
            "  b6:",
            "    r6 = union_payload r0, String",
            "    r7 = copy r6",
            "    r1 = copy 0",
            "    jump b1",
        ],
    );
}

#[test]
fn a_result_match_switches_on_ok_and_err() {
    let source = "fn f(r: Result<Int, String>) -> Int {\n return match r {\n Ok(n) => n\n Err(_) => 0\n }\n}";
    expect(
        source,
        &[
            "  b0:",
            "    r2 = variant_tag r0",
            "    switch r2 [0 => b2, 1 => b3] else b4",
            "  b1:",
            "    return r1",
            "  b2:",
            "    r3 = is_variant r0, 0",
            "    branch r3, b5, b3",
            "  b3:",
            "    r6 = is_variant r0, 1",
            "    branch r6, b6, b4",
            "  b4:",
            "    unreachable",
            "  b5:",
            "    r4 = variant_field r0, 0.0",
            "    r5 = copy r4",
            "    r1 = copy r5",
            "    jump b1",
            "  b6:",
            "    r1 = copy 0",
            "    jump b1",
        ],
    );
}

#[test]
fn if_let_binds_in_the_then_branch_and_falls_to_the_else() {
    let source = "fn f(a: Int?) -> Int {\n if let Some(n) = a { return n }\n return 0\n}";
    expect(
        source,
        &[
            "  b0:",
            "    r1 = is_some r0",
            "    branch r1, b3, b2",
            "  b1:",
            "    return r3",
            "  b2:",
            "    return 0",
            "  b3:",
            "    r2 = some_payload r0",
            "    r3 = copy r2",
            "    jump b1",
        ],
    );
}

#[test]
fn while_let_matches_each_round_and_exits_on_a_mismatch() {
    let source = "fn f(var s: Stream<Int>) -> Int {\n var t = 0\n while let Some(n) = s.next() { t += n }\n return t\n}";
    expect(
        source,
        &[
            "  b0:",
            "    r1 = copy 0",
            "    jump b1",
            "  b1:",
            "    r2 = copy r0",
            "    r4 = call f1(r2) -> r3",
            "    r0 = copy r3",
            "    r5 = is_some r4",
            "    branch r5, b4, b3",
            "  b2:",
            "    r8 = copy r1",
            "    r9 = add r8, r7",
            "    r1 = copy r9",
            "    jump b1",
            "  b3:",
            "    r10 = copy r1",
            "    return r10",
            "  b4:",
            "    r6 = some_payload r4",
            "    r7 = copy r6",
            "    jump b2",
        ],
    );
}

#[test]
fn a_let_with_a_record_pattern_reads_each_field() {
    let source = "struct P {\n x: Int\n y: Int\n}\nfn f(p: P) -> Int {\n let P { x, y } = p\n return x + y\n}";
    expect(
        source,
        &[
            "  b0:",
            "    r1 = field r0.0",
            "    r2 = copy r1",
            "    r3 = field r0.1",
            "    r4 = copy r3",
            "    r5 = add r2, r4",
            "    return r5",
            "  b1:",
            "    unreachable",
        ],
    );
}
