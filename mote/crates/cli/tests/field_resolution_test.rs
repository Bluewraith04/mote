//! Field access on receivers whose type is only inferred, through `mote run`: each read hits its own field.

use std::process::Command;

fn run(name: &str, source: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_fields_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

const POINT: &str = "struct P {\n    a: Int\n    b: Int\n}\n";

#[test]
fn a_for_loop_variable_reads_its_own_field() {
    let source = format!("{POINT}\nvar s = 0\nfor x in [P {{ a: 1, b: 2 }}, P {{ a: 3, b: 4 }}] {{ s = s + x.b }}\nprintln(s)\n");
    assert_eq!(run("for", &source), "6");
}

#[test]
fn a_collection_accessor_result_reads_its_own_field() {
    let source = format!("{POINT}\nlet xs = [P {{ a: 1, b: 2 }}, P {{ a: 3, b: 4 }}]\nlet y = xs.get(1)\nprintln(y.b)\n");
    assert_eq!(run("get", &source), "4");
}

#[test]
fn a_nested_field_reads_its_own_field() {
    let source = "struct In {\n    a: Int\n    b: Int\n}\nstruct Out {\n    x: Int\n    inner: In\n}\n\
                  let o = Out { x: 9, inner: In { a: 1, b: 2 } }\nprintln(o.inner.b)\n";
    assert_eq!(run("nested", source), "2");
}

#[test]
fn a_struct_from_another_module_reads_its_own_field() {
    let dir = std::env::temp_dir().join(format!("mote_fields_mod_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("shapes.mote"), "pub struct P {\n    pub a: Int\n    pub b: Int\n}\npub fn make() -> P {\n    return P { a: 1, b: 2 }\n}\n").unwrap();
    std::fs::write(dir.join("main.mote"), "import { make } from .shapes\nlet ps = [make(), make()]\nvar s = 0\nfor p in ps { s = s + p.b }\nprintln(s)\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "4");
}

#[test]
fn a_region_placed_struct_reads_its_own_field() {
    let source = "struct P {\n    x: Int\n    var y: Int\n}\nfn main() {\n    var p = P { x: 5, y: 1 }\n    println(p.y)\n    p.y = 9\n    println(p.y)\n    println(p.x)\n}\n";
    assert_eq!(run("region_field", source), "1\n9\n5");
}
