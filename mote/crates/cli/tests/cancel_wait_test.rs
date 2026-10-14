//! Cancelling a task parked in a reactor wait (sleep, accept, socket read) ends the wait at once.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_cancel_wait_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn passes(source: &str, tag: &str, want: &str) {
    let (ok, text) = run(source, tag);
    assert!(ok && text == want, "{tag}: expected {want:?}, got:\n{text}");
}

#[test]
fn a_sleeping_task_stops_when_cancelled() {
    let src = r#"
import std.time as time
fn main() {
    let started = time.unix_millis()
    scope {
        let t = spawn { time.sleep(time.Duration.from_secs(30)) }
        time.sleep(time.Duration.from_millis(100))
        t.cancel()
        println(t.join().is_err().to_string())
    }
    println((time.unix_millis() - started < 5000).to_string())
}
"#;
    passes(src, "sleep", "true\ntrue\n");
}

#[test]
fn a_task_waiting_in_accept_stops_when_cancelled() {
    let src = r#"
import std.time as time
import std.socket as net
fn main() {
    let started = time.unix_millis()
    scope {
        let t = spawn {
            let l = net.listen("127.0.0.1", 0).unwrap()
            l.accept()
        }
        time.sleep(time.Duration.from_millis(200))
        t.cancel()
        println(t.join().is_err().to_string())
    }
    println((time.unix_millis() - started < 5000).to_string())
}
"#;
    passes(src, "accept", "true\ntrue\n");
}

#[test]
fn a_task_waiting_in_a_socket_read_stops_when_cancelled() {
    let src = r#"
import std.time as time
import std.socket as net
fn main() {
    let started = time.unix_millis()
    scope {
        let t = spawn {
            let l = net.listen("127.0.0.1", 0).unwrap()
            let port = l.local_port().unwrap()
            let client = net.connect("127.0.0.1", port).unwrap()
            let conn = l.accept().unwrap()
            conn.read(16)
        }
        time.sleep(time.Duration.from_millis(300))
        t.cancel()
        println(t.join().is_err().to_string())
    }
    println((time.unix_millis() - started < 5000).to_string())
}
"#;
    passes(src, "read", "true\ntrue\n");
}

#[test]
fn any_does_not_wait_for_a_sleeping_loser() {
    let src = r#"
import std.task
import std.time as time
fn main() {
    let started = time.unix_millis()
    scope {
        let slow = spawn { time.sleep(time.Duration.from_secs(30)) }
        let fast = spawn { 42 }
        time.sleep(time.Duration.from_millis(100))
        match task.any([slow, fast]) {
            Ok(v) => println(v.to_string())
            Err(e) => println(e)
        }
    }
    println((time.unix_millis() - started < 5000).to_string())
}
"#;
    passes(src, "any", "42\ntrue\n");
}

#[test]
fn a_cancelled_parent_cancels_the_sleeping_child_in_its_scope() {
    let src = r#"
import std.time as time
fn main() {
    let started = time.unix_millis()
    scope {
        let outer = spawn {
            scope {
                let child = spawn { time.sleep(time.Duration.from_secs(30)) }
                child.join()
            }
        }
        time.sleep(time.Duration.from_millis(200))
        outer.cancel()
        println(outer.join().is_err().to_string())
    }
    println((time.unix_millis() - started < 5000).to_string())
}
"#;
    passes(src, "parent", "true\ntrue\n");
}

#[test]
fn a_faulting_task_cancels_its_sleeping_sibling_at_the_scope_end() {
    let src = r#"
import std.time as time
fn run() -> Result<Int, Error> {
    scope {
        spawn { time.sleep(time.Duration.from_secs(30)) }
        spawn {
            time.sleep(time.Duration.from_millis(100))
            let xs: List<Int> = []
            xs.get(0)
        }
    }
    return Ok(1)
}
fn main() {
    let started = time.unix_millis()
    println(run().is_err().to_string())
    println((time.unix_millis() - started < 5000).to_string())
}
"#;
    passes(src, "sibling", "true\ntrue\n");
}

