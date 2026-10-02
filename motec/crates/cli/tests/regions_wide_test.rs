//! Copies, calls and lambda bodies keep a struct in a region, and every value stays correct.

use std::process::Command;

fn run(source: &str, tag: &str, extra: &[&str]) -> String {
    let dir = std::env::temp_dir().join(format!("mote_widereg_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.mote");
    std::fs::write(&file, source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(&file).args(extra).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    text
}

fn stat(text: &str, name: &str) -> u64 {
    text.lines()
        .find_map(|l| l.strip_prefix(&format!("{name} = ")))
        .and_then(|v| v.split(' ').next())
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("no {name} in:\n{text}"))
}

const P: &str = "struct P {\n    var x: Int\n    var y: Int\n    pub fn norm(self) -> Int { return self.x + self.y }\n    pub fn bump(var self) { self.x += 1 }\n    pub fn same(self) -> P { return self }\n}\n\n";

#[test]
fn a_returned_struct_is_a_copy_of_the_region_value() {
    let src = format!("{P}fn make(a: Int) -> P {{\n    var p = P {{ x: a, y: 1 }}\n    p.x += 1\n    return p\n}}\n\nfn main() {{\n    var total = 0\n    var i = 0\n    while i < 100000 {{\n        total += make(i).x + make(i).y\n        i += 1\n    }}\n    println(total)\n}}\n");
    let text = run(&src, "returned", &["--mem-stats"]);
    assert!(text.contains("5000150000"), "{text}");
    assert!(stat(&text, "mem.regions.scopes") >= 100000, "{text}");
}

#[test]
fn a_copy_of_a_region_struct_is_independent() {
    let src = format!("{P}fn main() {{\n    var p = P {{ x: 1, y: 2 }}\n    var q = p\n    q.x = 9\n    p.y = 7\n    println(\"${{p.x}} ${{p.y}} ${{q.x}} ${{q.y}}\")\n}}\n");
    let text = run(&src, "copy", &["--mem-stats"]);
    assert!(text.contains("1 7 9 2"), "{text}");
    assert!(stat(&text, "mem.regions.scopes") >= 1, "{text}");
}

#[test]
fn a_callee_that_only_reads_takes_the_region_value() {
    let src = format!("{P}fn dist(a: P, b: P) -> Int {{\n    let c = a\n    let d = b\n    return (c.x - d.x) * (c.x - d.x) + (c.y - d.y) * (c.y - d.y)\n}}\n\nfn main() {{\n    let a = P {{ x: 1, y: 2 }}\n    let b = P {{ x: 4, y: 6 }}\n    println(dist(a, b))\n    println(a.norm() + b.norm())\n}}\n");
    let text = run(&src, "reads", &["--mem-stats"]);
    assert!(text.contains("25\n13"), "{text}");
    assert_eq!(stat(&text, "mem.regions.scopes"), 2, "{text}");
}

#[test]
fn a_var_self_method_writes_the_region_value_in_place() {
    let src = format!("{P}fn main() {{\n    var p = P {{ x: 1, y: 2 }}\n    p.bump()\n    p.bump()\n    println(p.x)\n}}\n");
    let text = run(&src, "varself", &["--mem-stats"]);
    assert!(text.contains("3"), "{text}");
    assert_eq!(stat(&text, "mem.regions.scopes"), 1, "{text}");
}

#[test]
fn a_method_that_returns_self_is_a_copy_the_caller_owns() {
    let src = format!("{P}fn main() {{\n    var p = P {{ x: 1, y: 2 }}\n    var q = p.same()\n    q.x = 50\n    println(\"${{p.x}} ${{q.x}}\")\n}}\n");
    let text = run(&src, "same", &["--mem-stats"]);
    assert!(text.contains("1 50"), "{text}");
    assert_eq!(stat(&text, "mem.regions.scopes"), 1, "{text}");
}

#[test]
fn recursion_that_only_reads_keeps_the_value_in_a_region() {
    let src = format!("{P}fn down(p: P, n: Int) -> Int {{\n    let q = p\n    if n == 0 {{\n        return q.x\n    }}\n    return down(p, n - 1)\n}}\n\nfn main() {{\n    let p = P {{ x: 5, y: 2 }}\n    println(down(p, 100))\n}}\n");
    let text = run(&src, "recursion", &["--mem-stats"]);
    assert!(text.contains("5"), "{text}");
    assert_eq!(stat(&text, "mem.regions.scopes"), 1, "{text}");
}

#[test]
fn mutually_recursive_callees_that_keep_a_parameter_send_the_argument_to_the_heap() {
    let src = format!("{P}fn a(p: P, var xs: List<P>, n: Int) {{\n    if n == 0 {{\n        xs.push(p)\n        return\n    }}\n    b(p, xs, n - 1)\n}}\n\nfn b(p: P, var xs: List<P>, n: Int) {{\n    a(p, xs, n)\n}}\n\nfn main() {{\n    var xs: List<P> = []\n    var p = P {{ x: 5, y: 2 }}\n    a(p, xs, 3)\n    xs[0].x = 1\n    println(\"${{p.x}} ${{xs[0].x}}\")\n}}\n");
    let text = run(&src, "mutual", &["--mem-stats"]);
    assert!(text.contains("5 1"), "{text}");
    assert_eq!(stat(&text, "mem.regions.scopes"), 0, "{text}");
}

#[test]
fn a_spawned_task_works_on_its_own_copy() {
    let src = format!("{P}fn main() {{\n    var p = P {{ x: 1, y: 2 }}\n    let t = spawn {{\n        p.x = 100\n        p.x + p.y\n    }}\n    let r = t.join()\n    println(\"${{p.x}} ${{r!}}\")\n}}\n");
    let text = run(&src, "spawn", &["--mem-stats"]);
    assert!(text.contains("1 102"), "{text}");
    assert!(stat(&text, "mem.regions.scopes") >= 1, "{text}");
}

#[test]
fn lambda_and_spawn_bodies_place_their_own_structs() {
    let src = format!("{P}fn main() {{\n    let f = |n: Int| {{\n        let p = P {{ x: n, y: 1 }}\n        return p.norm()\n    }}\n    let t = spawn {{\n        let q = P {{ x: 7, y: 8 }}\n        q.norm()\n    }}\n    let r = t.join()\n    println(\"${{f(4)}} ${{r!}}\")\n}}\n");
    let text = run(&src, "bodies", &["--mem-stats"]);
    assert!(text.contains("5 15"), "{text}");
    assert_eq!(stat(&text, "mem.regions.scopes"), 2, "{text}");
}

#[test]
fn a_lambda_body_frees_its_region_on_every_return() {
    let src = format!("{P}fn main() {{\n    let f = |n: Int| {{\n        let p = P {{ x: n, y: 1 }}\n        p.x + p.y\n    }}\n    var total = 0\n    var i = 0\n    while i < 300000 {{\n        total += f(i)\n        i += 1\n    }}\n    println(total)\n}}\n");
    let text = run(&src, "lambda_loop", &["--mem-stats"]);
    assert!(text.contains("45000150000"), "{text}");
    assert!(stat(&text, "mem.peak_rss") < 300 * 1024 * 1024, "{text}");
}

#[test]
fn a_struct_matched_by_a_pattern_is_copied_into_the_binding() {
    let src = format!("{P}fn main() {{\n    let p = P {{ x: 3, y: 4 }}\n    match p {{\n        q => {{\n            var r = q\n            r.x = 30\n            println(\"${{p.x}} ${{r.x}}\")\n        }}\n    }}\n}}\n");
    let text = run(&src, "match", &["--mem-stats"]);
    assert!(text.contains("3 30"), "{text}");
}

#[test]
fn a_class_is_shared_so_it_stays_on_the_heap() {
    let src = "class C {\n    var n: Int\n}\n\nfn main() {\n    let a = C { n: 1 }\n    let b = a\n    b.n = 5\n    println(a.n)\n}\n";
    let text = run(src, "class", &["--mem-stats"]);
    assert!(text.contains("5"), "{text}");
    assert_eq!(stat(&text, "mem.regions.scopes"), 0, "{text}");
}

#[test]
fn region_structs_holding_heap_values_survive_collections() {
    let src = "struct H {\n    var s: String\n    var n: Int\n}\n\nfn work(i: Int) -> Int {\n    let h = H { s: \"item ${i}\", n: i }\n    let xs = [i, i, i]\n    return h.s.len() + xs.len() + h.n\n}\n\nfn main() {\n    var total = 0\n    var i = 0\n    while i < 300000 {\n        total += work(i)\n        i += 1\n    }\n    println(total)\n}\n".to_string();
    let text = run(&src, "gc", &["--mem-stats"]);
    assert!(text.contains("45003938890"), "{text}");
    assert!(stat(&text, "mem.gc.collections") > 0, "{text}");
}

#[test]
fn a_returned_parameter_or_capture_is_a_copy() {
    let src = format!("{P}fn keep(p: P) -> P {{\n    return p\n}}\n\nfn main() {{\n    var p = P {{ x: 1, y: 2 }}\n    var q = keep(p)\n    q.x = 50\n    let f = |v: P| v\n    var r = f(p)\n    r.y = 60\n    println(\"${{p.x}} ${{p.y}} ${{q.x}} ${{r.y}}\")\n}}\n");
    let text = run(&src, "param_return", &[]);
    assert!(text.contains("1 2 50 60"), "{text}");
}

#[test]
fn a_tuple_bound_at_once_is_not_built() {
    let src = "fn main() {\n    var total = 0\n    var i = 0\n    while i < 300000 {\n        let (x, y) = (i, i + 1)\n        total += x + y\n        i += 1\n    }\n    let (p, _) = (total, 0)\n    println(p)\n}\n";
    let text = run(src, "tuple_let", &["--mem-stats"]);
    assert!(text.contains("90000000000"), "{text}");
    assert_eq!(stat(&text, "mem.gc.collections"), 0, "{text}");
}
