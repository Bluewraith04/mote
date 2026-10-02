//! `import.m as alias` reaches every export: functions (as values too), globals, types, variants and struct literals.

use std::process::Command;

const GEO: &str = "pub struct Pt {\n    pub x: Int\n    pub y: Int\n    pub fn new(x: Int, y: Int) -> Pt { return Pt { x: x, y: y } }\n}\n\npub fn origin() -> Pt {\n    return Pt { x: 0, y: 0 }\n}\n\npub class Counter {\n    pub var n: Int\n    pub fn new() -> Counter {\n        return Counter { n: 0 }\n    }\n    pub fn bump(var self) {\n        self.n = self.n + 1\n    }\n}\n\npub enum Shape {\n    Dot\n    Box(Int)\n}\n\npub let LIMIT = 7\n";

fn run(main: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_alias_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("geo.mote"), GEO).unwrap();
    std::fs::write(dir.join("main.mote"), main).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn an_alias_reaches_types_values_and_variants() {
    let main = "import .geo as g\n\nfn main() {\n    var c = g.Counter.new()\n    c.bump()\n    println(c.n)\n    let f = g.origin\n    let o: g.Pt = f()\n    println(o.y)\n    println(g.LIMIT)\n    let p = g.Pt.new(1, 2)\n    println(p.y)\n    let s = g.Shape.Box(3)\n    match s {\n        g.Shape.Box(n) => println(n)\n        g.Shape.Dot => println(0)\n    }\n}\n";
    let (ok, text) = run(main, "all");
    assert!(ok && text == "1\n0\n7\n2\n3\n", "{text}");
}

#[test]
fn an_unknown_member_type_names_the_module() {
    let (ok, text) = run("import .geo as g\n\nfn main() {\n    let p: g.Nope = g.origin()\n}\n", "unknown");
    assert!(!ok && text.contains("has no exported type 'Nope'"), "{text}");
}
