use mote_semantics::{compile, compile_all, Compiled};

use super::optimize;
use crate::uses::{operands_mut, reads};
use crate::{lower, print_function, verify, FunctionKind, Lowered, Operand};

fn lowered(source: &str) -> (Compiled, Lowered) {
    let mut compiled = compile(source);
    assert_eq!(compiled.errors, Vec::<String>::new());
    let lowered = lower(&mut compiled.program, &mut compiled.interner);
    assert_eq!(lowered.unsupported, []);
    (compiled, lowered)
}

/// The blocks of `name` after cleanup, without the header and register types.
fn after(source: &str, name: &str) -> String {
    let (mut compiled, mut lowered) = lowered(source);
    optimize(&mut lowered.module, &mut compiled.program, &mut compiled.interner);
    assert_eq!(verify(&lowered.module, &compiled.program, &compiled.interner), []);
    let (_, function) = lowered
        .module
        .functions
        .iter()
        .find(|(_, function)| matches!(&function.kind, FunctionKind::Instance(instance) if compiled.interner.resolve(compiled.program.defs[instance.def].name) == name))
        .unwrap_or_else(|| panic!("no function {name}"));
    let text = print_function(function, &compiled.program, &compiled.interner);
    text.lines().filter(|line| line.starts_with("  b") || line.starts_with("    ")).collect::<Vec<_>>().join("\n")
}

#[test]
fn a_copy_of_a_copy_is_read_where_it_began() {
    let got = after("fn f(a: Int) -> Int {\n let b = a\n let c = b\n return c + 1\n}", "f");
    assert_eq!(got, "  b0:\n    r3 = add r0, 1\n    return r3");
}

#[test]
fn constants_flow_through_copies_and_fold() {
    let got = after("fn f() -> Int {\n let a = 2 + 3\n return a * 4\n}", "f");
    assert_eq!(got, "  b0:\n    return 20");
}

#[test]
fn an_operation_that_would_fault_is_kept() {
    let got = after("fn f() -> Int {\n let a: Int8 = 127\n return (a + 1) as Int\n}", "f");
    assert!(got.contains("add 127, 1"), "{got}");
    let got = after("fn f() -> Int {\n let z = 0\n return 1 / z\n}", "f");
    assert!(got.contains("div 1, 0"), "{got}");
}

#[test]
fn an_unread_result_goes_unless_the_operation_can_fault() {
    let got = after("fn f(a: Int, b: Int) -> Int {\n let x = a * b\n let y = a == b\n let z = (a, b)\n return 1\n}", "f");
    assert_eq!(got, "  b0:\n    r2 = mul r0, r1\n    return 1");
}

#[test]
fn a_branch_on_a_constant_keeps_one_side() {
    let got = after("fn f() -> Int {\n if 1 < 2 {\n return 1\n }\n return 2\n}", "f");
    assert_eq!(got, "  b0:\n    return 1");
}

#[test]
fn a_chain_of_equality_tests_becomes_a_switch() {
    let got = after("fn f(n: Int) -> Int {\n return match n {\n 1 => 10\n 2 => 20\n 3 => 30\n _ => 0\n }\n}", "f");
    assert!(got.contains("switch r0"), "{got}");
    assert!(!got.contains("eq r0"), "{got}");
}

#[test]
fn an_array_that_enters_a_task_is_shared_only_when_nothing_writes_it() {
    let read = "fn f() -> Int {\n let xs: [Int] = [1, 2, 3]\n let h = spawn { return xs[0] }\n return h.await().unwrap()\n}";
    assert!(!after(read, "f").contains("detach"), "{}", after(read, "f"));
    let param = "fn f(xs: [Int]) -> Int {\n let h = spawn { return xs[0] }\n return h.await().unwrap()\n}";
    assert!(after(param, "f").contains("detach"));
    let written = "fn f() -> Int {\n var xs: [Int] = [1, 2, 3]\n let ys = xs\n let h = spawn { return ys[0] }\n xs[0] = 9\n return h.await().unwrap()\n}";
    assert!(after(written, "f").contains("detach"));
}

const BIG: &str = "fn big(a: Int) -> Int {\n let b = a * 2\n let c = b + a\n let d = c * c\n let e = d - b\n let g = e + c\n let h = g * 3\n let i = h - a\n let j = i * i\n let k = j + b\n return k - 1\n}\n";

