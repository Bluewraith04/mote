//! `mote check --memory`: where each struct literal lives and why.

use std::process::Command;

fn check(source: &str, tag: &str, memory: bool) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_memreport_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.mote");
    std::fs::write(&file, source).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mote"));
    cmd.arg("check").arg(&file);
    if memory {
        cmd.arg("--memory");
    }
    let out = cmd.output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

const POINT: &str = "struct Point {\n    var x: Int\n    var y: Int\n}\n\nfn take(p: Point) -> Int {\n    return p.x\n}\n\nfn keep(p: Point, var xs: List<Point>) {\n    xs.push(p)\n}\n\n";

const PEEK: &str = "\nfn peek(p: Point) -> Int {\n    let q = p\n    return q.x\n}\n";

fn report(body: &str, tag: &str) -> String {
    let (ok, text) = check(&format!("{POINT}fn main() {{\n{body}\n}}\n{PEEK}"), tag, true);
    assert!(ok, "{text}");
    text
}

#[test]
fn a_field_only_literal_is_in_registers() {
    let text = report("    let a = Point { x: 1, y: 2 }\n    println(a.x + a.y)", "registers");
    assert!(text.contains("main.mote:15:13  Point { .. }  registers, no memory"), "{text}");
    assert!(text.contains("1 struct literal(s): 1 in registers, 0 in regions, 0 on the heap."), "{text}");
}

#[test]
fn a_copied_literal_is_in_a_region() {
    let text = report("    let a = Point { x: 1, y: 2 }\n    let b = a\n    println(b.x)", "region");
    assert!(text.contains("main.mote:15:13  Point { .. }  region, freed when its block ends"), "{text}");
    assert!(text.contains("1 struct literal(s): 0 in registers, 1 in regions, 0 on the heap."), "{text}");
}

#[test]
fn a_whole_value_use_names_the_use() {
    let text = report("    var xs: List<Point> = []\n    let b = Point { x: 3, y: 4 }\n    xs.push(b)\n    println(xs.len())", "whole");
    assert!(text.contains("heap: `b` is used as a whole value at 17:13"), "{text}");
}

#[test]
fn a_call_that_keeps_its_argument_names_the_call() {
    let text = report("    var xs: List<Point> = []\n    let b = Point { x: 3, y: 4 }\n    keep(b, xs)\n    println(xs.len())", "retained");
    assert!(text.contains("heap: `b` is passed at 17:10 to a call that may keep it"), "{text}");
}

#[test]
fn a_call_that_only_reads_its_argument_leaves_it_in_a_region() {
    let text = report("    let b = Point { x: 3, y: 4 }\n    println(peek(b))", "reads");
    assert!(text.contains("region, freed when its block ends"), "{text}");
}

#[test]
fn a_call_that_only_reads_its_fields_passes_registers() {
    let text = report("    let b = Point { x: 3, y: 4 }\n    println(take(b))", "reads_fields");
    assert!(text.contains("registers, no memory"), "{text}");
}

#[test]
fn a_capture_names_the_use() {
    let text = report("    let c = Point { x: 7, y: 8 }\n    let f = || c.x\n    println(f())", "capture");
    assert!(text.contains("heap: `c` is used inside a spawn or lambda at 16:16"), "{text}");
}

#[test]
fn a_literal_that_is_not_bound_is_heap() {
    let text = report("    println(Point { x: 5, y: 6 }.x)", "unbound");
    assert!(text.contains("heap: not the initializer of a let or var"), "{text}");
}

#[test]
fn a_redeclared_name_is_heap() {
    let text = report("    let a = Point { x: 1, y: 2 }\n    println(a.x)\n    let a = Point { x: 3, y: 4 }\n    println(a.y)", "redeclared");
    assert!(text.contains("heap: `a` is declared more than once"), "{text}");
}

#[test]
fn a_generator_struct_is_placed_in_a_region() {
    let source = format!("{POINT}fn gen() -> Stream<Int> {{\n    let a = Point {{ x: 1, y: 2 }}\n    let b = a\n    yield b.x\n}}\n\nfn main() {{\n    println(1)\n}}\n");
    let (ok, text) = check(&source, "yield", true);
    assert!(ok, "{text}");
    assert!(text.contains("region, freed when its block ends"), "{text}");
}

#[test]
fn a_struct_with_too_many_fields_is_heap() {
    let fields: String = (0..64).map(|i| format!("    var f{i}: Int\n")).collect();
    let inits: Vec<String> = (0..64).map(|i| format!("f{i}: {i}")).collect();
    let source = format!("struct Wide {{\n{fields}}}\n\nfn main() {{\n    let w = Wide {{ {} }}\n    println(w.f0)\n}}\n", inits.join(", "));
    let (ok, text) = check(&source, "wide", true);
    assert!(ok, "{text}");
    assert!(text.contains("heap: 64 fields, and a region holds at most 63"), "{text}");
}

#[test]
fn std_modules_are_left_out() {
    let (ok, text) = check("import { map_from } from std.collections\n\nfn main() {\n    println(1)\n}\n", "std", true);
    assert!(ok, "{text}");
    assert!(!text.contains("<std>"), "{text}");
}

#[test]
fn without_the_flag_nothing_is_reported() {
    let (ok, text) = check(&format!("{POINT}fn main() {{\n    let a = Point {{ x: 1, y: 2 }}\n    println(a.x)\n}}\n"), "off", false);
    assert!(ok, "{text}");
    assert!(!text.contains("struct literal(s)"), "{text}");
}
