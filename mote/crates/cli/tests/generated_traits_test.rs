//! The attribute parser and its closed registry, and the generated traits `Display` and `Debug` in a trait list.

use std::process::Command;

const IMPORT: &str = "import { Display, Debug } from std.traits\n";

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_generated_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), format!("{IMPORT}{source}")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn check(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_generated_chk_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), format!("{IMPORT}{source}")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("check").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn rejected(source: &str, tag: &str, message: &str) {
    let (ok, text) = check(source, tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

#[test]
fn display_generates_a_name_and_fields_to_string() {
    let src = "class Point: (Display) {\n    x: Int\n    y: Int\n}\nfn main() {\n    println(Point { x: 3, y: 4 }.to_string())\n}\n";
    let (ok, text) = run(src, "display");
    assert!(ok && text.trim() == "Point(x: 3, y: 4)", "{text}");
}

#[test]
fn display_on_a_struct_with_no_fields_is_the_bare_name() {
    let src = "struct Unit: (Display) {\n}\nfn main() {\n    println(Unit {}.to_string())\n}\n";
    let (ok, text) = run(src, "unit");
    assert!(ok && text.trim() == "Unit", "{text}");
}

#[test]
fn display_is_used_by_string_interpolation() {
    let src = "class Point: (Display) {\n    x: Int\n}\nfn main() {\n    let p = Point { x: 9 }\n    println(\"p is ${p}\")\n}\n";
    let (ok, text) = run(src, "interp");
    assert!(ok && text.trim() == "p is Point(x: 9)", "{text}");
}

#[test]
fn debug_is_the_raw_field_dump_even_with_a_display_override() {
    let src = "class Point: (Display, Debug) {\n    x: Int\n}\nfn main() {\n    println(Point { x: 5 }.debug())\n}\n";
    let (ok, text) = run(src, "debug");
    assert!(ok && text.trim() == "Point { x: 5 }", "{text}");
}

#[test]
fn a_struct_may_list_both_traits_at_once() {
    let src = "struct P: (Display, Debug) {\n    n: Int\n}\nfn main() {\n    let p = P { n: 1 }\n    println(p.to_string())\n    println(p.debug())\n}\n";
    let (ok, text) = run(src, "both");
    assert!(ok && text.trim() == "P(n: 1)\nP { n: 1 }", "{text}");
}

#[test]
fn an_unknown_attribute_is_a_compile_error() {
    rejected("@wat\nfn f() {}\nfn main() { }\n", "unknown", "`@wat` is not a known attribute");
}

#[test]
fn derive_is_gone() {
    rejected("@derive(Display)\nclass X {\n    n: Int\n}\nfn main() { }\n", "derive", "there is no `@derive`: list the trait, `struct P: (Display)`");
}

#[test]
fn listing_an_unknown_trait_is_rejected() {
    let (ok, text) = check("class X: (Bogus) {\n    n: Int\n}\nfn main() { }\n", "unknowntrait");
    assert!(!ok && text.contains("Bogus"), "{text}");
}

#[test]
fn a_written_method_is_kept_and_the_trait_is_not_generated() {
    let src = "class X: (Display) {\n    n: Int\n    pub fn to_string(self) -> String { return \"x\" }\n}\nfn main() { println(X { n: 1 }.to_string()) }\n";
    let (ok, text) = run(src, "own");
    assert!(ok && text.trim() == "x", "{text}");
}

#[test]
fn a_listed_trait_works_on_an_enum() {
    let (ok, text) = check("enum E: (Display) {\n    A\n}\nfn main() { }\n", "onenum");
    assert!(ok, "{text}");
}

#[test]
fn a_listed_trait_needs_its_module_imported() {
    let dir = std::env::temp_dir().join(format!("mote_generated_scope_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), "struct P: (Display) {\n    x: Int\n}\nfn main() { }\n").unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_mote")).arg("check").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(!out.status.success() && text.contains("`Display` is not in scope; `import { Display } from std.traits`"), "{text}");
}
