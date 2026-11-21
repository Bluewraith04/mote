use mote_semantics::Compiled;

use super::tests::lowered;
use crate::{print_function, verify, FunctionKind, Lowered};

/// `f` and every closure, as their headers and blocks without the register types.
fn text(compiled: &Compiled, lowered: &Lowered) -> String {
    assert_eq!(verify(&lowered.module, &compiled.program, &compiled.interner), []);
    let mut out = Vec::new();
    for (id, function) in lowered.module.functions.iter() {
        let named = matches!(&function.kind, FunctionKind::Instance(instance) if compiled.interner.resolve(compiled.program.defs[instance.def].name) == "f");
        if named || matches!(function.kind, FunctionKind::Closure { .. } | FunctionKind::Native(_)) {
            out.push(format!("f{}:", id.index()));
            out.extend(print_function(function, &compiled.program, &compiled.interner).lines().filter(|line| !line.starts_with("  r")).map(str::to_string));
        }
    }
    out.join("\n")
}

/// The functions `f` leads to must be exactly `expected`, one line each, with nothing unsupported.
fn expect(source: &str, expected: &[&str]) {
    let (compiled, lowered) = lowered(source);
    assert_eq!(lowered.unsupported, []);
    assert_eq!(text(&compiled, &lowered), expected.join("\n"));
}

#[test]
fn a_lambda_is_its_own_function_and_the_value_carries_its_captures() {
    expect(
        "fn f() -> Int {\n let g = |n: Int| n + 1\n return g(2)\n}",
        &[
            "f0:",
            "fn instance f() -> Int",
            "  b0:",
            "    r0 = closure f1 []",
            "    r1 = copy r0",
            "    r2 = call value r1(2)",
            "    return r2",
            "f1:",
            "fn closure of f0(r0) -> Int",
            "  b0:",
            "    r1 = add r0, 1",
            "    return r1",
        ],
    );
    expect(
        "fn f(a: Int) -> Int {\n let g = |n: Int| n + a\n return g(2)\n}",
        &[
            "f0:",
            "fn instance f(r0) -> Int",
            "  b0:",
            "    r1 = closure f1 [r0]",
            "    r2 = copy r1",
            "    r3 = call value r2(2)",
            "    return r3",
            "f1:",
            "fn closure of f0(r1) -> Int",
            "  captures r0",
            "  b0:",
            "    r2 = add r1, r0",
            "    return r2",
        ],
    );
}

#[test]
fn a_captured_var_lives_in_a_cell_both_sides_share() {
    expect(
        "fn f() -> Int {\n var c = 0\n let inc = || { c += 1 }\n inc()\n inc()\n return c\n}",
        &[
            "f0:",
            "fn instance f() -> Int",
            "  b0:",
            "    r0 = new_cell 0",
            "    r1 = closure f1 [r0]",
            "    r2 = copy r1",
            "    call value r2()",
            "    call value r2()",
            "    r3 = load_cell r0",
            "    return r3",
            "f1:",
            "fn closure of f0() -> Null",
            "  captures r0",
            "  b0:",
            "    r1 = load_cell r0",
            "    r2 = add r1, 1",
            "    store_cell r0, r2",
            "    return _",
        ],
    );
}

#[test]
fn an_inner_closure_gets_the_cell_from_the_closure_around_it() {
    expect(
        "fn f() -> Int {\n var c = 0\n let a = || {\n let b = || { c += 1 }\n b()\n }\n a()\n return c\n}",
        &[
            "f0:",
            "fn instance f() -> Int",
            "  b0:",
            "    r0 = new_cell 0",
            "    r1 = closure f1 [r0]",
            "    r2 = copy r1",
            "    call value r2()",
            "    r3 = load_cell r0",
            "    return r3",
            "f1:",
            "fn closure of f0() -> Null",
            "  captures r0",
            "  b0:",
            "    r1 = closure f2 [r0]",
            "    r2 = copy r1",
            "    call value r2()",
            "    return _",
            "f2:",
            "fn closure of f1() -> Null",
            "  captures r0",
            "  b0:",
            "    r1 = load_cell r0",
            "    r2 = add r1, 1",
            "    store_cell r0, r2",
            "    return _",
        ],
    );
}

#[test]
fn a_var_made_in_a_loop_gets_a_new_cell_each_round() {
    expect(
        "fn f() {\n for i in 0..3 {\n var c = i\n let g = || { c += 1 }\n g()\n }\n}",
        &[
            "f0:",
            "fn instance f() -> Null",
            "  b0:",
            "    r0 = copy 0",
            "    r1 = copy 3",
            "    jump b1",
            "  b1:",
            "    r2 = lt r0, r1",
            "    branch r2, b2, b4",
            "  b2:",
            "    r3 = copy r0",
            "    r4 = new_cell r3",
            "    r5 = closure f1 [r4]",
            "    r6 = copy r5",
            "    call value r6()",
            "    jump b3",
            "  b3:",
            "    r0 = add r0, 1",
            "    jump b1",
            "  b4:",
            "    return _",
            "f1:",
            "fn closure of f0() -> Null",
            "  captures r0",
            "  b0:",
            "    r1 = load_cell r0",
            "    r2 = add r1, 1",
            "    store_cell r0, r2",
            "    return _",
        ],
    );
}

