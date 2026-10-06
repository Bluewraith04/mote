//! Escape analysis places non-escaping struct literals in regions: behaviour and placement.

use std::process::Command;

use isa::opcode::Opcode;
use modules::MultiFileCompiler;

const POINT: &str = "struct Point {\n    var x: Int\n    var y: Int\n}\n";

fn dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mote_regions_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(name: &str, source: &str) -> String {
    let dir = dir(name);
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).lines().collect::<Vec<_>>().join("\n")
}

fn region_sites(name: &str, source: &str) -> usize {
    let dir = dir(name);
    let main = dir.join("main.mote");
    std::fs::write(&main, source).unwrap();
    let compiled = MultiFileCompiler::new(dir.clone()).compile_program(&main).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    compiled
        .code_objects
        .iter()
        .flat_map(|c| c.instructions.iter())
        .filter(|i| (**i & 0xFF) as u8 == Opcode::ARENAALLOC as u8)
        .count()
}

fn sites(name: &str, body: &str) -> usize {
    region_sites(name, &format!("{POINT}{body}"))
}

#[test]
fn a_struct_used_only_through_its_fields_is_held_in_registers() {
    assert_eq!(sites("plain", "fn f(a: Int) -> Int {\n    var p = Point { x: a, y: 1 }\n    p.x = p.x + 1\n    return p.x + p.y\n}\n"), 0);
}

#[test]
fn a_struct_in_a_generator_is_held_in_registers() {
    assert_eq!(sites("generator", "fn f() -> Stream<Int> {\n    let p = Point { x: 1, y: 2 }\n    yield p.x\n    yield p.y\n}\n"), 0);
}

#[test]
fn a_struct_that_is_only_copied_or_read_is_placed_in_a_region() {
    let cases = [
        ("returned", "fn f() -> Point {\n    let p = Point { x: 1, y: 2 }\n    return p\n}\n"),
        ("argument", "fn g(p: Point) -> Int {\n    let q = p\n    return q.x\n}\nfn f() -> Int {\n    let p = Point { x: 1, y: 2 }\n    return g(p)\n}\n"),
        ("copied", "fn f() -> Int {\n    let p = Point { x: 1, y: 2 }\n    let q = p\n    return q.x\n}\n"),
        ("listed", "fn f() -> Int {\n    let p = Point { x: 1, y: 2 }\n    let xs = [p]\n    return xs.len()\n}\n"),
        ("compared", "fn f() -> Bool {\n    let p = Point { x: 1, y: 2 }\n    return p == p\n}\n"),
        ("matched", "fn f() -> Int {\n    let p = Point { x: 1, y: 2 }\n    match p {\n        q => { return q.x }\n    }\n}\n"),
        ("spawned", "fn f() -> Int {\n    let p = Point { x: 1, y: 2 }\n    let t = spawn { p.x + 1 }\n    let r = t.join()\n    return 1\n}\n"),
    ];
    for (name, body) in cases {
        assert_eq!(sites(name, body), 1, "{name} must use a region");
    }
}

#[test]
fn a_struct_that_escapes_stays_on_the_heap() {
    let cases = [
        ("kept", "fn keep(p: Point, var xs: List<Point>) { xs.push(p) }\nfn f() -> Int {\n    var xs: List<Point> = []\n    let p = Point { x: 1, y: 2 }\n    keep(p, xs)\n    return xs.len()\n}\n"),
        ("pushed", "fn f() -> Int {\n    var xs: List<Point> = []\n    let p = Point { x: 1, y: 2 }\n    xs.push(p)\n    return xs.len()\n}\n"),
        ("captured", "fn f() -> Int {\n    let p = Point { x: 1, y: 2 }\n    let g = || { return p.x }\n    return g()\n}\n"),
        ("reassigned", "fn f() -> Int {\n    var p = Point { x: 1, y: 2 }\n    p = Point { x: 3, y: 4 }\n    return p.x\n}\n"),
        ("shadowed", "fn f() -> Int {\n    let p = Point { x: 1, y: 2 }\n    let n = 0\n    if n == 0 {\n        let p = Point { x: 5, y: 6 }\n        return p.x\n    }\n    return p.x\n}\n"),
        ("through a function value", "fn f() -> Int {\n    let p = Point { x: 1, y: 2 }\n    let g = |q: Point| q.x\n    return g(p)\n}\n"),
    ];
    for (name, body) in cases {
        let count = sites(name, body);
        assert!(count <= 1, "{name}");
        assert_eq!(count, 0, "{name} must not use a region");
    }
}