#[test]
fn a_small_function_is_inlined_at_every_call() {
    let got = after("fn inc(a: Int) -> Int {\n return a + 1\n}\nfn f(x: Int) -> Int {\n return inc(x) + inc(inc(x))\n}", "f");
    assert!(!got.contains("call"), "{got}");
}

#[test]
fn a_large_function_is_inlined_only_when_it_has_one_call() {
    let once = format!("{BIG}fn f(x: Int) -> Int {{\n return big(x)\n}}");
    assert!(!after(&once, "f").contains("call"), "{}", after(&once, "f"));
    let twice = format!("{BIG}fn f(x: Int) -> Int {{\n return big(x) + big(1)\n}}");
    assert!(after(&twice, "f").contains("call"), "{}", after(&twice, "f"));
}

#[test]
fn a_recursive_function_keeps_its_calls() {
    let source = "fn fact(n: Int) -> Int {\n if n <= 1 {\n return 1\n }\n return n * fact(n - 1)\n}\nfn f() -> Int {\n return fact(5)\n}";
    assert!(after(source, "f").contains("call"), "{}", after(source, "f"));
}

#[test]
fn a_record_that_is_only_read_and_written_by_field_has_no_allocation() {
    let build = "struct P {\n var x: Int\n var y: Int\n}\nfn f(a: Int) -> Int {\n let p = P { x: a, y: 2 }\n return p.x + p.y\n}";
    assert!(!after(build, "f").contains("record"), "{}", after(build, "f"));
    let write = "struct P {\n var x: Int\n var y: Int\n}\nfn f(a: Int) -> Int {\n var p = P { x: 1, y: 2 }\n p.x = a\n return p.x + p.y\n}";
    assert!(!after(write, "f").contains("record"), "{}", after(write, "f"));
    let tuple = "fn f(a: Int) -> Int {\n let t = (a, 2)\n return t.0 + t.1\n}";
    assert!(!after(tuple, "f").contains("tuple"), "{}", after(tuple, "f"));
}

#[test]
fn a_record_that_leaves_the_function_stays() {
    let returned = "struct P {\n var x: Int\n}\nfn f(a: Int) -> P {\n return P { x: a }\n}";
    assert!(after(returned, "f").contains("record"));
    let passed = "struct P {\n var x: Int\n}\nfn take(p: P) -> Int {\n if p.x > 0 {\n return take(P { x: p.x - 1 })\n }\n return 0\n}\nfn f(a: Int) -> Int {\n return take(P { x: a })\n}";
    assert!(after(passed, "f").contains("record"), "{}", after(passed, "f"));
}

#[test]
fn a_cell_no_closure_holds_is_a_register() {
    let got = after("fn f() -> Int {\n var c = 0\n let g = || { c += 1 }\n return c\n}", "f");
    assert!(!got.contains("cell"), "{got}");
}

const APPLY: &str = "fn apply(f: (Int) -> Int, n: Int) -> Int {\n var s = 0\n var i = 0\n while i < n {\n s += f(i)\n i += 1\n }\n return s\n}\n";

#[test]
fn a_call_through_a_closure_made_in_the_same_function_runs_the_body_in_place() {
    let got = after(&format!("{APPLY}fn run(k: Int) -> Int {{\n return apply(|x| x * k, 10)\n}}"), "run");
    assert!(!got.contains("call value") && !got.contains("closure f"), "{got}");
    assert!(got.contains("mul"), "{got}");
}

#[test]
fn a_closure_that_changes_a_captured_variable_is_still_called_through_its_value() {
    let got = after(&format!("{APPLY}fn run() -> Int {{\n var total = 0\n let r = apply(|x| {{\n total += x\n return x\n }}, 3)\n return r + total\n}}"), "run");
    assert!(got.contains("call value"), "{got}");
}

#[test]
fn a_closure_made_again_each_round_passes_the_values_of_that_round() {
    let got = after("fn run(n: Int) -> Int {\n var total = 0\n var i = 0\n while i < n {\n let k = i * 2\n let f = |x: Int| x + k\n total += f(1)\n i += 1\n }\n return total\n}", "run");
    assert!(!got.contains("call value"), "{got}");
}

