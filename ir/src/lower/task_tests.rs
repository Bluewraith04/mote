use mote_semantics::Compiled;

use super::tests::lowered;
use crate::{print_function, verify, FunctionKind, Lowered};

/// The functions named in `names`, and every closure, as their headers and blocks without the register types.
fn text(compiled: &Compiled, lowered: &Lowered, names: &[&str]) -> String {
    assert_eq!(verify(&lowered.module, &compiled.program, &compiled.interner), []);
    let mut out = Vec::new();
    for (id, function) in lowered.module.functions.iter() {
        let named = matches!(&function.kind, FunctionKind::Instance(instance) if names.contains(&compiled.interner.resolve(compiled.program.defs[instance.def].name)));
        if named || matches!(function.kind, FunctionKind::Closure { .. }) {
            out.push(format!("f{}:", id.index()));
            out.extend(print_function(function, &compiled.program, &compiled.interner).lines().filter(|line| !line.starts_with("  r")).map(str::to_string));
        }
    }
    out.join("\n")
}

/// The named functions and the closures must be exactly `expected`, one line each, with nothing unsupported.
fn expect(source: &str, names: &[&str], expected: &[&str]) {
    let (compiled, lowered) = lowered(source);
    assert_eq!(lowered.unsupported, []);
    assert_eq!(text(&compiled, &lowered, names), expected.join("\n"));
}

const RES: &str = "struct Res {\n id: Int\n fn close(self) {\n }\n}\nfn open() -> Res {\n return Res { id: 1 }\n}\n";

#[test]
fn a_spawn_body_is_a_function_of_its_own_with_the_values_it_captures() {
    expect(
        "fn f(k: Int) -> Int {\n let h = spawn { return k * 7 }\n return 1\n}",
        &["f"],
        &[
            "f0:",
            "fn instance f(r0) -> Int",
            "  b0:",
            "    r1 = spawn f1 [r0]",
            "    r2 = copy r1",
            "    return 1",
            "f1:",
            "fn closure of f0() -> Int",
            "  captures r0",
            "  b0:",
            "    r1 = mul r0, 7",
            "    return r1",
        ],
    );
}

#[test]
fn an_array_enters_a_task_as_a_copy_going_in_and_coming_out() {
    expect(
        "fn f(xs: [Int]) {\n let h = spawn { return xs.len() }\n}",
        &["f"],
        &[
            "f0:",
            "fn instance f(r0) -> Null",
            "  b0:",
            "    r1 = detach r0",
            "    r2 = spawn f2 [r1]",
            "    r3 = copy r2",
            "    return _",
            "f2:",
            "fn closure of f0() -> Int",
            "  captures r0",
            "  b0:",
            "    r1 = call f1(r0)",
            "    return r1",
        ],
    );
    expect(
        "fn f() {\n let h = spawn {\n let a: [Int] = [0; 4]\n return a\n }\n}",
        &["f"],
        &[
            "f0:",
            "fn instance f() -> Null",
            "  b0:",
            "    r0 = spawn f1 []",
            "    r1 = copy r0",
            "    return _",
            "f1:",
            "fn closure of f0() -> [Int]",
            "  b0:",
            "    r0 = array_repeat 0, 4",
            "    r1 = copy r0",
            "    r2 = detach r1",
            "    return r2",
        ],
    );
}

#[test]
fn an_array_sent_on_a_channel_is_copied_before_it_goes() {
    expect(
        "import { Channel } from std.sync\nfn f(xs: [Int]) {\n let (tx, rx) = Channel<[Int]>(1)\n tx.send(xs)\n}",
        &["f", "send"],
        &[
            "f0:",
            "fn instance f(r0) -> Null",
            "  b0:",
            "    r1 = call f1(1)",
            "    r2 = tuple_field r1.0",
            "    r3 = copy r2",
            "    r4 = tuple_field r1.1",
            "    r5 = copy r4",
            "    call f2(r3, r0)",
            "    return _",
            "f2:",
            "fn instance send(r0, r1) -> Null",
            "  b0:",
            "    r2 = detach r1",
            "    call intrinsic sender_send(r0, r2)",
            "    return _",
        ],
    );
}

