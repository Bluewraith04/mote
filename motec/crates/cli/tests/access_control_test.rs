//! Privacy is per module; fields, methods, literals and `pub` signatures.

use std::process::Command;

const SHAPES: &str = "pub struct P {\n    pub x: Int\n    y: Int\n    pub var z: Int\n    pub fn new(x: Int, y: Int) -> P { return P { x: x, y: y, z: 0 } }\n    pub fn sum(self) -> Int { return self.x + self.y }\n    fn hidden(self) -> Int { return self.y }\n}\n";

fn run_files(files: &[(&str, &str)], tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_access_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, src) in files {
        std::fs::write(dir.join(name), src).unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn rejected(shapes: &str, main: &str, tag: &str, message: &str) {
    let (ok, text) = run_files(&[("shapes.mote", shapes), ("main.mote", main)], tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

fn main_with(body: &str) -> String {
    format!("import {{ P }} from .shapes\nimport {{ Any }} from std.experimental.types\nfn main() {{\n    var p = P.new(1, 2)\n{body}\n}}\n")
}

#[test]
fn public_members_work_across_modules() {
    let main = main_with("    println(p.x)\n    println(p.z)\n    println(p.sum())");
    let (ok, text) = run_files(&[("shapes.mote", SHAPES), ("main.mote", &main)], "public");
    assert!(ok && text == "1\n0\n3\n", "{text}");
}

#[test]
fn a_private_field_is_unreadable_from_another_module() {
    rejected(SHAPES, &main_with("    println(p.y)"), "field_read", "field `y` of `P` is private; mark it `pub`");
}

#[test]
fn a_pub_field_is_never_writable_from_another_module() {
    rejected(SHAPES, &main_with("    p.z = 5"), "field_write", "field `z` of `P` can only be written in its module");
}

#[test]
fn what_a_pub_field_holds_cannot_change_from_another_module() {
    let shapes = "pub class Bag {\n    pub var items: List<Int>\n    pub fn new() -> Bag { return Bag { items: [] } }\n}\n";
    let main = "import { Bag } from .shapes\nfn main() {\n    let b = Bag.new()\n    b.items.push(1)\n}\n";
    rejected(shapes, main, "field_holds", "field `items` of `Bag` can only be written in its module");
}

#[test]
fn a_private_method_is_uncallable_from_another_module() {
    rejected(SHAPES, &main_with("    println(p.hidden())"), "method", "method `hidden` of `P` is private; mark it `pub`");
}

#[test]
fn a_literal_outside_its_module_names_the_constructors() {
    rejected(SHAPES, &main_with("    let q = P { x: 1, y: 2, z: 3 }"), "literal", "can only be written in its module; use `P.new`");
}

#[test]
fn a_type_with_no_constructor_says_so() {
    let shapes = "pub struct Q {\n    pub n: Int\n}\n";
    let main = "import { Q } from .shapes\nfn main() {\n    let q = Q { n: 1 }\n}\n";
    rejected(shapes, main, "no_ctor", "`Q` has no public constructor");
}

#[test]
fn a_pub_function_cannot_name_a_private_type() {
    let shapes = "struct Secret {\n    n: Int\n}\npub fn leak() -> Secret { return Secret { n: 1 } }\n";
    rejected(shapes, "import { leak } from .shapes\nfn main() { }\n", "pub_fn", "`fn leak` is `pub` but its type `Secret` is not");
}

#[test]
fn a_pub_field_and_a_pub_global_cannot_name_a_private_type() {
    let field = "struct Secret {\n    n: Int\n}\npub struct Holder {\n    pub s: Secret\n}\n";
    rejected(field, "import { Holder } from .shapes\nfn main() { }\n", "pub_field", "`Holder.s` is `pub` but its type `Secret` is not");
    let global = "struct Secret {\n    n: Int\n}\npub let s = Secret { n: 1 }\n";
    rejected(global, "import { s } from .shapes\nfn main() { }\n", "pub_global", "`s` is `pub` but its type `Secret` is not");
}

#[test]
fn a_listed_trait_method_is_callable_from_another_module() {
    let shapes = "pub trait Area {\n    fn area(self) -> Int\n}\npub class Sq: (Area) {\n    side: Int\n    pub fn new(side: Int) -> Sq { return Sq { side: side } }\n    fn area(self) -> Int { return self.side * self.side }\n}\n";
    let main = "import { Sq } from .shapes\nfn main() {\n    println(Sq.new(3).area())\n}\n";
    let (ok, text) = run_files(&[("shapes.mote", shapes), ("main.mote", main)], "trait_impl");
    assert!(ok && text.trim() == "9", "{text}");
}

#[test]
fn a_private_field_read_through_any_faults_at_run_time() {
    let main = main_with("    let a: Any = p\n    println(a.x)\n    println(a.y)");
    let (ok, text) = run_files(&[("shapes.mote", SHAPES), ("main.mote", &main)], "any_read");
    assert!(!ok && text.starts_with("1\n") && text.contains("field 'y' is private to its module"), "{text}");
}

#[test]
fn a_field_written_through_any_faults_at_run_time() {
    let main = main_with("    let a: Any = p\n    a.z = 4");
    let (ok, text) = run_files(&[("shapes.mote", SHAPES), ("main.mote", &main)], "any_write");
    assert!(!ok && text.contains("field 'z' can only be written in its module"), "{text}");
}

#[test]
fn a_module_reaches_its_own_fields_through_any() {
    let src = "import { Any } from std.experimental.types\nstruct P {\n    y: Int\n}\nfn main() {\n    let a: Any = P { y: 3 }\n    println(a.y)\n}\n";
    let (ok, text) = run_files(&[("main.mote", src)], "any_own");
    assert!(ok && text.trim() == "3", "{text}");
}