#[test]
fn what_both_sides_of_an_if_agree_on_still_holds_after_it() {
    let got = after("fn f(n: Int) -> Int {\n var i = 0\n var s = 0\n while i < n {\n if i % 2 == 0 {\n s += 1\n }\n i += 1\n }\n return s\n}", "f");
    assert_eq!(got.matches("num_wrapping_add").count(), 1, "{got}");
    assert!(got.contains("add r"), "{got}");
}

#[test]
fn a_deep_copy_of_a_value_that_holds_no_object_is_a_plain_copy() {
    let got = after("import { Mutex } from std.sync\nfn f() -> Int {\n let m = Mutex(0)\n return m.get()\n}", "f");
    assert!(!got.contains("deep_clone"), "{got}");
}

#[test]
fn a_deep_copy_of_a_list_built_to_be_copied_is_a_plain_copy_and_one_of_a_list_held_elsewhere_is_not() {
    let fresh = after("import { Mutex } from std.sync\nfn f() -> Int {\n let m = Mutex([0])\n return m.get().len()\n}", "f");
    assert_eq!(fresh.matches("deep_clone").count(), 1, "{fresh}");
    let held = after("import { Mutex } from std.sync\nfn f() -> Int {\n let xs = [1]\n let m = Mutex(xs)\n return m.get().len() + xs.len()\n}", "f");
    assert_eq!(held.matches("deep_clone").count(), 2, "{held}");
}

#[test]
fn a_match_on_ranges_becomes_one_switch() {
    let text = after("fn f(n: Int) -> Int {\n return match n {\n 1..=5 => 10\n 6..=9 => 20\n 20 => 30\n _ => 0\n }\n}", "f");
    assert!(text.contains("switch r0"), "{text}");
    assert!(!text.contains("ge r0") && !text.contains("le r0"), "{text}");
}

#[test]
fn every_plain_function_of_std_verifies_after_cleanup() {
    let mut compiled = compile_all("fn main() {}");
    assert_eq!(compiled.errors, Vec::<String>::new());
    let mut lowered = lower(&mut compiled.program, &mut compiled.interner);
    optimize(&mut lowered.module, &mut compiled.program, &mut compiled.interner);
    assert_eq!(verify(&lowered.module, &compiled.program, &compiled.interner), []);
}

#[test]
fn the_operand_walk_agrees_with_the_register_walk() {
    let mut compiled = compile_all("fn main() {}");
    let mut lowered = lower(&mut compiled.program, &mut compiled.interner);
    let ids: Vec<_> = lowered.module.functions.iter().map(|(id, _)| id).collect();
    for id in ids {
        let blocks: Vec<_> = lowered.module.functions[id].blocks.iter().map(|(block, _)| block).collect();
        for block in blocks {
            for inst in &mut lowered.module.functions[id].blocks[block].insts {
                let expected = reads(&inst.op);
                let walked: Vec<_> = operands_mut(&mut inst.op).into_iter().filter_map(|operand| if let Operand::Reg(reg) = operand { Some(*reg) } else { None }).collect();
                assert_eq!(walked, expected, "{:?}", inst.op);
            }
        }
    }
}

#[test]
fn a_loop_with_constant_bounds_is_unrolled_and_its_counter_folds() {
    let got = after("fn f() -> Int {\n var total = 0\n for i in 0..4 {\n total += i * 2\n }\n return total\n}", "f");
    assert_eq!(got, "  b0:\n    return 12");
    let got = after("fn f() -> Int {\n var total = 0\n for i in 2..5 {\n total += i\n }\n return total\n}", "f");
    assert_eq!(got, "  b0:\n    return 9");
    let got = after("fn f() -> Int {\n var total = 7\n for i in 5..5 {\n total += i\n }\n return total\n}", "f");
    assert_eq!(got, "  b0:\n    return 7");
}

#[test]
fn a_loop_whose_bound_is_not_a_small_constant_stays() {
    let open = after("fn f(n: Int) -> Int {\n var total = 0\n for i in 0..n {\n total += i\n }\n return total\n}", "f");
    assert!(open.contains("lt "), "{open}");
    let long = after("fn f() -> Int {\n var total = 0\n for i in 0..100 {\n total += i\n }\n return total\n}", "f");
    assert!(long.contains("lt "), "{long}");
}

