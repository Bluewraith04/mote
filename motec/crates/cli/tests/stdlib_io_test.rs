
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT_TEST_ID: AtomicU32 = AtomicU32::new(0);

fn run_mote_with_stdin(script: &str, stdin_data: &str) -> (String, String, bool) {
    let id = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let temp = std::env::temp_dir().join(format!("mote_io_test_{}_{}.mote", std::process::id(), id));
    std::fs::write(&temp, script).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_mote"))
        .arg("run")
        .arg(&temp)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn mote");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin_data.as_bytes())
        .unwrap();
    let output = child.wait_with_output().expect("failed to wait on mote");

    std::fs::remove_file(&temp).ok();
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.success(),
    )
}

#[test]
fn test_read_line_reads_a_real_pipe_then_eof() {
    let script = "\
import std.sys.io as io

pub fn main() {
    let sin = io.stdin()
    for _ in 0..3 {
        match sin.read_line() {
            Ok(v) => {
                match v {
                    Some(line) => { io.stdout().write_line(\"got:\" + line) }
                    None => { io.stdout().write_line(\"eof\") }
                }
            }
            Err(e) => { io.stdout().write_line(\"read failed\") }
        }
    }
}
";
    let (stdout, stderr, ok) = run_mote_with_stdin(script, "hello there\nmore\n");
    assert!(ok, "mote run failed; stderr:\n{stderr}");
    assert_eq!(stdout, "got:hello there\ngot:more\neof\n");
}

#[test]
fn test_blocking_read_line_does_not_stall_other_tasks() {
    use std::io::{BufRead, BufReader};
    use std::sync::mpsc;
    use std::time::Duration;

    let script = "\
import std.sys.io as io

fn main() -> Int {
    spawn { io.stdout().write_line(\"child ran\") }
    let r = io.stdin().read_line()
    match r {
        Ok(v) => { io.stdout().write_line(\"got a line\") }
        Err(e) => { io.stdout().write_line(\"read failed\") }
    }
    return 0
}
return main()";
    let id = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let temp = std::env::temp_dir().join(format!("mote_io_test_{}_{}.mote", std::process::id(), id));
    std::fs::write(&temp, script).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_mote"))
        .arg("run")
        .arg(&temp)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn mote");
    let mut stdin = child.stdin.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let stdout = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send(line.unwrap()).is_err() {
                break;
            }
        }
    });

    let first = rx.recv_timeout(Duration::from_secs(10));
    if first.is_err() {
        child.kill().ok();
    }
    assert_eq!(first.as_deref(), Ok("child ran"), "child task never ran while stdin was idle");

    stdin.write_all(b"hello\n").unwrap();
    drop(stdin);
    let second = rx.recv_timeout(Duration::from_secs(10)).expect("no line after stdin input");
    assert_eq!(second, "got a line");
    assert!(child.wait().unwrap().success());
    std::fs::remove_file(&temp).ok();
}
