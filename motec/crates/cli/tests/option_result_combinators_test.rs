//! Option/Result combinators — map, and_then, unwrap_or_else, ok_or, or, or_else.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_optres_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn check(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_optres_chk_{}_{}", tag, std::process::id()));
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
fn option_map_transforms_some_and_skips_none() {
    let src = "fn double(x: Int) -> Int { return x * 2 }\npub fn main() {\n    let a: Option<Int> = Some(5)\n    let b: Option<Int> = None\n    println(a.map(double).unwrap())\n    println(b.map(double).is_none())\n}\n";
    let (ok, text) = run(src, "opt_map");
    assert!(ok && text == "10\ntrue\n", "{text}");
}

#[test]
fn option_and_then_chains_and_short_circuits() {
    let src = "fn half(x: Int) -> Option<Int> {\n    if x % 2 == 0 { return Some(x / 2) }\n    return None\n}\npub fn main() {\n    println(Some(4).and_then(half).unwrap())\n    println(Some(5).and_then(half).is_none())\n}\n";
    let (ok, text) = run(src, "opt_and_then");
    assert!(ok && text == "2\ntrue\n", "{text}");
}

#[test]
fn unwrap_or_else_calls_its_fallback_only_on_failure() {
    let src = "fn zero() -> Int { return 0 }\npub fn main() {\n    let a: Option<Int> = Some(9)\n    let b: Option<Int> = None\n    println(a.unwrap_or_else(zero).to_string())\n    println(b.unwrap_or_else(zero).to_string())\n}\n";
    let (ok, text) = run(src, "unwrap_or_else");
    assert!(ok && text == "9\n0\n", "{text}");
}

#[test]
fn ok_or_converts_option_to_result() {
    let src = "pub fn main() {\n    let a: Option<Int> = Some(3)\n    let b: Option<Int> = None\n    println(a.ok_or(\"missing\").unwrap().to_string())\n    println(b.ok_or(\"missing\").is_err())\n}\n";
    let (ok, text) = run(src, "ok_or");
    assert!(ok && text == "3\ntrue\n", "{text}");
}

#[test]
fn result_map_transforms_ok_and_skips_err() {
    let src = "fn double(x: Int) -> Int { return x * 2 }\npub fn main() {\n    let a: Result<Int, String> = Ok(5)\n    let b: Result<Int, String> = Err(\"bad\")\n    println(a.map(double).unwrap())\n    println(b.map(double).is_err())\n}\n";
    let (ok, text) = run(src, "res_map");
    assert!(ok && text == "10\ntrue\n", "{text}");
}

#[test]
fn result_or_and_or_else_fall_back_only_on_err() {
    let src = "pub fn main() {\n    let a: Result<Int, String> = Ok(5)\n    let b: Result<Int, String> = Err(\"bad\")\n    println(a.or(Ok(99)).unwrap().to_string())\n    println(b.or(Ok(99)).unwrap().to_string())\n    println(b.or_else(|| { return Ok(7) }).unwrap().to_string())\n    println(a.or_else(|| { return Ok(7) }).unwrap().to_string())\n}\n";
    let (ok, text) = run(src, "res_or");
    assert!(ok && text == "5\n99\n7\n5\n", "{text}");
}

#[test]
fn map_on_a_non_option_type_is_a_checker_error() {
    rejected(
        "pub fn main() {\n    let x = 5\n    x.map(5)\n}\n",
        "map_non_option",
        "no method",
    );
}
