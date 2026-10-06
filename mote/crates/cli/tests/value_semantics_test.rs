//! Struct value semantics through `mote run`: structural `==`, and read-only `for` and `match` bindings.

use std::process::{Command, Output};

fn mote(name: &str, source: &str) -> Output {
    let dir = std::env::temp_dir().join(format!("mote_values_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    out
}

fn run(name: &str, source: &str) -> String {
    let out = mote(name, source);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn fails(name: &str, source: &str) -> String {
    let out = mote(name, source);
    assert!(!out.status.success());
    String::from_utf8_lossy(&out.stderr).to_string()
}

const P: &str = "struct P {\n    var x: Int\n}\nstruct Line {\n    var a: P\n    var b: P\n}\n";

#[test]
fn equal_structs_compare_equal_and_different_ones_do_not() {
    let source = format!("{P}\nlet a = P {{ x: 1 }}\nprintln(a == P {{ x: 1 }})\nprintln(a == P {{ x: 2 }})\nprintln(a != P {{ x: 2 }})\n");
    assert_eq!(run("eq", &source), "true\nfalse\ntrue");
}

#[test]
fn nested_structs_compare_through_their_fields() {
    let source = format!(
        "{P}\nlet l = Line {{ a: P {{ x: 1 }}, b: P {{ x: 2 }} }}\nlet m = Line {{ a: P {{ x: 1 }}, b: P {{ x: 2 }} }}\nlet n = Line {{ a: P {{ x: 1 }}, b: P {{ x: 3 }} }}\nprintln(l == m)\nprintln(l == n)\n"
    );
    assert_eq!(run("nested", &source), "true\nfalse");
}

#[test]
fn a_for_variable_is_read_only() {
    let source = format!("{P}\nlet xs = [P {{ x: 1 }}, P {{ x: 2 }}]\nfor p in xs {{\n    p.x = 99\n}}\n");
    assert!(fails("for", &source).contains("`p` is read-only"));
}

#[test]
fn a_match_binding_is_read_only() {
    let source = format!("{P}\nlet m = P {{ x: 7 }}\nmatch m {{\n    q => {{\n        q.x = 8\n    }}\n}}\n");
    assert!(fails("match", &source).contains("`q` is read-only"));
}

#[test]
fn a_for_over_ints_still_sums() {
    assert_eq!(run("ints", "var s = 0\nfor n in [1, 2, 3] {\n    s = s + n\n}\nprintln(s)\n"), "6");
}