#[test]
fn cancelling_a_finished_task_twice_is_harmless() {
    let src = r#"
fn main() {
    scope {
        let t = spawn { 1 }
        println(t.join().unwrap().to_string())
        t.cancel()
        t.cancel()
    }
    println("done")
}
"#;
    passes(src, "finished", "1\ndone\n");
}

#[test]
fn a_cancelled_sleep_never_wakes_the_task_that_reuses_its_slot() {
    let src = r#"
import std.time as time
fn main() {
    scope {
        let sleeper = spawn { time.sleep(time.Duration.from_millis(300)) }
        time.sleep(time.Duration.from_millis(50))
        sleeper.cancel()
        sleeper.join()
        let (tx, rx) = Channel<Int>(1)
        let waiter = spawn { rx.recv() }
        time.sleep(time.Duration.from_millis(600))
        println(waiter.is_ready().to_string())
        tx.send(7)
        println(waiter.join().unwrap().unwrap().to_string())
    }
}
"#;
    passes(src, "reuse", "false\n7\n");
}

#[test]
fn a_cancel_landing_as_a_task_starts_to_sleep_is_never_missed() {
    let src = r#"
import std.time as time
fn round(i: Int) -> Int {
    var out = 0
    scope {
        let t = spawn { time.sleep(time.Duration.from_secs(30)) }
        var k = 0
        while k < i % 200 { k = k + 1 }
        t.cancel()
        let r = t.join()
        out = 1
    }
    return out
}
fn main() {
    var total = 0
    var i = 0
    while i < 1500 {
        total = total + round(i)
        i = i + 1
    }
    println(total.to_string())
}
"#;
    passes(src, "start", "1500\n");
}

#[test]
fn cancels_racing_a_sleep_that_is_ending_never_hang_or_lose_a_task() {
    let src = r#"
import std.time as time
fn round(i: Int) -> Int {
    var out = 0
    scope {
        let t = spawn { time.sleep(time.Duration.from_millis(i % 3)) }
        time.sleep(time.Duration.from_millis(i % 2))
        t.cancel()
        let r = t.join()
        out = 1
    }
    return out
}
fn main() {
    var total = 0
    var i = 0
    while i < 300 {
        total = total + round(i)
        i = i + 1
    }
    println(total.to_string())
}
"#;
    passes(src, "race", "300\n");
}

#[test]
fn a_task_waiting_for_a_datagram_stops_when_cancelled() {
    let src = r#"
import std.time as time
import std.socket as net
fn main() {
    let started = time.unix_millis()
    scope {
        let t = spawn {
            let u = net.bind("127.0.0.1", 0).unwrap()
            u.recv_from(16)
        }
        time.sleep(time.Duration.from_millis(200))
        t.cancel()
        println(t.join().is_err().to_string())
    }
    println((time.unix_millis() - started < 5000).to_string())
}
"#;
    passes(src, "udp", "true\ntrue\n");
}

#[test]
fn a_task_waiting_on_a_ticker_stops_when_cancelled() {
    let src = r#"
import std.time as time
fn main() {
    let started = time.unix_millis()
    scope {
        let t = spawn {
            let ticks = time.ticker(time.Duration.from_secs(30))
            ticks.recv()
        }
        time.sleep(time.Duration.from_millis(200))
        t.cancel()
        println(t.join().is_err().to_string())
    }
    println((time.unix_millis() - started < 5000).to_string())
}
"#;
    passes(src, "ticker", "true\ntrue\n");
}

#[cfg(unix)]
#[test]
fn a_cancelled_task_in_a_process_run_is_released_when_the_process_ends() {
    let src = r#"
import std.time as time
import std.process as process
fn main() {
    let started = time.unix_millis()
    scope {
        let t = spawn { process.run("sleep", ["1"]) }
        time.sleep(time.Duration.from_millis(200))
        t.cancel()
        let r = t.join()
        println((time.unix_millis() - started < 4000).to_string())
    }
}
"#;
    passes(src, "process", "true\n");
}
