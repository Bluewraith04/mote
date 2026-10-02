//! A fallible native's call answers a `Result`: `Ok` with the value, `Err` with the kind and message.

use std::process::Command;

fn run(source: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_fallible_{}_{}", std::process::id(), source.len()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.mote");
    std::fs::write(&file, source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(&file).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_success_is_ok_with_the_value() {
    let out = run("import std.string as str\n\nfn main() {\n    println(str.parse_int(\"-42\").unwrap())\n    println(str.parse_float(\"2.5\").unwrap())\n}\n");
    assert_eq!(out, "-42\n2.5\n");
}

#[test]
fn a_failure_is_err_with_its_kind_and_message() {
    let source = "import std.string as str\n\nfn main() {\n    match str.parse_int(\"bad\") {\n        Ok(n) => println(n)\n        Err(e) => {\n            println(e.message)\n            println(e.kind == ErrorKind.InvalidData)\n        }\n    }\n}\n";
    assert_eq!(run(source), "not an integer: bad\ntrue\n");
}

#[test]
fn a_failure_propagates_through_the_question_mark() {
    let source = "import std.string as str\n\nfn sum(a: String, b: String) -> Result<Int, Error> {\n    let x = str.parse_int(a)?\n    let y = str.parse_int(b)?\n    return Ok(x + y)\n}\n\nfn main() {\n    println(sum(\"1\", \"2\").unwrap())\n    println(sum(\"1\", \"two\").is_err())\n}\n";
    assert_eq!(run(source), "3\ntrue\n");
}
