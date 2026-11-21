use super::tests::{code, lowered};

/// The blocks of `f` in `source` must be exactly `expected`, one line each, with nothing unsupported.
fn expect(source: &str, expected: &[&str]) {
    let (compiled, lowered) = lowered(source);
    assert_eq!(lowered.unsupported, []);
    assert_eq!(code(&compiled, &lowered, "f"), expected.join("\n"));
}

const RESULT_G: &str = "fn g() -> Result<Int, String> {\n return Ok(1)\n}\n";

#[test]
fn tuples_lists_arrays_and_maps_are_built_from_their_items_in_order() {
    expect("fn f(a: Int) -> (Int, String) {\n return (a, \"x\")\n}", &["  b0:", "    r1 = tuple r0, \"x\"", "    return r1"]);
    expect("fn f(a: Int) -> List<Int> {\n return [a, 2]\n}", &["  b0:", "    r1 = list r0, 2", "    return r1"]);
    expect("fn f() -> [Int; 3] {\n let a: [Int; 3] = [1, 2, 3]\n return a\n}", &["  b0:", "    r0 = array 1, 2, 3", "    r1 = copy r0", "    return r1"]);
    expect("fn f() -> [Int] {\n return [0; 8]\n}", &["  b0:", "    r0 = array_repeat 0, 8", "    return r0"]);
    expect("fn f() -> Map<String, Int> {\n return {\"a\": 1, \"b\": 2}\n}", &["  b0:", "    r0 = call f1()", "    call f2(r0, \"a\", 1) -> r1", "    call f2(r0, \"b\", 2) -> r2", "    return r0"]);
}

#[test]
fn a_range_expression_builds_a_range_value() {
    expect("import { Range } from std.collections\nfn f(n: Int) -> Range<Int> {\n return 0..n\n}", &["  b0:", "    r1 = range 0, r0", "    return r1"]);
}

#[test]
fn a_record_literal_lists_every_field_in_declaration_order() {
    expect("struct P {\n x: Int\n y: Int\n z: Int = 9\n}\nfn f(a: Int) -> P {\n return P { y: a, x: 1 }\n}", &["  b0:", "    r1 = record 1, r0, 9", "    return r1"]);
}

#[test]
fn a_record_update_reads_the_unlisted_fields_from_the_base() {
    expect("struct P {\n x: Int\n y: Int\n}\nfn f(p: P) -> P {\n return P { ..p, x: 5 }\n}", &["  b0:", "    r1 = field r0.1", "    r2 = record 5, r1", "    return r2"]);
}

#[test]
fn a_variant_lists_its_payload_in_declaration_order() {
    let source = "enum E {\n A(Int, String)\n B { n: Int, m: Int }\n C\n}\nfn f() -> E {\n return E.B { m: 2, n: 1 }\n}";
    expect(source, &["  b0:", "    r0 = variant 1 1, 2", "    return r0"]);
    expect("enum E {\n A(Int)\n C\n}\nfn f() -> E {\n return E.C\n}", &["  b0:", "    r0 = variant 1", "    return r0"]);
}

#[test]
fn slices_copy_a_range_and_an_assigned_slice_writes_the_list() {
    expect("fn f(xs: List<Int>, a: Int) -> List<Int> {\n return xs[a..]\n}", &["  b0:", "    r2 = slice r0[r1..]", "    return r2"]);
    expect("fn f(xs: List<Int>) -> List<Int> {\n return xs[0..8..2]\n}", &["  b0:", "    r1 = slice r0[0..8..2]", "    return r1"]);
    expect("fn f(var xs: List<Int>) {\n xs[1..3] = [7, 8]\n}", &["  b0:", "    r1 = copy r0", "    r2 = list 7, 8", "    set_slice r1[1..3], r2", "    return _"]);
    expect("fn f(var xs: List<Int>) {\n xs[1..3] = 0\n}", &["  b0:", "    r1 = copy r0", "    set_slice r1[1..3], 0", "    return _"]);
}

#[test]
fn an_index_assignment_reads_and_writes_the_same_element() {
    expect("fn f(var xs: List<Int>) {\n xs[0] += 1\n}", &["  b0:", "    r1 = index r0[0]", "    r2 = add r1, 1", "    set_index r0[0], r2", "    return _"]);
}

