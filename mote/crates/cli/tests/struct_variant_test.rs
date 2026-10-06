//! Enum variants with named fields: construction, `match` with `..`, errors, and module aliases.

use std::process::Command;

fn run_dir(files: &[(&str, &str)], tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_struct_variant_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, src) in files {
        std::fs::write(dir.join(name), src).unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn run(src: &str, tag: &str) -> (bool, String) {
    run_dir(&[("main.mote", src)], tag)
}

const SHAPE: &str = "enum Shape {\n    Circle(Int)\n    Rect { w: Int, h: Int }\n    Named { label: String, size: Int }\n}\n";

#[test]
fn named_field_variants_build_and_match() {
    let src = format!("{SHAPE}fn area(s: Shape) -> Int {{\n    match s {{\n        Shape.Circle(r) => {{ return 3 * r * r }}\n        Shape.Rect {{ w, h }} => {{ return w * h }}\n        Named {{ size: n, .. }} => {{ return n }}\n    }}\n}}\nfn main() {{\n    println(area(Shape.Rect {{ h: 3, w: 4 }}))\n    println(area(Rect {{ w: 2, h: 5 }}))\n    println(area(Named {{ label: \"x\", size: 7 }}))\n    println(Shape.Rect {{ w: 1, h: 2 }} == Rect {{ h: 2, w: 1 }})\n    match Shape.Named {{ size: 1, label: \"hi\" }} {{\n        Shape.Named {{ label, size: _ }} => {{ println(label) }}\n        _ => {{}}\n    }}\n}}\n");
    let (ok, text) = run(&src, "run");
    assert!(ok && text == "12\n10\n7\ntrue\nhi\n", "{text}");
}

#[test]
fn named_field_variant_mistakes_are_errors() {
    let body = |b: &str| format!("{SHAPE}fn main() {{\n{b}\n}}\n");
    for (b, want) in [
        ("    let a = Shape.Rect { w: 1 }", "`Shape.Rect` literal is missing field(s) `h`"),
        ("    let a = Shape.Rect { w: 1, h: \"x\" }", "Type mismatch on 'Shape.Rect' field 'h': expected 'Int', found 'String'"),
        ("    let a = Shape.Rect { w: 1, h: 2, d: 3 }", "`Shape.Rect` has no field `d`"),
        ("    match Shape.Circle(1) {\n        Shape.Rect { w } => {}\n        _ => {}\n    }", "`Shape.Rect` pattern is missing field(s) `h`; add `..` to skip them"),
    ] {
        let (ok, text) = run(&body(b), "err");
        assert!(!ok && text.contains(want), "{b}: {text}");
    }
}

#[test]
fn a_variant_from_another_module_works_through_an_alias() {
    let geo = "pub enum Shape {\n    Rect { w: Int, h: Int }\n}\n";
    let main = "import { Shape } from .geo\nimport .geo as g\nfn main() {\n    let q = g.Shape.Rect { w: 4, h: 5 }\n    match q {\n        Shape.Rect { w, h } => { println(w * h) }\n    }\n    println(q == Shape.Rect { h: 5, w: 4 })\n}\n";
    let (ok, text) = run_dir(&[("geo.mote", geo), ("main.mote", main)], "module");
    assert!(ok && text == "20\ntrue\n", "{text}");
}

#[test]
fn literal_fields_land_in_declaration_order() {
    let src = "struct P {\n    x: Int\n    y: Int\n}\nclass C {\n    a: Int\n    b: String\n}\nfn main() {\n    let p = P { y: 2, x: 1 }\n    println(p.x)\n    let c = C { b: \"hi\", a: 5 }\n    println(c.a)\n}\n";
    let (ok, text) = run(src, "order");
    assert!(ok && text == "1\n5\n", "{text}");
}
