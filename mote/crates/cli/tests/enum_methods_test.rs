//! Methods on enums, trait lists on enums, and `@derive` on enums.

use std::process::Command;

fn run_files(files: &[(&str, &str)], tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_enum_methods_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, src) in files {
        std::fs::write(dir.join(name), src).unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn run(source: &str, tag: &str) -> (bool, String) {
    run_files(&[("main.mote", source)], tag)
}

fn rejected(source: &str, tag: &str, message: &str) {
    let (ok, text) = run(source, tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

const LIGHT: &str = "enum Light {\n    Red\n    Green\n\n    fn next(self) -> Light {\n        match self {\n            Light.Red => return Light.Green\n            Light.Green => return Light.Red\n        }\n    }\n\n    fn start() -> Light { return Light.Red }\n}\n";

const MACHINE: &str = include_str!("programs_mote/enum_state_machine.mote");

#[test]
fn done_bar_a_state_machine_with_a_trait_and_a_bound_runs() {
    let (ok, text) = run(MACHINE, "machine");
    assert!(ok && text == "red\ngreen\nyellow\nred\ngreen!\n2\n0\n", "{text}");
}

#[test]
fn an_instance_method_reads_its_variant() {
    let src = "enum Shape {\n    Circle(Float)\n    Rect { w: Float, h: Float }\n\n    fn area(self) -> Float {\n        match self {\n            Shape.Circle(r) => return 3.0 * r * r\n            Shape.Rect { w, h } => return w * h\n        }\n    }\n}\nfn main() {\n    println(Shape.Circle(2.0).area())\n    println(Shape.Rect { w: 2.0, h: 3.0 }.area())\n}\n";
    let (ok, text) = run(src, "instance");
    assert!(ok && text.trim() == "12.0\n6.0", "{text}");
}

#[test]
fn a_transition_method_returns_the_next_value() {
    let src = format!("{LIGHT}fn main() {{\n    let l = Light.start()\n    println(l.next())\n    println(l.next().next())\n}}\n");
    let (ok, text) = run(&src, "transition");
    assert!(ok && text.trim() == "Green\nRed", "{text}");
}

#[test]
fn a_generic_enum_has_methods() {
    let src = "enum Tree<T> {\n    Leaf\n    Node(T, Int)\n\n    fn size(self) -> Int {\n        match self {\n            Tree.Leaf => return 0\n            Tree.Node(v, n) => return n\n        }\n    }\n}\nfn main() {\n    let t: Tree<Int> = Tree.Node(5, 2)\n    println(t.size())\n    println(Tree.Leaf.size())\n}\n";
    let (ok, text) = run(src, "generic");
    assert!(ok && text.trim() == "2\n0", "{text}");
}

#[test]
fn a_method_takes_a_struct_and_a_default() {
    let src = "struct P {\n    x: Int\n    y: Int\n}\nenum E {\n    A(Int)\n    B\n\n    fn shift(self, p: P, step: Int = 2) -> Int {\n        match self {\n            E.A(v) => return v + p.x + p.y + step\n            E.B => return p.x\n        }\n    }\n}\nfn main() {\n    let p = P { x: 1, y: 2 }\n    println(E.A(4).shift(p))\n    println(E.B.shift(p, 9))\n}\n";
    let (ok, text) = run(src, "struct_arg");
    assert!(ok && text.trim() == "9\n1", "{text}");
}

#[test]
fn an_enum_method_wins_over_the_option_combinators() {
    let src = "enum Opt {\n    Some(Int)\n    None\n\n    fn map(self, k: Int) -> Int {\n        match self {\n            Opt.Some(v) => return v * k\n            Opt.None => return 0\n        }\n    }\n}\nfn main() {\n    println(Opt.Some(4).map(3))\n}\n";
    let (ok, text) = run(src, "shadow");
    assert!(ok && text.trim() == "12", "{text}");
}

#[test]
fn a_user_to_string_is_what_interpolation_prints() {
    let src = "enum Level {\n    Low\n    High\n\n    fn to_string(self) -> String {\n        match self {\n            Level.Low => return \"low\"\n            Level.High => return \"high\"\n        }\n    }\n}\nfn main() {\n    println(\"level ${Level.High} and ${Level.Low}\")\n}\n";
    let (ok, text) = run(src, "interp");
    assert!(ok && text.trim() == "level high and low", "{text}");
}

#[test]
fn a_method_cannot_share_a_variants_name() {
    rejected("enum E {\n    A\n    B\n\n    fn A(self) -> Int { return 1 }\n}\nfn main() { }\n", "clash", "`E` has a variant named `A`, so a method cannot have that name");
}

#[test]
fn var_self_is_refused_on_an_enum() {
    rejected("enum E {\n    A\n    B\n\n    fn flip(var self) -> Int { return 1 }\n}\nfn main() { }\n", "var_self", "`var self` is not allowed");
}

#[test]
fn variants_come_before_methods() {
    rejected("enum E {\n    A\n    fn f(self) -> Int { return 1 }\n    B\n}\nfn main() { }\n", "order", "variants come before methods in an enum body");
}

#[test]
fn an_unknown_method_is_an_error() {
    rejected(&format!("{LIGHT}fn main() {{\n    println(Light.Red.nope())\n}}\n"), "unknown", "no method `.nope()` on `Light`");
}

#[test]
fn a_private_method_is_private_to_its_module() {
    let shapes = "pub enum Light {\n    Red\n    Green\n\n    pub fn next(self) -> Light {\n        match self {\n            Light.Red => return Light.Green\n            Light.Green => return Light.Red\n        }\n    }\n\n    fn secret(self) -> Int { return 1 }\n}\n";
    let (ok, text) = run_files(&[("shapes.mote", shapes), ("main.mote", "import { Light } from .shapes\nfn main() {\n    println(Light.Red.next())\n}\n")], "pub_ok");
    assert!(ok && text.trim() == "Green", "{text}");
    let (ok, text) = run_files(&[("shapes.mote", shapes), ("main.mote", "import { Light } from .shapes\nfn main() {\n    println(Light.Red.secret())\n}\n")], "pub_no");
    assert!(!ok && text.contains("private"), "{text}");
}

#[test]
fn an_enum_lists_a_trait_and_gets_its_defaults() {
    let src = "trait Named {\n    fn name(self) -> String\n    fn shout(self) -> String { return \"${self.name()}!\" }\n}\nenum Light: (Named) {\n    Red\n    Green\n\n    fn name(self) -> String {\n        match self {\n            Light.Red => return \"red\"\n            Light.Green => return \"green\"\n        }\n    }\n}\nfn main() {\n    println(Light.Green.shout())\n}\n";
    let (ok, text) = run(src, "listed");
    assert!(ok && text.trim() == "green!", "{text}");
}

#[test]
fn an_enum_satisfies_a_bound() {
    let src = "trait Named {\n    fn name(self) -> String\n}\nenum Light: (Named) {\n    Red\n\n    fn name(self) -> String { return \"red\" }\n}\nfn describe<T: Named>(x: T) -> String { return x.name() }\nfn main() {\n    println(describe(Light.Red))\n}\n";
    let (ok, text) = run(src, "bound");
    assert!(ok && text.trim() == "red", "{text}");
}

#[test]
fn a_listed_trait_with_a_missing_method_is_an_error() {
    rejected("trait Named {\n    fn name(self) -> String\n}\nenum Light: (Named) {\n    Red\n}\nfn main() { }\n", "missing", "`Light` does not implement `Named`: missing method `name`");
}

#[test]
fn derive_display_and_debug_work_on_an_enum() {
    let src = "@derive(Display, Debug)\nenum Shape {\n    Circle(Float)\n    Rect { w: Float, h: Float }\n    Empty\n}\nfn main() {\n    println(Shape.Circle(1.5).to_string())\n    println(Shape.Rect { w: 2.0, h: 3.0 }.debug())\n    println(\"${Shape.Empty}\")\n}\n";
    let (ok, text) = run(src, "derive");
    assert!(ok && text.trim() == "Circle(1.5)\nRect { w: 2.0, h: 3.0 }\nEmpty", "{text}");
}

#[test]
fn derive_over_an_existing_enum_method_is_an_error() {
    rejected("@derive(Display)\nenum E {\n    A\n\n    fn to_string(self) -> String { return \"a\" }\n}\nfn main() { }\n", "derive_clash", "already has a method named `to_string`");
}

#[test]
fn a_derived_enum_satisfies_display() {
    let src = "@derive(Display)\nenum Level {\n    Low\n    High\n}\nfn show<T: Display>(x: T) -> String { return x.to_string() }\nfn main() {\n    println(show(Level.High))\n}\n";
    let (ok, text) = run(src, "derive_bound");
    assert!(ok && text.trim() == "High", "{text}");
}
