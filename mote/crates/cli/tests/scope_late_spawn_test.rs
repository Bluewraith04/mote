//! A task spawned into a scope that already has a fault starts cancelled.

use std::process::Command;

fn mote(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_late_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn cancelling_a_task_does_not_fault_its_scope() {
    let src = "\
import std.time as time

fn main() {
    scope {
        let a = spawn {
            time.sleep(time.Duration.from_secs(60))
            return 1
        }
        let b = spawn {
            time.sleep(time.Duration.from_millis(100))
            return 2
        }
        time.sleep(time.Duration.from_millis(20))
        a.cancel()
        println(b.join().unwrap())
        let late = spawn { return 3 }
        println(late.join().unwrap())
    }
    println(\"after scope\")
}
";
    let (ok, text) = mote(src, "cancel_ok");
    assert!(ok && text == "2\n3\nafter scope\n", "got: {text}");
}

#[test]
fn a_task_spawned_after_a_sibling_faulted_is_cancelled() {
    let src = "\
import std.time as time

fn main() {
    let started = time.now()
    scope {
        let a = spawn {
            let xs: List<Int> = []
            return xs.get(1)
        }
        println(a.join().is_err())
        let late = spawn {
            time.sleep(time.Duration.from_secs(60))
            return 1
        }
        println(late.join().is_err())
    }
    println(time.now().duration_since(started).as_secs() < 5)
}
";
    let (ok, text) = mote(src, "late");
    assert!(ok && text == "true\ntrue\ntrue\n", "got: {text}");
}
