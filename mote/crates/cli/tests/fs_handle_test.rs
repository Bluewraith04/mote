//! `std.sys.fs` file handles through `mote run` on a real file.

use std::process::Command;

fn run(name: &str, body: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_fs_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let data = dir.join("data.txt");
    let source = format!("import std.sys.fs\nimport {{ File }} from std.sys.fs\n\nlet path = \"{}\"\n{body}", data.display());
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn write_append_seek_and_read_lines_round_trip() {
    let body = r#"
let w: File = fs.create(path).unwrap()
w.write_text("one\ntwo\n").unwrap()
w.close().unwrap()
let a: File = fs.append(path).unwrap()
a.write_text("three\n").unwrap()
a.close().unwrap()
let r: File = fs.open(path).unwrap()
println(r.read_line().unwrap().unwrap())
println(r.seek_end(-6).unwrap())
println(r.read_line().unwrap().unwrap())
println(r.read_line().unwrap().is_none())
println(r.seek(0).unwrap())
println(r.read(3).unwrap().len())
r.close().unwrap()
"#;
    assert_eq!(run("round_trip", body), "one\n8\nthree\ntrue\n0\n3");
}

#[test]
fn a_closed_handle_and_a_missing_file_are_errors() {
    let body = r#"
let w: File = fs.create(path).unwrap()
w.close().unwrap()
println(w.close().is_err())
println(fs.open(path + ".absent").is_err())
"#;
    assert_eq!(run("errors", body), "true\ntrue");
}

#[test]
fn lines_streams_the_rest_of_the_file_and_a_failure_is_one_err() {
    let body = r#"
let w: File = fs.create(path).unwrap()
w.write_text("a\nb\nc\n").unwrap()
w.close().unwrap()
let r: File = fs.open(path).unwrap()
println(r.read_line().unwrap().unwrap())
for item in r.lines() {
    match item {
        Ok(line) => { println(line) }
        _ => { println("err") }
    }
}
r.close().unwrap()
var errors = 0
for item in r.lines() {
    match item {
        Ok(line) => { println("line") }
        _ => { errors = errors + 1 }
    }
}
println(errors)
"#;
    assert_eq!(run("lines", body), "a\nb\nc\n1");
}