#[test]
fn early_exits_leave_the_region_stack_balanced() {
    let source = format!(
        "{POINT}\nfn early(n: Int) -> Int {{\n    var i = 0\n    var total = 0\n    while i < n {{\n        let q = Point {{ x: i, y: 2 }}\n        if q.x == 3 {{\n            i = i + 1\n            continue\n        }}\n        if q.x > 6 {{\n            break\n        }}\n        total = total + q.x * q.y\n        i = i + 1\n    }}\n    return total\n}}\n\nfn ret(a: Int) -> Int {{\n    let p = Point {{ x: a, y: 1 }}\n    if p.x > 5 {{\n        return p.x\n    }}\n    return p.y\n}}\n\nfn main() {{\n    println(early(20))\n    println(ret(9))\n    println(ret(2))\n    var s = 0\n    var i = 0\n    while i < 200000 {{\n        s = s + ret(i)\n        i = i + 1\n    }}\n    println(s)\n}}\n"
    );
    assert_eq!(run("exits", &source), "36\n9\n1\n19999899991");
}

#[test]
fn a_try_inside_a_region_returns_cleanly() {
    let source = format!(
        "{POINT}\nfn pick(v: Int) -> Result<Int, Error> {{\n    if v > 0 {{\n        return Ok(v)\n    }}\n    return Err(Error.new(ErrorKind.Other, \"neg\"))\n}}\n\nfn f(v: Int) -> Result<Int, Error> {{\n    let p = Point {{ x: v, y: 1 }}\n    let got = pick(p.x)?\n    return Ok(got + p.y)\n}}\n\nfn main() {{\n    println(f(4).is_ok())\n    println(f(-1).is_err())\n    var i = 0\n    var bad = 0\n    while i < 100000 {{\n        if f(-1).is_ok() {{\n            bad = bad + 1\n        }}\n        i = i + 1\n    }}\n    println(bad)\n}}\n"
    );
    assert_eq!(run("try", &source), "true\ntrue\n0");
}

const RECURSIVE: &str = "fn tag(p: Point) -> Int {\n    let q = p\n    return q.x\n}\n\nfn f(n: Int) -> Int {\n    let p = Point { x: n, y: 1 }\n    if n == 0 {\n        return p.y\n    }\n    return f(n - 1) + tag(p)\n}\n";

#[test]
fn a_recursive_function_places_its_struct_in_a_region() {
    assert_eq!(sites("recursive", RECURSIVE), 1);
}

#[test]
fn deep_recursion_takes_a_region_per_frame_and_frees_them() {
    let source = format!("{POINT}\n{RECURSIVE}\nfn main() {{\n    println(f(200000))\n    println(f(10))\n}}\n");
    assert_eq!(run("deep", &source), "20000100001\n56");
}

#[test]
fn runaway_recursion_with_regions_faults() {
    let dir = dir("runaway");
    let source = format!("{POINT}\nfn g(n: Int) -> Int {{\n    let p = Point {{ x: n, y: 1 }}\n    return g(n + 1) + p.x\n}}\n\nfn main() {{\n    println(g(0))\n}}\n");
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("stack overflow"), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn a_struct_over_63_slots_stays_on_the_heap() {
    let fields = |n: usize| (0..n).map(|i| format!("    var f{i}: Int\n")).collect::<String>();
    let init = |n: usize| (0..n).map(|i| format!("f{i}: {i}, ")).collect::<String>();
    for (n, expected) in [(63, 1), (64, 0)] {
        let source = format!("struct Wide {{\n{}}}\n\nfn f() -> Int {{\n    let w = Wide {{ {}}}\n    return w.f0 + w.f{}\n}}\n", fields(n), init(n), n - 1);
        assert_eq!(region_sites(&format!("wide{n}"), &source), expected, "{n} fields");
    }
}

#[test]
fn a_heap_value_held_only_by_a_region_object_survives_collections() {
    let source = "struct Holder {\n    var name: String\n    var n: Int\n}\n\nfn churn(k: Int) -> Int {\n    var t = 0\n    var i = 0\n    while i < k {\n        let xs = [i, i, i]\n        t += xs.len()\n        i += 1\n    }\n    return t\n}\n\nfn name_of(h: Holder) -> String {\n    let k = h\n    return k.name\n}\n\nfn f(tag: Int) -> String {\n    let h = Holder { name: \"item ${tag}\", n: tag }\n    let junk = churn(300000)\n    return name_of(h)\n}\n\nfn main() {\n    println(f(7))\n}\n";
    assert_eq!(region_sites("holder", source), 1);
    assert_eq!(run("holder", source), "item 7");
}
