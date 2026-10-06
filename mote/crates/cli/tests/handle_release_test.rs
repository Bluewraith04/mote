//! A file no object holds any more is closed by the collector, so dropped handles stay under the OS limit.

#![cfg(unix)]

use std::process::Command;

fn run(body: &str, tag: &str, extra: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_handles_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.mote");
    let source = format!("import {{ open }} from std.sys.fs\n\nfn main() {{\n{body}\n}}\n").replace("SELF", &file.to_string_lossy());
    std::fs::write(&file, source).unwrap();
    let script = format!("ulimit -n 256; exec {} run {} {extra}", env!("CARGO_BIN_EXE_mote"), file.display());
    let out = Command::new("sh").arg("-c").arg(script).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

const DROP_LOOP: &str = "    var n = 0\n    while n < 1500 {\n        match open(\"SELF\") {\n            Ok(_) => { n += 1 }\n            Err(e) => {\n                println(\"failed at ${n}\")\n                return\n            }\n        }\n    }\n    println(\"opened ${n}\")";

#[test]
fn dropped_files_are_closed_by_the_collector() {
    let (ok, text) = run(DROP_LOOP, "drop", "");
    assert!(ok, "{text}");
    assert!(text.contains("opened 1500"), "{text}");
}

#[test]
fn without_a_collector_the_same_program_runs_out_of_descriptors() {
    let (_, text) = run(DROP_LOOP, "nogc", "--gc nogc");
    assert!(text.contains("failed at"), "{text}");
}

#[test]
fn a_file_still_held_stays_open() {
    let body = "    let keep = open(\"SELF\")?\n    var n = 0\n    while n < 1000 {\n        let f = open(\"SELF\")?\n        n += 1\n    }\n    match keep.read_line() {\n        Ok(line) => { println(\"still open: ${line.is_some()}\") }\n        Err(e) => { println(\"closed: ${e.message}\") }\n    }";
    let (ok, text) = run(body, "held", "");
    assert!(ok, "{text}");
    assert!(text.contains("still open: true"), "{text}");
}

#[test]
fn dropped_sockets_are_closed_by_the_collector() {
    let body = "    var n = 0\n    while n < 600 {\n        match listen(\"127.0.0.1\", 0) {\n            Ok(_) => { n += 1 }\n            Err(e) => {\n                println(\"failed at ${n}\")\n                return\n            }\n        }\n    }\n    println(\"listened ${n}\")";
    let source = body.to_string();
    let dir = std::env::temp_dir().join(format!("mote_handles_sockets_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.mote");
    std::fs::write(&file, format!("import {{ listen }} from std.sys.net\n\nfn main() {{\n{source}\n}}\n")).unwrap();
    let script = format!("ulimit -n 256; exec {} run {}", env!("CARGO_BIN_EXE_mote"), file.display());
    let out = Command::new("sh").arg("-c").arg(script).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    assert!(text.contains("listened 600"), "{text}");
}

#[test]
fn a_file_closed_by_its_program_is_not_closed_twice_by_the_collector() {
    let body = "    var n = 0\n    while n < 1000 {\n        let f = open(\"SELF\")?\n        f.close()?\n        n += 1\n    }\n    println(\"done ${n}\")";
    let (ok, text) = run(body, "closed", "");
    assert!(ok, "{text}");
    assert!(text.contains("done 1000"), "{text}");
}
