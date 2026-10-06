//! `std.task.pin` / `unpin` / `is_pinned` from Mote programs.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_pinned_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn a_task_reports_whether_it_is_pinned() {
    let src = r#"
import std.task

fn main() {
    println(task.is_pinned())
    task.pin()
    println(task.is_pinned())
    task.unpin()
    println(task.is_pinned())
}
"#;
    let (ok, text) = run(src, "query");
    assert!(ok && text == "false\ntrue\nfalse\n", "{text}");
}

#[test]
fn a_pinned_task_serves_requests_from_the_pool_over_channels() {
    let src = r#"
import std.task

fn main() {
    let (ask, inbox) = Channel<Int>(4)
    let (tx, answers) = Channel<Int>(4)
    scope {
        spawn {
            task.pin()
            for n in inbox {
                tx.send(n * n)
            }
        }
        for i in 1..4 {
            ask.send(i)
        }
        ask.close()
        for r in answers {
            println(r)
        }
    }
}
"#;
    let (ok, text) = run(src, "channels");
    assert!(ok && text == "1\n4\n9\n", "{text}");
}

#[test]
fn pinned_tasks_still_finish_when_a_task_is_blocked_on_them() {
    let src = r#"
import std.task

fn main() {
    scope {
        let a = spawn {
            task.pin()
            40 + 2
        }
        println(a.join()!)
    }
}
"#;
    let (ok, text) = run(src, "join");
    assert!(ok && text == "42\n", "{text}");
}
