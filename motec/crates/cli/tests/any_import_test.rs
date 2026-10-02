//! `Any` is not in scope until imported from `std.experimental.types`; std code uses unions and generics.

use std::process::Command;

fn mote(cmd: &str, source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_any_import_{}_{}_{}", cmd, tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg(cmd).arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn any_needs_its_import() {
    let (ok, text) = mote("check", "pub fn main() {\n    let a: Any = 1\n}\n", "bare");
    assert!(!ok && text.contains("`Any` is not in scope; `import { Any } from std.experimental.types`"), "{text}");
}

#[test]
fn any_works_once_imported() {
    let src = "import { Any } from std.experimental.types\npub fn main() {\n    let a: Any = 5\n    let n: Int = a\n    println(n + 1)\n}\n";
    let (ok, text) = mote("run", src, "imported");
    assert!(ok && text == "6\n", "{text}");
}

#[test]
fn print_and_typeof_take_any_value_without_any() {
    let (ok, text) = mote("run", "pub fn main() {\n    println([1, 2])\n    println(typeof(\"s\"))\n    print(3)\n    println(null)\n}\n", "print");
    assert!(ok && text == "[1, 2]\nString\n3null\n", "{text}");
}

#[test]
fn a_map_of_a_union_needs_no_any() {
    let src = "pub fn main() {\n    let m: Map<String, Int | String> = {\"a\": 1, \"b\": \"x\"}\n    println(m[\"a\"] == 1)\n}\n";
    let (ok, text) = mote("run", src, "union");
    assert!(ok && text == "true\n", "{text}");
}

#[test]
fn std_names_any_in_one_place_only() {
    let std = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../modules/std");
    let files: Vec<_> = std::fs::read_dir(&std).unwrap().map(|e| e.unwrap().path()).collect();
    for f in files.iter().filter(|f| f.extension().is_some_and(|e| e == "mote")) {
        let text = std::fs::read_to_string(f).unwrap();
        let uses = text.lines().filter(|l| !l.trim_start().starts_with("//") && l.split(|c: char| !c.is_alphanumeric() && c != '_').any(|w| w == "Any")).count();
        let name = f.file_name().unwrap().to_string_lossy();
        assert!(uses == 0 || name == "error.mote", "{name} names `Any` {uses} time(s)");
    }
}
