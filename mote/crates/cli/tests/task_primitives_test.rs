//! `std.task`: `timeout`, and `all` cancelling the tasks after the first `Err`.

use std::process::Command;

fn mote(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_taskprim_{}_{}", tag, std::process::id()));
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

#[test]
fn timeout_answers_the_value_when_it_is_in_time() {
    let src = "\
import std.task as task
import std.time as time

fn main() {
    match task.timeout(time.Duration.from_millis(2000), || { return 7 }) {
        Ok(v) => { println(v) }
        Err(e) => { println(e.message) }
    }
}
";
    runs(src, "timeout_ok", "7\n");
}

#[test]
fn timeout_answers_timed_out_and_cancels_the_work() {
    let src = "\
import std.task as task
import std.time as time
import { Semaphore } from std.task

fn main() {
    let sem = Semaphore.new(1)
    let started = time.now()
    let r = task.timeout(time.Duration.from_millis(50), || {
        sem.acquire()
        time.sleep(time.Duration.from_secs(60))
        return 1
    })
    match r {
        Ok(v) => { println(\"value\") }
        Err(e) => {
            match e.kind {
                ErrorKind.TimedOut => { println(\"timed out\") }
                _ => { println(\"other\") }
            }
        }
    }
    println(time.now().duration_since(started).as_secs() < 5)
    time.sleep(time.Duration.from_millis(100))
    println(sem.available())
}
";
    runs(src, "timeout_cut", "timed out\ntrue\n1\n");
}

#[test]
fn timeout_answers_other_when_the_work_faults() {
    let src = "\
import std.task as task
import std.time as time

fn main() {
    let r = task.timeout(time.Duration.from_millis(2000), || {
        let xs: List<Int> = []
        return xs.get(3)
    })
    match r {
        Ok(v) => { println(\"value\") }
        Err(e) => {
            match e.kind {
                ErrorKind.Other => { println(\"other\") }
                _ => { println(\"kind\") }
            }
        }
    }
}
";
    runs(src, "timeout_fault", "other\n");
}

#[test]
fn all_cancels_the_tasks_after_the_first_error() {
    let src = "\
import std.task as task
import std.time as time

fn main() {
    let a = spawn { return 1 }
    let b = spawn {
        let xs: List<Int> = []
        return xs.get(2)
    }
    let c = spawn {
        time.sleep(time.Duration.from_secs(60))
        return 3
    }
    let started = time.now()
    println(task.all([a, b, c]).is_err())
    println(time.now().duration_since(started).as_secs() < 5)
    println(c.is_ready())
}
";
    runs(src, "all_cancel", "true\ntrue\ntrue\n");
}

#[test]
fn par_map_keeps_the_order_of_the_items() {
    let src = "\
import std.task as task

fn main() {
    println(task.par_map([1, 2, 3, 4, 5, 6, 7], 3, |x| { return x * 10 }).unwrap())
    let none: List<Int> = []
    println(task.par_map(none, 4, |x| { return x }).unwrap())
    println(task.par_map([5, 6], 0, |x| { return x + 1 }).unwrap())
}
";
    runs(src, "par_order", "[10, 20, 30, 40, 50, 60, 70]\n[]\n[6, 7]\n");
}

#[test]
fn par_map_runs_at_most_n_at_once() {
    let src = "\
import std.task as task
import std.time as time

class Gauge {
    inside: Shared<Int>
    most: Shared<Int>
}

fn main() {
    let g = Gauge { inside: Shared(0), most: Shared(0) }
    let r = task.par_map([1, 2, 3, 4, 5, 6, 7, 8], 2, |x| {
        g.inside.update(|n| { n += 1 })
        let now = g.inside.get()
        g.most.update(|m| { if now > m { m = now } })
        time.sleep(time.Duration.from_millis(20))
        g.inside.update(|n| { n -= 1 })
        return x
    })
    println(r.unwrap().len())
    println(g.most.get())
}
";
    runs(src, "par_limit", "8\n2\n");
}

#[test]
fn par_map_stops_the_rest_when_one_item_faults() {
    let src = "\
import std.task as task
import std.time as time

fn main() {
    let started = time.now()
    let r = task.par_map([1, 2, 3, 4], 4, |x| {
        if x == 1 {
            let xs: List<Int> = []
            return xs.get(1)
        }
        time.sleep(time.Duration.from_secs(60))
        return x
    })
    println(r.is_err())
    println(time.now().duration_since(started).as_secs() < 5)
}
";
    runs(src, "par_fault", "true\ntrue\n");
}

#[test]
fn all_still_collects_every_result_in_order() {
    let src = "\
import std.task as task

fn main() {
    let a = spawn { return 1 }
    let b = spawn { return 2 }
    println(task.all([a, b]).unwrap())
}
";
    runs(src, "all_ok", "[1, 2]\n");
}
