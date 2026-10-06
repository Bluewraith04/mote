//! Per-module error enums (`IoError`, `ParseError`) layered over `Error` and `ErrorKind`.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_err_enums_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn io_error_of_maps_a_missing_file_to_not_found() {
    let src = "import std.sys.fs as fs\nimport { io_error_of } from std.error\npub fn main() {\n    match fs.read_text(\"/does/not/exist/at/all.txt\") {\n        Ok(_) => { println(\"unexpected ok\") }\n        Err(e) => {\n            match io_error_of(e) {\n                NotFound => { println(\"not found\") }\n                _ => { println(\"other\") }\n            }\n        }\n    }\n}\n";
    let (ok, text) = run(src, "io_not_found");
    assert!(ok && text == "not found\n", "{text}");
}

#[test]
fn parse_error_of_maps_a_bad_int_to_invalid_syntax() {
    let src = "import std.string as str\nimport { parse_error_of } from std.error\npub fn main() {\n    match str.parse_int(\"not a number\") {\n        Ok(_) => { println(\"unexpected ok\") }\n        Err(e) => {\n            match parse_error_of(e) {\n                InvalidSyntax => { println(\"invalid syntax\") }\n                Other => { println(\"other\") }\n            }\n        }\n    }\n}\n";
    let (ok, text) = run(src, "parse_invalid");
    assert!(ok && text == "invalid syntax\n", "{text}");
}

#[test]
fn existing_error_typed_callers_are_unaffected() {
    let src = "import std.sys.fs as fs\npub fn main() {\n    match fs.read_text(\"/does/not/exist/at/all.txt\") {\n        Ok(_) => { println(\"unexpected ok\") }\n        Err(e) => { println(e.message) }\n    }\n}\n";
    let (ok, text) = run(src, "unaffected");
    assert!(ok && text.starts_with("open:"), "{text}");
}
