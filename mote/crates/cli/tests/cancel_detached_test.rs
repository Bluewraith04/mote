//! A detached task that is cancelled while it waits ends quietly: it is not a fault.

use std::process::Command;

#[test]
fn a_cancelled_detached_task_is_not_reported() {
    let src = "\
import std.time as time

fn main() {
    let h = spawn {
        var i = 0
        while i < 100000 {
            time.sleep(time.Duration.from_millis(1))
            i += 1
        }
    }
    time.sleep(time.Duration.from_millis(20))
    h.cancel()
    time.sleep(time.Duration.from_millis(20))
    println(h.is_ready())
}
";
    let dir = std::env::temp_dir().join(format!("mote_cancel_detached_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success() && stdout == "true\n" && stderr.is_empty(), "got: {stdout}{stderr}");
}
