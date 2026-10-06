//! `std.sys.process` through `mote run`, running real children.
#![cfg(unix)]

use std::process::Command;

fn run(name: &str, body: &str) -> (String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("mote_process_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = format!("import std.sys.process as process\nimport {{ Output }} from std.sys.process\n\n{body}");
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (String::from_utf8_lossy(&out.stdout).trim().to_string(), out.status.code())
}

#[test]
fn a_child_runs_with_captured_output_and_a_failing_status_is_ok() {
    let body = r#"
fn main() {
    let o: Output = process.run("sh", ["-c", "echo out; echo err >&2; exit 4"]).unwrap()
    println(o.status)
    println(o.success())
    println(o.stdout_text().unwrap().trim())
    println(o.stderr_text().unwrap().trim())
}
"#;
    assert_eq!(run("capture", body), ("4\nfalse\nout\nerr".to_string(), Some(0)));
}

#[test]
fn stdin_env_and_cwd_reach_the_child() {
    let body = r#"
fn main() {
    let input = Bytes()
    input.extend("hello".bytes())
    let o: Output = process.run_with("sh", ["-c", "cat; printf ':%s:%s' \"$MOTE_V\" \"$PWD\""], ["MOTE_V=7"], input, "/").unwrap()
    println(o.stdout_text().unwrap())
}
"#;
    assert_eq!(run("inputs", body), ("hello:7:/".to_string(), Some(0)));
}

#[test]
fn a_missing_program_is_an_error_and_exit_sets_the_status() {
    let body = r#"
fn main() {
    println(process.run("mote-no-such-program-x", []).is_err())
    process.exit(3)
}
"#;
    assert_eq!(run("exit", body), ("true".to_string(), Some(3)));
}
