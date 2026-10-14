//! `std.signal` delivers watched OS signals as a stream, in order.
#![cfg(unix)]

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

struct Running {
    child: Child,
    dir: std::path::PathBuf,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

const PROGRAM: &str = "import std.signal as signal\n\
    fn main() {\n\
        let sigs = signal.watch([signal.Signal.Interrupt, signal.Signal.Terminate, signal.Signal.User1])\n\
        println(\"ready\")\n\
        for s in sigs {\n\
            if s == signal.Signal.Interrupt { println(\"interrupt\") }\n\
            if s == signal.Signal.User1 { println(\"user1\") }\n\
            if s == signal.Signal.Terminate {\n\
                println(\"terminate\")\n\
                break\n\
            }\n\
        }\n\
        println(\"done\")\n\
    }\n";

fn start(source: &str, tag: &str) -> (Running, BufReader<std::process::ChildStdout>) {
    let dir = std::env::temp_dir().join(format!("mote_signal_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("main.mote");
    std::fs::write(&path, source).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(&path).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().unwrap();
    let out = BufReader::new(child.stdout.take().unwrap());
    (Running { child, dir }, out)
}

fn line(out: &mut BufReader<std::process::ChildStdout>) -> String {
    let mut s = String::new();
    out.read_line(&mut s).unwrap();
    s.trim().to_string()
}

fn send(run: &Running, signal: i32) {
    // SAFETY: the pid is a live child of this test.
    unsafe { libc::kill(run.child.id() as i32, signal) };
}

#[test]
fn watched_signals_arrive_in_order_and_the_program_exits_cleanly() {
    let (mut run, mut out) = start(PROGRAM, "order");
    assert_eq!(line(&mut out), "ready");
    send(&run, libc::SIGINT);
    assert_eq!(line(&mut out), "interrupt");
    send(&run, libc::SIGUSR1);
    assert_eq!(line(&mut out), "user1");
    send(&run, libc::SIGTERM);
    assert_eq!(line(&mut out), "terminate");
    assert_eq!(line(&mut out), "done");
    assert!(run.child.wait().unwrap().success());
}

#[test]
fn an_unwatched_signal_keeps_its_default_action() {
    let (mut run, mut out) = start(PROGRAM, "default");
    assert_eq!(line(&mut out), "ready");
    send(&run, libc::SIGUSR2);
    let status = run.child.wait().unwrap();
    assert!(!status.success());
}
