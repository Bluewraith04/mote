//! `?` checks the error type it returns and turns a `Failure` into an `Error`; `Error` carries a code.

use std::process::Command;

fn go(verb: &str, source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_failure_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg(verb).arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

const AUTH: &str = "import { Failure } from std.error\n\nenum AuthError: (Failure) {\n    Missing\n    Expired(String)\n\n    pub fn into_error(self) -> Error {\n        match self {\n            Missing => { return Error.new(ErrorKind.Unauthenticated, \"no token\").with_code(\"auth_missing\") }\n            Expired(who) => { return Error.new(ErrorKind.Unauthenticated, \"expired for \" + who).with_code(\"auth_expired\") }\n        }\n    }\n}\n\nfn check(n: Int) -> Result<Int, AuthError> {\n    if n == 0 { return Err(AuthError.Missing) }\n    if n == 1 { return Err(AuthError.Expired(\"bo\")) }\n    return Ok(n)\n}\n\n";

#[test]
fn a_failure_becomes_an_error_at_question_mark() {
    let src = format!("{AUTH}fn run(n: Int) -> Result<Int, Error> {{\n    let v = check(n)?\n    return Ok(v + 1)\n}}\n\nfn main() {{\n    for n in [0, 1, 5] {{\n        match run(n) {{\n            Ok(v) => {{ println(v) }}\n            Err(e) => {{ println(\"${{e.message}} ${{e.code}} ${{e.kind == ErrorKind.Unauthenticated}}\") }}\n        }}\n    }}\n}}\n");
    let (ok, text) = go("run", &src, "convert");
    assert!(ok && text == "no token auth_missing true\nexpired for bo auth_expired true\n6\n", "{text}");
}

#[test]
fn the_same_error_type_passes_through() {
    let src = format!("{AUTH}fn run(n: Int) -> Result<Int, AuthError> {{\n    let v = check(n)?\n    return Ok(v + 1)\n}}\n\nfn main() {{\n    println(run(5).is_ok())\n    println(run(0).is_err())\n}}\n");
    let (ok, text) = go("run", &src, "same");
    assert!(ok && text == "true\ntrue\n", "{text}");
}

#[test]
fn a_different_error_type_is_refused() {
    let src = "fn b() -> Result<Int, String> { return Err(\"x\") }\n\nfn run() -> Result<Int, Error> {\n    let w = b()?\n    return Ok(w)\n}\n\nfn main() { }\n";
    let (ok, text) = go("check", src, "string");
    assert!(!ok && text.contains("`?` would return `String`, but this function returns `Error`"), "{text}");
}

#[test]
fn a_type_without_failure_is_refused() {
    let src = "enum Oops {\n    Bad\n}\n\nfn a() -> Result<Int, Oops> { return Err(Oops.Bad) }\n\nfn run() -> Result<Int, Error> {\n    let v = a()?\n    return Ok(v)\n}\n\nfn main() { }\n";
    let (ok, text) = go("check", src, "nofailure");
    assert!(!ok && text.contains("`?` would return `Oops`"), "{text}");
}

#[test]
fn a_code_survives_context() {
    let src = "fn main() {\n    let e = Error.new(ErrorKind.Conflict, \"taken\").with_code(\"name_taken\").context(\"signup\")\n    println(e.message)\n    println(e.code)\n    println(e.kind == ErrorKind.Conflict)\n    println(Error.new(ErrorKind.Other, \"x\").code == \"\")\n}\n";
    let (ok, text) = go("run", src, "code");
    assert!(ok && text == "signup: taken\nname_taken\ntrue\ntrue\n", "{text}");
}