#[test]
fn an_unrolled_loop_faults_where_the_loop_faulted() {
    let got = after("fn f() -> Int {\n var total = 0\n for i in 0..3 {\n total += 10 / (2 - i)\n }\n return total\n}", "f");
    assert_eq!(got, "  b0:\n    r7 = div 10, 0\n    r8 = add 15, r7\n    return r8");
}

/// What `main` shows of the described type after cleanup, without the register types.
fn described(source: &str) -> String {
    let mut compiled = compile_all(source);
    assert_eq!(compiled.errors, Vec::<String>::new());
    let mut lowered = lower(&mut compiled.program, &mut compiled.interner);
    optimize(&mut lowered.module, &mut compiled.program, &mut compiled.interner);
    assert_eq!(verify(&lowered.module, &compiled.program, &compiled.interner), []);
    let (_, function) = lowered.module.functions.iter().find(|(_, function)| matches!(&function.kind, FunctionKind::Instance(instance) if compiled.interner.resolve(compiled.program.defs[instance.def].name) == "main")).expect("a main");
    print_function(function, &compiled.program, &compiled.interner)
}

#[test]
fn a_type_described_at_a_known_type_has_no_table_read_left() {
    let got = described("import { describe } from std.meta\nstruct P {\n x: Int\n y: Int\n}\nfn main() {\n println(describe<P>().fields().len())\n}");
    assert!(!got.contains("meta_ints") && !got.contains("meta_texts") && !got.contains(" lt "), "{got}");
    assert!(got.contains("copy \"x\"") && got.contains("copy \"y\""), "{got}");
}

#[test]
fn the_variants_of_a_known_enum_have_no_table_read_left() {
    let got = described("import { describe } from std.meta\nenum Shape {\n Empty\n Circle(Float)\n Rect { width: Float, height: Float }\n}\nfn main() {\n println(describe<Shape>().variants().len())\n}");
    assert!(!got.contains("meta_ints") && !got.contains("meta_texts"), "{got}");
    assert!(got.contains("copy \"Rect\"") && got.contains("copy \"width\""), "{got}");
}

#[test]
fn a_type_described_through_a_value_read_by_position_still_reads_the_table() {
    let got = described("import { describe } from std.meta\nstruct P {\n x: Int\n}\nfn name_of(id: Int) -> String {\n return describe<P>().name()\n}\nfn main() {\n println(name_of(1))\n}");
    assert!(!got.contains("meta_ints"), "{got}");
}

#[test]
fn a_subtraction_after_a_guard_that_bounds_the_operand_cannot_overflow() {
    let got = after("fn fib(n: Int) -> Int {\n if n < 2 {\n return n\n }\n return fib(n - 1) + fib(n - 2)\n}", "fib");
    assert_eq!(got.matches("num_wrapping_sub").count(), 2, "{got}");
    assert!(got.contains("add r3, r5"), "{got}");
}

#[test]
fn an_increment_under_the_loop_test_cannot_overflow() {
    let got = after("fn f(n: Int) -> Int {\n var i = 0\n var s = 0\n while i < n {\n s += i\n i += 1\n }\n return s\n}", "f");
    assert!(got.contains("num_wrapping_add(r1, 1)") && got.contains("add r2, r1"), "{got}");
}

#[test]
fn an_operation_with_no_bounds_keeps_its_check() {
    let got = after("fn f(a: Int) -> Int {\n return a + 1\n}", "f");
    assert!(got.contains("add r0, 1") && !got.contains("wrapping"), "{got}");
}

#[test]
fn the_bounds_of_a_register_end_where_it_is_written() {
    let got = after("var level = 7\nfn f(n: Int) -> Int {\n var m = n\n if m < 2 {\n return 0\n }\n m = level\n return m - 1\n}", "f");
    assert!(got.contains("sub") && !got.contains("wrapping"), "{got}");
}

#[test]
fn bounds_from_a_constant_test_reach_exactly_as_far_as_the_type_allows() {
    let fits = after("fn f(n: Int8) -> Int8 {\n if n < 100 {\n return n + 28\n }\n return 0\n}", "f");
    assert!(fits.contains("num_wrapping_add"), "{fits}");
    let over = after("fn f(n: Int8) -> Int8 {\n if n < 100 {\n return n + 29\n }\n return 0\n}", "f");
    assert!(!over.contains("wrapping"), "{over}");
}
