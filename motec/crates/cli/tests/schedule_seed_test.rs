//! `--schedule-seed` varies where tasks switch, reproducibly, and a failing seeded run names its seed.

use std::process::Command;

const RACE: &str = "\
fn main() {
    var log = Shared([0])
    scope {
        spawn {
            var i = 0
            while i < 400 {
                log.update(|xs| { xs.push(1) })
                i = i + 1
            }
        }
        spawn {
            var i = 0
            while i < 400 {
                log.update(|xs| { xs.push(2) })
                i = i + 1
            }
        }
    }
    let xs = log.get()
    var first_switch = 1
    while xs[first_switch] == xs[1] { first_switch = first_switch + 1 }
    println(first_switch)
}
";

fn mote(verb: &str, source: &str, tag: &str, extra: &[&str]) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_seed_{}_{}_{}", verb, tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.mote");
    std::fs::write(&file, source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg(verb).arg(&file).args(extra).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn switch_point(seed: u32, tag: &str) -> String {
    let (ok, text) = mote("run", RACE, tag, &["--schedule-seed", &seed.to_string()]);
    assert!(ok, "{text}");
    text
}

const RECEIVERS: &str = "\
fn main() {
    let (tx, rx) = Channel<Int>(4)
    scope {
        let ra = rx.clone()
        let rb = rx.clone()
        let rc = rx.clone()
        let a = spawn { return ra.recv().unwrap() }
        let b = spawn { return rb.recv().unwrap() }
        let c = spawn { return rc.recv().unwrap() }
        spawn {
            tx.send(1)
            tx.send(2)
            tx.send(3)
        }
        println(a.join().unwrap() * 100 + b.join().unwrap() * 10 + c.join().unwrap())
    }
}
";

const SENDERS: &str = "\
fn main() {
    let (tx, rx) = Channel<Int>(1)
    tx.send(0)
    scope {
        let t1 = tx.clone()
        let t2 = tx.clone()
        let t3 = tx.clone()
        spawn { t1.send(1) }
        spawn { t2.send(2) }
        spawn { t3.send(3) }
        let reader = spawn {
            var seen = 0
            var i = 0
            while i < 4 {
                seen = seen * 10 + rx.recv().unwrap()
                i = i + 1
            }
            return seen
        }
        println(reader.join().unwrap())
    }
}
";

#[test]
fn tasks_parked_on_a_receive_wake_in_the_order_they_parked() {
    let (ok, text) = mote("run", RECEIVERS, "recv_order", &["--workers", "1"]);
    assert!(ok && text == "123\n", "{text}");
}

#[test]
fn tasks_parked_on_a_full_channel_wake_in_the_order_they_parked() {
    let (ok, text) = mote("run", SENDERS, "send_order", &["--workers", "1"]);
    assert!(ok && text == "123\n", "{text}");
}

#[test]
fn a_seed_replays_the_same_switches() {
    for seed in [1, 2, 3] {
        assert_eq!(switch_point(seed, "a"), switch_point(seed, "b"), "seed {seed}");
    }
}

#[test]
fn different_seeds_switch_in_different_places() {
    let points: std::collections::HashSet<String> = (1..=8).map(|s| switch_point(s, "many")).collect();
    assert!(points.len() > 1, "every seed switched at {points:?}");
}

#[test]
fn a_failing_seeded_test_names_its_seed() {
    let src = "test \"broken\" {\n    assert_eq(1 + 1, 3)\n}\n";
    let (ok, text) = mote("test", src, "fail", &["--schedule-seed", "42"]);
    assert!(!ok && text.contains("schedule seed: 42"), "{text}");
}

#[test]
fn a_passing_seeded_test_prints_no_seed() {
    let src = "test \"fine\" {\n    assert_eq(2 + 2, 4)\n}\n";
    let (ok, text) = mote("test", src, "pass", &["--schedule-seed", "42"]);
    assert!(ok && !text.contains("schedule seed"), "{text}");
}

#[test]
fn a_seed_needs_a_number() {
    let (ok, text) = mote("test", "test \"t\" {\n}\n", "bad", &["--schedule-seed", "many"]);
    assert!(!ok && text.contains("--schedule-seed requires a number"), "{text}");
}
