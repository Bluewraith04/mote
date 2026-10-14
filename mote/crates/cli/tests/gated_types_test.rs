//! `Set`, `Stream` and `Task` need their std module imported.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_gated_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn a_use_without_the_import_names_the_import() {
    for (body, name, module) in [
        ("let s: Set<Int> = Set()\nprintln(s.len())\n", "Set", "std.collections"),
        ("fn f() -> Stream<Int> { yield 1 }\nprintln(1)\n", "Stream", "std.stream"),
        ("scope {\n    let t: Task<Int> = spawn { 1 }\n    println(t.join().unwrap())\n}\n", "Task", "std.task"),
    ] {
        let (ok, text) = run(body, name);
        assert!(!ok && text.contains(&format!("`{name}` is not in scope; `import {{ {name} }} from {module}`")), "{name}: {text}");
    }
}

#[test]
fn the_selective_import_brings_the_type_in() {
    let (ok, text) = run("import { Set } from std.collections\nlet s: Set<Int> = Set()\nprintln(s.len())\n", "selective");
    assert!(ok && text == "0\n", "{text}");
}

#[test]
fn any_form_of_the_module_import_works() {
    for (import, tag) in [("import std.collections", "plain"), ("import std.collections as c", "alias"), ("import { * } from std.collections", "glob")] {
        let (ok, text) = run(&format!("{import}\nlet s: Set<Int> = Set()\nprintln(s.len())\n"), tag);
        assert!(ok && text == "0\n", "{tag}: {text}");
    }
}

#[test]
fn a_type_of_the_same_name_in_the_module_needs_no_import() {
    let (ok, text) = run("struct Task {\n    n: Int\n}\nlet t = Task { n: 3 }\nprintln(t.n)\n", "own");
    assert!(ok && text == "3\n", "{text}");
}

#[test]
fn a_generator_in_another_module_needs_the_import_there() {
    let dir = std::env::temp_dir().join(format!("mote_gated_two_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("gen.mote"), "pub fn one() -> Stream<Int> { yield 1 }\n").unwrap();
    std::fs::write(dir.join("main.mote"), "import { Stream } from std.stream\nimport { one } from gen\nfor x in one() { println(x) }\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(!out.status.success() && text.contains("`Stream` is not in scope"), "{text}");
}
