//! `std.task.Semaphore`: permits limit how many tasks run a section at once and come back when a holder ends.

use std::process::Command;

fn mote(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_sem_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn runs(source: &str, tag: &str, want: &str) {
    let (ok, text) = mote(source, tag);
    assert!(ok && text == want, "expected `{want}`, got: {text}");
}

fn fails(source: &str, tag: &str, message: &str) {
    let (ok, text) = mote(source, tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

#[test]
fn permits_limit_how_many_tasks_are_inside_at_once() {
    let src = "\
import { Semaphore } from std.task
import std.time as time

fn main() {
    let sem = Semaphore.new(3)
    var inside = Shared(0)
    var most = Shared(0)
    scope {
        for i in 0..9 {
            spawn {
                sem.acquire()
                inside.update(|n| { n += 1 })
                let now = inside.get()
                most.update(|m| { if now > m { m = now } })
                time.sleep(time.Duration.from_millis(20))
                inside.update(|n| { n -= 1 })
                sem.release()
            }
        }
    }
    println(most.get())
    println(sem.available())
}
";
    runs(src, "limit", "3\n3\n");
}

#[test]
fn with_permit_returns_the_value_and_gives_the_permit_back() {
    let src = "\
import { Semaphore } from std.task

fn main() {
    let sem = Semaphore.new(1)
    let v = sem.with_permit(|| { return 42 })
    println(v)
    println(sem.available())
}
";
    runs(src, "with_permit", "42\n1\n");
}

#[test]
fn a_task_that_faults_holding_a_permit_gives_it_back() {
    let src = "\
import { Semaphore } from std.task

fn main() {
    let sem = Semaphore.new(1)
    let t = spawn {
        sem.acquire()
        let xs: List<Int> = []
        return xs.get(3)
    }
    println(t.join().is_err())
    sem.acquire()
    println(sem.available())
}
";
    runs(src, "fault", "true\n0\n");
}

#[test]
fn a_task_cancelled_holding_a_permit_gives_it_back() {
    let src = "\
import { Semaphore } from std.task
import std.time as time

fn main() {
    let sem = Semaphore.new(1)
    let t = spawn {
        sem.acquire()
        time.sleep(time.Duration.from_secs(60))
    }
    time.sleep(time.Duration.from_millis(50))
    t.cancel()
    t.join()
    sem.acquire()
    println(sem.available())
}
";
    runs(src, "cancel_holder", "0\n");
}

#[test]
fn a_waiter_is_woken_by_a_release_and_a_cancelled_waiter_is_not_stuck() {
    let src = "\
import { Semaphore } from std.task
import std.time as time

fn main() {
    let sem = Semaphore.new(1)
    sem.acquire()
    let waiting = spawn {
        sem.acquire()
        return 1
    }
    let doomed = spawn {
        sem.acquire()
        return 2
    }
    time.sleep(time.Duration.from_millis(50))
    doomed.cancel()
    doomed.join()
    sem.release()
    println(waiting.join().unwrap())
}
";
    runs(src, "waiter", "1\n");
}

#[test]
fn permits_all_come_back_after_contention_and_cancels() {
    let src = "\
import { Semaphore } from std.task
import std.time as time

fn main() {
    let sem = Semaphore.new(2)
    var hs: List<Task<Int>> = []
    for i in 0..60 {
        hs.push(spawn {
            sem.acquire()
            time.sleep(time.Duration.from_millis(1))
            sem.release()
            return i
        })
    }
    var k = 0
    for h in hs {
        if k % 3 == 0 { h.cancel() }
        k = k + 1
    }
    for h in hs { h.join() }
    println(sem.available())
}
";
    runs(src, "contention", "2\n");
}

#[test]
fn releasing_without_a_permit_is_a_fault() {
    let src = "\
import { Semaphore } from std.task

fn main() {
    let sem = Semaphore.new(1)
    sem.release()
}
";
    fails(src, "no_permit", "holds no permit");
}