#[test]
fn optionals_unions_and_any_convert_with_one_op_each() {
    expect("fn f(a: Int) -> Int? {\n return a\n}", &["  b0:", "    r1 = to_optional r0", "    return r1"]);
    expect("fn f() -> Int? {\n return null\n}", &["  b0:", "    return null"]);
    expect("fn f(a: Int) -> (Int | String) {\n return a\n}", &["  b0:", "    r1 = to_union r0", "    return r1"]);
    expect("fn f() -> (Int8 | String) {\n return 5\n}", &["  b0:", "    r0 = copy 5", "    r1 = to_union r0", "    return r1"]);
    expect("fn f(a: (Int | String)) -> (Int | String | Bool) {\n return a\n}", &["  b0:", "    r1 = widen_union r0", "    return r1"]);
}

#[test]
fn a_narrowed_name_reads_the_payload() {
    let optional = "fn f(a: Int?) -> Int {\n if a != null { return a + 1 }\n return 0\n}";
    expect(optional, &["  b0:", "    r1 = ne r0, null", "    branch r1, b1, b2", "  b1:", "    r2 = some_payload r0", "    r3 = add r2, 1", "    return r3", "  b2:", "    return 0"]);
    let union = "fn f(a: (Int | String)) -> Int {\n if a is Int { return a + 1 }\n return 0\n}";
    expect(union, &["  b0:", "    r1 = is_type r0, Int", "    branch r1, b1, b2", "  b1:", "    r2 = union_payload r0, Int", "    r3 = add r2, 1", "    return r3", "  b2:", "    return 0"]);
}

#[test]
fn testing_a_value_against_its_own_type_is_true() {
    expect("fn f(a: Int?) -> Bool {\n return a is Int?\n}", &["  b0:", "    r1 = copy true", "    return r1"]);
}

#[test]
fn unwrap_faults_in_the_op_itself() {
    expect("fn f(a: Int?) -> Int {\n return a!\n}", &["  b0:", "    r1 = unwrap r0", "    return r1"]);
}

#[test]
fn propagating_an_optional_returns_none() {
    let source = "fn f(a: Int?) -> Int? {\n let n = a?\n return n + 1\n}";
    expect(source, &["  b0:", "    r1 = is_some r0", "    branch r1, b1, b2", "  b1:", "    r2 = some_payload r0", "    r3 = copy r2", "    r4 = add r3, 1", "    r5 = to_optional r4", "    return r5", "  b2:", "    return null"]);
}

#[test]
fn propagating_a_result_returns_the_error_in_the_function_s_result_type() {
    let same = format!("{RESULT_G}fn f() -> Result<Int, String> {{\n let n = g()?\n return Ok(n + 1)\n}}");
    expect(&same, &["  b0:", "    r0 = call f0()", "    r1 = is_variant r0, 0", "    branch r1, b1, b2", "  b1:", "    r4 = variant_field r0, 0.0", "    r5 = copy r4", "    r6 = add r5, 1", "    r7 = variant 0 r6", "    return r7", "  b2:", "    r2 = variant_field r0, 1.0", "    r3 = variant 1 r2", "    return r3"]);
    let union = format!("{RESULT_G}fn f() -> Result<Int, String | Bool> {{\n let n = g()?\n return Ok(n)\n}}");
    expect(&union, &["  b0:", "    r0 = call f0()", "    r1 = is_variant r0, 0", "    branch r1, b1, b2", "  b1:", "    r5 = variant_field r0, 0.0", "    r6 = copy r5", "    r7 = variant 0 r6", "    return r7", "  b2:", "    r2 = variant_field r0, 1.0", "    r3 = to_union r2", "    r4 = variant 1 r3", "    return r4"]);
}

#[test]
fn coalesce_assignment_wraps_the_new_value_as_an_optional() {
    expect("fn f(var a: Int?) {\n a ??= 5\n}", &["  b0:", "    r1 = copy r0", "    r2 = is_some r1", "    branch r2, b2, b1", "  b1:", "    r3 = to_optional 5", "    r0 = copy r3", "    jump b2", "  b2:", "    return _"]);
}
