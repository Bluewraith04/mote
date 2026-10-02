//! A struct field of struct type is embedded in its parent's slots; behaviour is unchanged.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    run_files(&[("main.mote", source)], tag)
}

fn run_files(files: &[(&str, &str)], tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_embed_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, src) in files {
        std::fs::write(dir.join(name), src).unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

const TYPES: &str = "struct Point {\n    var x: Int\n    var y: Int\n    pub fn sum(self) -> Int { return self.x + self.y }\n    pub fn shift(var self, d: Int) {\n        self.x += d\n        self.y += d\n    }\n}\n\nstruct Line {\n    var a: Point\n    var b: Point\n    var tag: String\n}\n\nstruct Scene {\n    var line: Line\n    var n: Int\n}\n\nclass Holder {\n    var p: Point\n    var name: String\n}\n\nfn total(p: Point) -> Int {\n    return p.x + p.y\n}\n\nfn bump(var p: Point) {\n    p.x += 100\n}\n";

fn lines(body: &str, tag: &str) -> Vec<String> {
    let source = format!("{TYPES}\nfn main() {{\n    var l = Line {{ a: Point {{ x: 1, y: 2 }}, b: Point {{ x: 3, y: 4 }}, tag: \"t\" }}\n{body}}}\n");
    let (ok, text) = run(&source, tag);
    assert!(ok, "{text}");
    text.lines().map(String::from).collect()
}

#[test]
fn nested_fields_read_and_write_in_place() {
    let out = lines("    println(l.a.x + l.b.y)\n    l.b.y = 40\n    l.a.x += 10\n    println(l.a.x)\n    println(l.b.y)\n", "rw");
    assert_eq!(out, ["5", "11", "40"]);
}

#[test]
fn reading_a_nested_struct_whole_gives_a_copy() {
    let out = lines("    let p = l.a\n    var q = l.a\n    q.x = 99\n    println(p.sum())\n    println(l.a.x)\n    println(q.x)\n    println(total(l.b))\n    println(l.a == p)\n    println(l.a == q)\n", "copy");
    assert_eq!(out, ["3", "1", "99", "7", "true", "false"]);
}

#[test]
fn a_nested_struct_is_assigned_whole() {
    let out = lines("    l.a = Point { x: 7, y: 8 }\n    println(l.a.sum())\n    let q = Point { x: 99, y: 5 }\n    l.b = q\n    println(l.b.x)\n    println(l.b.y)\n", "assign");
    assert_eq!(out, ["15", "99", "5"]);
}

#[test]
fn var_parameters_and_var_self_write_through_a_nested_struct() {
    let out = lines("    l.a.shift(5)\n    println(l.a.x)\n    bump(l.b)\n    println(l.b.x)\n", "var");
    assert_eq!(out, ["6", "103"]);
}

#[test]
fn copying_the_outer_struct_copies_what_is_embedded() {
    let out = lines("    var m = l\n    m.a.x = -1\n    println(l.a.x)\n    println(m.a.x)\n    println(l == m)\n    var s = Scene { line: l, n: 3 }\n    s.line.b.y = 1234\n    println(s.line.b.y)\n    println(l.b.y)\n    s.line.a.shift(1)\n    println(s.line.a.x)\n", "outer");
    assert_eq!(out, ["1", "-1", "false", "1234", "4", "2"]);
}

#[test]
fn a_class_can_hold_an_embedded_struct() {
    let out = lines("    let h = Holder { p: Point { x: 5, y: 6 }, name: \"h\" }\n    h.p.x = 50\n    println(h.p.sum())\n", "class");
    assert_eq!(out, ["56"]);
}

#[test]
fn a_nested_struct_prints_as_a_struct() {
    let out = lines("    println(l.a)\n    println(l)\n", "print");
    assert_eq!(out, ["Point { x: 1, y: 2 }", "Line { a: Point { x: 1, y: 2 }, b: Point { x: 3, y: 4 }, tag: \"t\" }"]);
}

#[test]
fn nested_structs_work_as_keys_elements_and_task_results() {
    let src = "struct V2 {\n    var x: Int\n    var y: Int\n}\n\nstruct Seg {\n    var start: V2\n    var stop: V2\n}\n\nfn len2(s: Seg) -> Int {\n    let dx = s.stop.x - s.start.x\n    let dy = s.stop.y - s.start.y\n    return dx * dx + dy * dy\n}\n\nfn mk(i: Int) -> Seg {\n    return Seg { start: V2 { x: i, y: 0 }, stop: V2 { x: i + 3, y: 4 } }\n}\n\nfn main() {\n    var m = Map<Seg, Int>()\n    m.set(mk(1), 5)\n    println(m.get(mk(1)))\n    var xs = [mk(1), mk(2)]\n    xs[0].stop.y = 50\n    println(xs[0].stop.y)\n    println(xs[1].stop.y)\n    let t = spawn { len2(mk(10)) }\n    println(t.join())\n}\n";
    let (ok, text) = run(src, "coll");
    assert!(ok, "{text}");
    assert_eq!(text.lines().collect::<Vec<_>>(), ["5", "50", "4", "Ok(25)"]);
}

#[test]
fn a_nested_struct_reached_through_any_is_read_whole() {
    let src = "import { Any } from std.experimental.types\n\nstruct V2 {\n    var x: Int\n    var y: Int\n}\n\nstruct Seg {\n    var start: V2\n    var stop: V2\n}\n\nfn pick(a: Any) -> Any {\n    return a.stop\n}\n\nfn main() {\n    let s = Seg { start: V2 { x: 1, y: 2 }, stop: V2 { x: 3, y: 4 } }\n    println(pick(s))\n}\n";
    let (ok, text) = run(src, "any");
    assert!(ok, "{text}");
    assert_eq!(text.trim(), "V2 { x: 3, y: 4 }");
}

#[test]
fn a_generic_struct_keeps_its_embedded_concrete_field() {
    let src = "struct V2 {\n    var x: Int\n    var y: Int\n}\n\nstruct Pair<T> {\n    var first: T\n    var v: V2\n}\n\nfn main() {\n    let p = Pair { first: 7, v: V2 { x: 1, y: 2 } }\n    println(p.v.y + p.first)\n}\n";
    let (ok, text) = run(src, "generic");
    assert!(ok && text.trim() == "9", "{text}");
}

#[test]
fn a_struct_from_another_module_is_embedded() {
    let geo = "pub struct Point {\n    pub var x: Int\n    pub var y: Int\n    pub fn new(x: Int, y: Int) -> Point { return Point { x: x, y: y } }\n}\n";
    let main = "import { Point } from geo\n\nstruct Line {\n    var a: Point\n    var b: Point\n}\n\nfn main() {\n    let l = Line { a: Point.new(1, 2), b: Point.new(3, 4) }\n    println(l.a.x + l.b.y)\n    println(l)\n}\n";
    let (ok, text) = run_files(&[("geo.mote", geo), ("main.mote", main)], "modules");
    assert!(ok, "{text}");
    assert_eq!(text.lines().collect::<Vec<_>>(), ["5", "Line { a: Point { x: 1, y: 2 }, b: Point { x: 3, y: 4 } }"]);
}