#[test]
fn a_scope_waits_for_its_tasks_at_its_end() {
    expect(
        "fn f() -> Int {\n scope {\n let a = spawn { return 1 }\n }\n return 2\n}",
        &["f"],
        &[
            "f0:",
            "fn instance f() -> Int",
            "  b0:",
            "    r0 = scope_begin",
            "    r1 = spawn f1 [] in r0",
            "    r2 = copy r1",
            "    scope_end r0",
            "    return 2",
            "f1:",
            "fn closure of f0() -> Int",
            "  b0:",
            "    return 1",
        ],
    );
}

#[test]
fn a_return_out_of_a_scope_waits_first() {
    expect(
        "fn f(c: Bool) -> Int {\n scope {\n let a = spawn { return 1 }\n if c { return 5 }\n }\n return 2\n}",
        &["f"],
        &[
            "f0:",
            "fn instance f(r0) -> Int",
            "  b0:",
            "    r1 = scope_begin",
            "    r2 = spawn f1 [] in r1",
            "    r3 = copy r2",
            "    branch r0, b1, b2",
            "  b1:",
            "    scope_end r1",
            "    return 5",
            "  b2:",
            "    scope_end r1",
            "    return 2",
            "f1:",
            "fn closure of f0() -> Int",
            "  b0:",
            "    return 1",
        ],
    );
}

#[test]
fn break_and_continue_out_of_a_scope_wait_first() {
    expect(
        "fn f() {\n for i in 0..3 {\n scope {\n let a = spawn { return 1 }\n if i == 1 { break }\n if i == 2 { continue }\n }\n }\n}",
        &["f"],
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
            "    r4 = scope_begin",
            "    r5 = spawn f1 [] in r4",
            "    r6 = copy r5",
            "    r7 = eq r3, 1",
            "    branch r7, b5, b6",
            "  b3:",
            "    r0 = add r0, 1",
            "    jump b1",
            "  b4:",
            "    return _",
            "  b5:",
            "    scope_end r4",
            "    jump b4",
            "  b6:",
            "    r8 = eq r3, 2",
            "    branch r8, b7, b8",
            "  b7:",
            "    scope_end r4",
            "    jump b3",
            "  b8:",
            "    scope_end r4",
            "    jump b3",
            "f1:",
            "fn closure of f0() -> Int",
            "  b0:",
            "    return 1",
        ],
    );
}

#[test]
fn a_with_closes_its_resource_at_the_end_of_the_block() {
    expect(
        &format!("{RES}fn f() {{\n with p = open() {{\n let n = 1\n }}\n}}"),
        &["f"],
        &["f1:", "fn instance f() -> Null", "  b0:", "    r0 = call f0()", "    r1 = copy r0", "    defer r1, f2", "    r2 = copy 1", "    undefer", "    call f2(r1)", "    return _"],
    );
}

#[test]
fn a_return_out_of_a_with_closes_first() {
    expect(
        &format!("{RES}fn f(c: Bool) -> Int {{\n with p = open() {{\n if c {{ return 1 }}\n }}\n return 2\n}}"),
        &["f"],
        &[
            "f1:",
            "fn instance f(r0) -> Int",
            "  b0:",
            "    r1 = call f0()",
            "    r2 = copy r1",
            "    defer r2, f2",
            "    branch r0, b1, b2",
            "  b1:",
            "    undefer",
            "    call f2(r2)",
            "    return 1",
            "  b2:",
            "    undefer",
            "    call f2(r2)",
            "    return 2",
        ],
    );
}

#[test]
fn a_return_through_a_with_inside_a_scope_closes_then_waits() {
    expect(
        &format!("{RES}fn f() -> Int {{\n scope {{\n with p = open() {{\n return 1\n }}\n }}\n return 2\n}}"),
        &["f"],
        &[
            "f1:",
            "fn instance f() -> Int",
            "  b0:",
            "    r0 = scope_begin",
            "    r1 = call f0()",
            "    r2 = copy r1",
            "    defer r2, f2",
            "    undefer",
            "    call f2(r2)",
            "    scope_end r0",
            "    return 1",
            "  b1:",
            "    return 2",
        ],
    );
}
