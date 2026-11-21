use mote_semantics::{compile, Compiled};

use crate::{lower, print_function, verify, FunctionKind, Lowered};

/// Compiles `source` with the real std and lowers every instance it needs.
pub(super) fn lowered(source: &str) -> (Compiled, Lowered) {
    let mut compiled = compile(source);
    assert_eq!(compiled.errors, Vec::<String>::new());
    let lowered = lower(&mut compiled.program, &mut compiled.interner);
    (compiled, lowered)
}

/// The text of the function named `name`; the lowering must have verified clean.
fn dump(compiled: &Compiled, lowered: &Lowered, name: &str) -> String {
    assert_eq!(verify(&lowered.module, &compiled.program, &compiled.interner), []);
    let (_, function) = lowered
        .module
        .functions
        .iter()
        .find(|(_, function)| matches!(&function.kind, FunctionKind::Instance(instance) if compiled.interner.resolve(compiled.program.defs[instance.def].name) == name))
        .unwrap_or_else(|| panic!("no function {name}"));
    print_function(function, &compiled.program, &compiled.interner)
}

/// Only the blocks of a function, without its header and register types.
pub(super) fn code(compiled: &Compiled, lowered: &Lowered, name: &str) -> String {
    let text = dump(compiled, lowered, name);
    text.lines().filter(|line| line.starts_with("  b") || line.starts_with("    ")).collect::<Vec<_>>().join("\n")
}

#[test]
fn arithmetic_and_let_lower_to_three_address_code() {
    let (compiled, lowered) = lowered("fn f(a: Int, b: Int) -> Int {\n let c = a + b\n return c * 2\n}");
    assert_eq!(lowered.unsupported, []);
    let expected = "fn instance f(r0, r1) -> Int\n  r0: Int\n  r1: Int\n  r2: Int\n  r3: Int\n  r4: Int\n  b0:\n    r2 = add r0, r1\n    r3 = copy r2\n    r4 = mul r3, 2\n    return r4\n";
    assert_eq!(dump(&compiled, &lowered, "f"), expected);
}

#[test]
fn a_var_is_copied_when_read_and_written_in_place() {
    let (compiled, lowered) = lowered("fn f() -> Int {\n var x = 1\n x += 2\n x = x * 3\n return x\n}");
    assert_eq!(lowered.unsupported, []);
    let expected = "  b0:\n    r0 = copy 1\n    r1 = copy r0\n    r2 = add r1, 2\n    r0 = copy r2\n    r3 = copy r0\n    r4 = mul r3, 3\n    r0 = copy r4\n    r5 = copy r0\n    return r5";
    assert_eq!(code(&compiled, &lowered, "f"), expected);
}

#[test]
fn named_arguments_are_evaluated_in_the_order_written() {
    let (compiled, lowered) = lowered("fn g(a: Int, b: Int) -> Int { return a - b }\nfn h() -> Int { return 1 }\nfn k() -> Int { return 2 }\nfn f() -> Int { return g(b=h(), a=k()) }");
    assert_eq!(lowered.unsupported, []);
    let expected = "  b0:\n    r0 = call f1()\n    r1 = call f2()\n    r2 = call f0(r1, r0)\n    return r2";
    assert_eq!(code(&compiled, &lowered, "f"), expected);
}

#[test]
fn a_var_argument_is_copied_in_and_the_result_written_back() {
    let (compiled, lowered) = lowered("fn bump(var n: Int) { n += 1 }\nfn f() -> Int {\n var c = 0\n bump(c)\n bump(1 + 2)\n return c\n}");
    assert_eq!(lowered.unsupported, []);
    assert_eq!(code(&compiled, &lowered, "bump"), "  b0:\n    r1 = copy r0\n    r2 = add r1, 1\n    r0 = copy r2\n    return _");
    let expected = "  b0:\n    r0 = copy 0\n    r1 = copy r0\n    call f0(r1) -> r2\n    r0 = copy r2\n    r3 = add 1, 2\n    call f0(r3) -> r4\n    r5 = copy r0\n    return r5";
    assert_eq!(code(&compiled, &lowered, "f"), expected);
}

#[test]
fn a_field_write_goes_through_the_object_or_a_copy_stored_back() {
    let source = "struct In {\n var n: Int\n}\nstruct Out {\n var inner: In\n}\nclass C {\n var n: Int\n}\nfn a(var o: Out) { o.inner.n = 3 }\nfn b(var c: C) { c.n = 4 }\nfn d(var i: In) { i.n += 1 }";
    let (compiled, lowered) = lowered(source);
    assert_eq!(lowered.unsupported, []);
    let nested = "  b0:\n    r1 = field r0.0\n    r2 = copy r1\n    set_field r2.0, 3\n    set_field r0.0, r2\n    return _";
    assert_eq!(code(&compiled, &lowered, "a"), nested);
    assert_eq!(code(&compiled, &lowered, "b"), "  b0:\n    set_field r0.0, 4\n    return _");
    assert_eq!(code(&compiled, &lowered, "d"), "  b0:\n    r1 = field r0.0\n    r2 = add r1, 1\n    set_field r0.0, r2\n    return _");
}

#[test]
fn text_is_shown_and_concatenated() {
    let (compiled, lowered) = lowered("fn f(n: Int) -> String { return \"n=${n}!\" }");
    assert_eq!(lowered.unsupported, []);
    assert_eq!(code(&compiled, &lowered, "f"), "  b0:\n    r1 = show r0\n    r2 = concat \"n=\", r1, \"!\"\n    return r2");
}
