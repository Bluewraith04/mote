//! `&&` and `||` evaluate the right side only when the left does not decide the result.

use std::process::Command;

fn run(name: &str, source: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_short_circuit_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

const HELPERS: &str = "fn t() -> Bool { println(\"t\"); return true }\nfn f() -> Bool { println(\"f\"); return false }\n";

#[test]
fn and_skips_the_right_side_when_the_left_is_false() {
    let source = format!("{HELPERS}println(f() && t())\nprintln(t() && f())\n");
    assert_eq!(run("and", &source), "f\nfalse\nt\nf\nfalse");
}

#[test]
fn or_skips_the_right_side_when_the_left_is_true() {
    let source = format!("{HELPERS}println(t() || f())\nprintln(f() || t())\n");
    assert_eq!(run("or", &source), "t\ntrue\nf\nt\ntrue");
}

#[test]
fn a_guard_protects_an_out_of_range_access() {
    let source = "let xs: List<Int> = []\nif xs.len() > 0 && xs.get(xs.len() - 1) != 5 {\n    println(\"hit\")\n}\nprintln(\"ok\")\n";
    assert_eq!(run("guard", source), "ok");
}
