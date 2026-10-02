//! The `@name`/`@name(args)` attribute parser, its closed registry, and `@derive(Display, Debug)`.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_derive_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn check(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_derive_chk_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("check").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn rejected(source: &str, tag: &str, message: &str) {
    let (ok, text) = check(source, tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

#[test]
fn derive_display_generates_a_name_and_fields_to_string() {
    let src = "@derive(Display)\nclass Point {\n    x: Int\n    y: Int\n}\nfn main() {\n    println(Point { x: 3, y: 4 }.to_string())\n}\n";
    let (ok, text) = run(src, "display");
    assert!(ok && text.trim() == "Point(x: 3, y: 4)", "{text}");
}

#[test]
fn derive_display_on_a_struct_with_no_fields_is_the_bare_name() {
    let src = "@derive(Display)\nstruct Unit {\n}\nfn main() {\n    println(Unit {}.to_string())\n}\n";
    let (ok, text) = run(src, "unit");
    assert!(ok && text.trim() == "Unit", "{text}");
}

#[test]
fn derive_display_is_used_by_string_interpolation() {
    let src = "@derive(Display)\nclass Point {\n    x: Int\n}\nfn main() {\n    let p = Point { x: 9 }\n    println(\"p is ${p}\")\n}\n";
    let (ok, text) = run(src, "interp");
    assert!(ok && text.trim() == "p is Point(x: 9)", "{text}");
}

#[test]
fn derive_debug_is_the_raw_field_dump_even_with_a_display_override() {
    let src = "@derive(Display, Debug)\nclass Point {\n    x: Int\n}\nfn main() {\n    println(Point { x: 5 }.debug())\n}\n";
    let (ok, text) = run(src, "debug");
    assert!(ok && text.trim() == "Point { x: 5 }", "{text}");
}

#[test]
fn a_struct_may_derive_both_traits_at_once() {
    let src = "@derive(Display, Debug)\nstruct P {\n    n: Int\n}\nfn main() {\n    let p = P { n: 1 }\n    println(p.to_string())\n    println(p.debug())\n}\n";
    let (ok, text) = run(src, "both");
    assert!(ok && text.trim() == "P(n: 1)\nP { n: 1 }", "{text}");
}

#[test]
fn an_unknown_attribute_is_a_compile_error() {
    rejected("@wat\nfn f() {}\nfn main() { }\n", "unknown", "`@wat` is not a known attribute");
}

#[test]
fn derive_on_a_function_is_rejected() {
    rejected("@derive(Display)\nfn f() {}\nfn main() { }\n", "onfn", "can only be used on a `struct`, `class` or `enum`");
}

#[test]
fn deriving_an_unknown_trait_is_rejected() {
    rejected("@derive(Bogus)\nclass X {\n    n: Int\n}\nfn main() { }\n", "unknowntrait", "`Bogus` cannot be derived");
}

#[test]
fn deriving_over_an_existing_method_is_rejected() {
    let src = "@derive(Display)\nclass X {\n    n: Int\n    pub fn to_string(self) -> String { return \"x\" }\n}\nfn main() { }\n";
    rejected(src, "clash", "already has a method named `to_string`");
}

#[test]
fn derive_on_an_enum_is_accepted() {
    let (ok, text) = check("@derive(Display)\nenum E {\n    A\n}\nfn main() { }\n", "onenum");
    assert!(ok, "{text}");
}
