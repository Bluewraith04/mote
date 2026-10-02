//! Trait declarations, `struct C: (Trait)`, structural checks and default methods.

use std::process::Command;

fn run_files(files: &[(&str, &str)], tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_trait_{}_{}", tag, std::process::id()));
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

const SHAPE: &str = "trait Shape {\n    fn area(self) -> Int\n    fn describe(self) -> String { return \"area ${self.area()}\" }\n}\n";
const SQ: &str = "class Sq: (Shape) {\n    side: Int\n    fn area(self) -> Int { return self.side * self.side }\n}\n";

#[test]
fn a_listed_trait_is_satisfied_by_the_bodys_methods() {
    let src = "trait Shape {\n    fn area(self) -> Int\n}\nclass Sq: (Shape) {\n    side: Int\n    fn area(self) -> Int { return self.side * self.side }\n}\nfn main() {\n    println(Sq { side: 3 }.area())\n}\n";
    let (ok, text) = run(src, "supply");
    assert!(ok && text.trim() == "9", "{text}");
}

#[test]
fn a_default_method_is_copied_into_the_listing_type() {
    let src = format!("{SHAPE}{SQ}fn main() {{\n    println(Sq {{ side: 2 }}.describe())\n}}\n");
    let (ok, text) = run(&src, "default");
    assert!(ok && text.trim() == "area 4", "{text}");
}

#[test]
fn a_type_may_override_a_default() {
    let src = format!("{SHAPE}class Sq: (Shape) {{\n    side: Int\n    fn area(self) -> Int {{ return self.side * self.side }}\n    fn describe(self) -> String {{ return \"square\" }}\n}}\nfn main() {{\n    println(Sq {{ side: 2 }}.describe())\n}}\n");
    let (ok, text) = run(&src, "override");
    assert!(ok && text.trim() == "square", "{text}");
}

#[test]
fn a_type_that_does_not_list_the_trait_gets_no_default() {
    let src = format!("{SHAPE}class Sq {{\n    side: Int\n    fn area(self) -> Int {{ return self.side }}\n}}\nfn main() {{\n    println(Sq {{ side: 2 }}.describe())\n}}\n");
    rejected(&src, "unlisted", "no method `.describe()` on `Sq`");
}

#[test]
fn a_missing_required_method_is_an_error() {
    rejected("trait Shape {\n    fn area(self) -> Int\n}\nclass Sq: (Shape) {\n    side: Int\n}\nfn main() { }\n", "missing", "`Sq` does not implement `Shape`: missing method `area`");
}

#[test]
fn a_mismatched_signature_is_an_error() {
    rejected("trait Shape {\n    fn area(self) -> Int\n}\nclass Sq: (Shape) {\n    side: Int\n    fn area(self) -> String { return \"x\" }\n}\nfn main() { }\n", "mismatch", "`area` returns");
    rejected("trait Shape {\n    fn scale(self, k: Int) -> Int\n}\nclass Sq: (Shape) {\n    side: Int\n    fn scale(self, k: String) -> Int { return 1 }\n}\nfn main() { }\n", "mismatch_param", "`scale` takes");
    rejected("trait Shape {\n    fn area(self) -> Int\n}\nclass Sq: (Shape) {\n    side: Int\n    fn area() -> Int { return 1 }\n}\nfn main() { }\n", "mismatch_self", "must take a `self` receiver");
}

#[test]
fn a_struct_lists_traits_too() {
    let src = "trait Total {\n    fn total(self) -> Int\n}\nstruct P: (Total) {\n    x: Int\n    y: Int\n    fn total(self) -> Int { return self.x + self.y }\n}\nfn main() {\n    println(P { x: 1, y: 2 }.total())\n}\n";
    let (ok, text) = run(src, "struct_list");
    assert!(ok && text.trim() == "3", "{text}");
}

#[test]
fn several_traits_may_be_listed() {
    let src = "trait A {\n    fn a(self) -> Int\n}\ntrait B {\n    fn b(self) -> Int\n}\nstruct P: (A, B) {\n    n: Int\n    fn a(self) -> Int { return self.n }\n    fn b(self) -> Int { return self.n * 2 }\n}\nfn main() {\n    let p = P { n: 3 }\n    println(p.a() + p.b())\n}\n";
    let (ok, text) = run(src, "several");
    assert!(ok && text.trim() == "9", "{text}");
}

#[test]
fn a_generic_type_lists_traits_after_its_parameters() {
    let src = "trait Get {\n    fn get(self) -> Int\n}\nclass Box<T>: (Get) {\n    v: T\n    fn get(self) -> Int { return 7 }\n}\nfn main() {\n    let b = Box { v: 5 }\n    println(b.get())\n}\n";
    let (ok, text) = run(src, "generic_list");
    assert!(ok && text.trim() == "7", "{text}");
}

#[test]
fn self_in_a_trait_means_the_listing_type() {
    let src = "trait Same {\n    fn same(self, other: Self) -> Bool\n}\nclass P: (Same) {\n    n: Int\n    fn same(self, other: P) -> Bool { return self.n == other.n }\n}\nfn main() {\n    println(P { n: 1 }.same(P { n: 1 }))\n}\n";
    let (ok, text) = run(src, "self_type");
    assert!(ok && text.trim() == "true", "{text}");
    rejected("trait Same {\n    fn same(self, other: Self) -> Bool\n}\nclass P: (Same) {\n    n: Int\n    fn same(self, other: Int) -> Bool { return true }\n}\nfn main() { }\n", "self_bad", "`same` takes");
}

#[test]
fn listing_something_that_is_not_a_trait_is_an_error() {
    rejected("class C {\n    n: Int\n}\nclass D: (C) {\n    n: Int\n}\nfn main() { }\n", "not_trait", "`C` is not a trait");
}

#[test]
fn a_generic_trait_is_refused_for_now() {
    rejected("trait Box<T> {\n    fn get(self) -> Int\n}\nfn main() { }\n", "generic_trait", "has type parameters, which are not supported yet");
}

#[test]
fn a_trait_and_its_default_work_across_modules() {
    let (ok, text) = run_files(
        &[
            ("shapes.mote", "pub trait Shape {\n    fn area(self) -> Int\n    fn describe(self) -> String { return \"area ${self.area()}\" }\n}\n"),
            ("main.mote", "import { Shape } from .shapes\nclass Sq: (Shape) {\n    side: Int\n    fn area(self) -> Int { return self.side * self.side }\n}\nfn main() {\n    println(Sq { side: 3 }.describe())\n}\n"),
        ],
        "cross_module",
    );
    assert!(ok && text.trim() == "area 9", "{text}");
}

#[test]
fn a_listed_traits_methods_are_pub() {
    let (ok, text) = run_files(
        &[
            ("shapes.mote", "pub trait Area {\n    fn area(self) -> Int\n}\npub class Sq: (Area) {\n    side: Int\n    pub fn new(side: Int) -> Sq { return Sq { side: side } }\n    fn area(self) -> Int { return self.side * self.side }\n}\n"),
            ("main.mote", "import { Sq } from .shapes\nfn main() {\n    println(Sq.new(4).area())\n}\n"),
        ],
        "listed_pub",
    );
    assert!(ok && text.trim() == "16", "{text}");
}

#[test]
fn impl_is_gone() {
    rejected("class C {\n    n: Int\n}\nimpl C {\n    fn f(self) -> Int { return 1 }\n}\nfn main() { }\n", "no_impl", "there is no `impl`: write the methods in the type's body and list traits as `struct T: (Trait)`");
}

#[test]
fn a_class_has_no_parent() {
    rejected("class A {\n    n: Int\n}\nclass B: A {\n    m: Int\n}\nfn main() { }\n", "no_parent", "a class has no parent: list traits as `class C: (Trait)`");
}

#[test]
fn a_trait_list_needs_parentheses() {
    rejected("trait T {\n    fn f(self) -> Int\n}\nstruct S: T {\n    n: Int\n}\nfn main() { }\n", "no_parens", "list traits in parentheses: `struct C: (Trait)`");
}