#[test]
fn a_captured_var_parameter_moves_into_a_cell_and_comes_back_on_return() {
    expect(
        "fn f(var n: Int) {\n let g = || { n += 1 }\n g()\n}",
        &[
            "f0:",
            "fn instance f(r0) -> Null",
            "  b0:",
            "    r1 = new_cell r0",
            "    r2 = closure f1 [r1]",
            "    r3 = copy r2",
            "    call value r3()",
            "    r0 = load_cell r1",
            "    return _",
            "f1:",
            "fn closure of f0() -> Null",
            "  captures r0",
            "  b0:",
            "    r1 = load_cell r0",
            "    r2 = add r1, 1",
            "    store_cell r0, r2",
            "    return _",
        ],
    );
}

#[test]
fn a_nested_fn_that_calls_itself_reads_itself_from_a_cell_filled_after_it_exists() {
    expect(
        "fn f(k: Int) -> Int {\n fn fact(n: Int) -> Int {\n if n <= 1 { return k }\n return n * fact(n - 1)\n }\n return fact(4)\n}",
        &[
            "f0:",
            "fn instance f(r0) -> Int",
            "  b0:",
            "    r1 = new_cell null",
            "    r2 = closure f1 [r0, r1]",
            "    store_cell r1, r2",
            "    r3 = load_cell r1",
            "    r4 = call value r3(4)",
            "    return r4",
            "f1:",
            "fn closure of f0(r2) -> Int",
            "  captures r0, r1",
            "  b0:",
            "    r3 = le r2, 1",
            "    branch r3, b1, b2",
            "  b1:",
            "    return r0",
            "  b2:",
            "    r4 = load_cell r1",
            "    r5 = sub r2, 1",
            "    r6 = call value r4(r5)",
            "    r7 = mul r2, r6",
            "    return r7",
        ],
    );
}

#[test]
fn a_named_function_as_a_value_is_a_closure_without_captures() {
    expect(
        "fn double(n: Int) -> Int { return n * 2 }\nfn f() -> Int {\n let g: (Int) -> Int = double\n return g(4)\n}",
        &["f1:", "fn instance f() -> Int", "  b0:", "    r0 = closure f0 []", "    r1 = copy r0", "    r2 = call value r1(4)", "    return r2"],
    );
}

#[test]
fn a_native_as_a_value_goes_through_a_function_that_calls_it() {
    expect(
        "import { sqrt } from std.math\nimport { sleep } from std.task\nfn f() -> Float {\n let s: (Int) -> Null = sleep\n let g: (Float) -> Float = sqrt\n let h: (Float) -> Float = sqrt\n return g(4.0)\n}",
        &[
            "f0:",
            "fn instance f() -> Float",
            "  b0:",
            "    r0 = closure f1 []",
            "    r1 = copy r0",
            "    r2 = closure f2 []",
            "    r3 = copy r2",
            "    r4 = closure f2 []",
            "    r5 = copy r4",
            "    r6 = call value r3(4.0)",
            "    return r6",
            "f1:",
            "fn native sleep(r0) -> Null",
            "  b0:",
            "    call intrinsic sleep(r0)",
            "    return _",
            "f2:",
            "fn native sqrt(r0) -> Float",
            "  b0:",
            "    r1 = call intrinsic math_sqrt(r0)",
            "    return r1",
        ],
    );
}

#[test]
fn a_var_argument_to_a_function_value_is_copied_in_and_the_result_written_back() {
    expect(
        "fn bump(var n: Int) { n += 1 }\nfn f() -> Int {\n var x = 1\n let h: (var Int) -> Null = bump\n h(x)\n return x\n}",
        &[
            "f1:",
            "fn instance f() -> Int",
            "  b0:",
            "    r0 = copy 1",
            "    r1 = closure f0 []",
            "    r2 = copy r1",
            "    r3 = copy r0",
            "    call value r2(r3) -> r4",
            "    r0 = copy r4",
            "    r5 = copy r0",
            "    return r5",
        ],
    );
}

#[test]
fn a_function_fitting_a_type_by_conversion_is_wrapped() {
    expect(
        "fn one(n: Int?) -> Int { return 1 }\nfn f() -> Int? {\n let b: (Int) -> Int? = one\n return b(1)\n}",
        &[
            "f1:",
            "fn instance f() -> Int?",
            "  b0:",
            "    r0 = closure f0 []",
            "    r1 = closure f2 [r0]",
            "    r2 = copy r1",
            "    r3 = call value r2(1)",
            "    return r3",
            "f2:",
            "fn closure of f1(r1) -> Int?",
            "  captures r0",
            "  b0:",
            "    r2 = to_optional r1",
            "    r3 = call value r0(r2)",
            "    r4 = to_optional r3",
            "    return r4",
        ],
    );
}

#[test]
fn a_wrapper_passes_a_var_parameter_through_and_drops_an_unwanted_result() {
    expect(
        "fn bump(var n: Int) -> Int {\n n += 1\n return n\n}\nfn f() {\n var x = 0\n let h: (var Int) -> Null = bump\n h(x)\n}",
        &[
            "f1:",
            "fn instance f() -> Null",
            "  b0:",
            "    r0 = copy 0",
            "    r1 = closure f0 []",
            "    r2 = closure f2 [r1]",
            "    r3 = copy r2",
            "    r4 = copy r0",
            "    call value r3(r4) -> r5",
            "    r0 = copy r5",
            "    return _",
            "f2:",
            "fn closure of f1(r1) -> Null",
            "  captures r0",
            "  b0:",
            "    r3 = call value r0(r1) -> r2",
            "    r1 = copy r2",
            "    return _",
        ],
    );
}
