//! `@stable` placement: a `pub` item of a supported kind, no arguments, `std` only.

use std::process::Command;

fn check(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_stable_{}_{}", tag, std::process::id()));
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
fn stable_on_a_user_module_pub_fn_is_rejected() {
    let src = "@stable\npub fn foo() -> Int { return 1 }\npub fn main() {\n    println(foo().to_string())\n}\n";
    rejected(src, "user_module", "only legal inside `std`");
}

#[test]
fn stable_on_a_private_item_is_rejected() {
    let src = "@stable\nfn foo() -> Int { return 1 }\npub fn main() {\n    println(foo().to_string())\n}\n";
    rejected(src, "private_item", "only legal on a `pub` item");
}

#[test]
fn stable_on_an_import_is_rejected() {
    let src = "@stable\nimport std.sys.fs\npub fn main() {\n    println(\"ok\")\n}\n";
    rejected(src, "bad_kind", "can only be used on a `fn`, `struct`, `class`, `enum` or `trait`");
}

#[test]
fn stable_with_arguments_is_rejected() {
    let src = "@stable(foo)\npub fn f() -> Int { return 1 }\npub fn main() {\n    println(f().to_string())\n}\n";
    rejected(src, "with_args", "takes no arguments");
}

#[test]
fn unknown_attribute_is_still_rejected() {
    let src = "@bogus\npub fn f() -> Int { return 1 }\npub fn main() {\n    println(f().to_string())\n}\n";
    rejected(src, "unknown_attr", "not a known attribute");
}

#[test]
fn a_program_with_no_stable_attribute_is_unaffected() {
    let src = "pub fn main() {\n    println(\"fine\")\n}\n";
    let (ok, text) = check(src, "plain");
    assert!(ok, "{text}");
}
