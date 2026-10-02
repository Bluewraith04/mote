//! `Channel<T>(n)` makes a `Sender` and a `Receiver`; senders move into `spawn`, and the channel closes with its last one.

use std::process::Command;

fn mote(verb: &str, source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_ends_{}_{}_{}", verb, tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg(verb).arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn runs(source: &str, tag: &str, want: &str) {
    let (ok, text) = mote("run", source, tag);
    assert!(ok && text == want, "expected `{want}`, got: {text}");
}

fn fails(verb: &str, source: &str, tag: &str, message: &str) {
    let (ok, text) = mote(verb, source, tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

#[test]
fn a_channel_closes_when_its_owner_task_ends() {
    let src = "\
fn main() {
    let (tx, rx) = Channel<Int>(4)
    scope {
        spawn {
            tx.send(1)
            tx.send(2)
            tx.send(3)
        }
        var sum = 0
        for x in rx { sum = sum + x }
        println(sum)
    }
}
";
    runs(src, "owner_ends", "6\n");
}

#[test]
fn a_channel_stays_open_until_every_clone_is_gone() {
    let src = "\
fn main() {
    let (tx, rx) = Channel<Int>(2)
    scope {
        let a = tx.clone()
        let b = tx.clone()
        spawn { a.send(10) }
        spawn { b.send(20) }
        tx.close()
        var sum = 0
        for x in rx { sum = sum + x }
        println(sum)
    }
}
";
    runs(src, "clones", "30\n");
}

#[test]
fn a_drained_closed_channel_gives_none() {
    let src = "\
fn main() {
    let (tx, rx) = Channel<Int>(2)
    tx.send(5)
    tx.close()
    println(rx.recv().unwrap())
    println(rx.recv().is_none())
}
";
    runs(src, "drained", "5\ntrue\n");
}

#[test]
fn sending_on_a_closed_sender_is_a_fault() {
    let src = "fn main() {\n    let (tx, rx) = Channel<Int>(1)\n    tx.close()\n    tx.send(1)\n}\n";
    fails("run", src, "send_closed", "closed");
}

#[test]
fn a_clone_still_sends_after_the_original_closes() {
    let src = "\
fn main() {
    let (tx, rx) = Channel<Int>(2)
    let other = tx.clone()
    tx.close()
    other.send(8)
    println(rx.recv().unwrap())
}
";
    runs(src, "clone_after_close", "8\n");
}

#[test]
fn a_capacity_of_zero_hands_each_value_over() {
    let src = "\
fn main() {
    let (tx, rx) = Channel<Int>(0)
    scope {
        spawn {
            tx.send(1)
            tx.send(2)
            tx.send(3)
        }
        var sum = 0
        for x in rx { sum = sum + x }
        println(sum)
    }
}
";
    runs(src, "rendezvous", "6\n");
}

#[test]
fn a_capacity_of_zero_send_waits_for_a_receiver() {
    let src = "fn main() {\n    let (tx, rx) = Channel<Int>(0)\n    tx.send(1)\n}\n";
    fails("run", src, "rendezvous_stuck", "deadlock");
}

#[test]
fn an_end_moves_into_a_spawn() {
    let src = "fn main() {\n    let (tx, rx) = Channel<Int>(1)\n    scope {\n        spawn { tx.send(1) }\n        tx.send(2)\n    }\n}\n";
    fails("check", src, "moved", "cannot use `tx` — it moved into a `spawn`; clone it first");
}

#[test]
fn a_clone_taken_before_the_spawn_stays_usable() {
    let src = "\
fn main() {
    let (tx, rx) = Channel<Int>(2)
    let mine = tx.clone()
    scope {
        spawn { tx.send(1) }
    }
    mine.send(2)
    mine.close()
    var sum = 0
    for x in rx { sum = sum + x }
    println(sum)
}
";
    runs(src, "clone_before", "3\n");
}

#[test]
fn an_end_cannot_move_into_a_spawn_on_every_pass_of_a_loop() {
    let src = "fn main() {\n    let (tx, rx) = Channel<Int>(1)\n    var i = 0\n    while i < 2 {\n        spawn { tx.send(i) }\n        i = i + 1\n    }\n}\n";
    fails("check", src, "loop_move", "moves into `spawn` on every pass of the loop; clone it inside the loop");
}

#[test]
fn a_clone_made_inside_the_loop_may_move() {
    let src = "\
fn main() {
    let (tx, rx) = Channel<Int>(4)
    scope {
        var i = 1
        while i <= 3 {
            let mine = tx.clone()
            let n = i
            spawn { mine.send(n) }
            i = i + 1
        }
        tx.close()
        var sum = 0
        for x in rx { sum = sum + x }
        println(sum)
    }
}
";
    runs(src, "loop_clone", "6\n");
}

#[test]
fn a_receiver_cannot_close() {
    let src = "fn main() {\n    let (tx, rx) = Channel<Int>(1)\n    rx.close()\n}\n";
    fails("check", src, "rx_close", "a `Receiver` can't close a channel; close its `Sender`s");
}

#[test]
fn a_channel_of_a_mutable_class_is_rejected() {
    let src = "class Box { var n: Int }\nfn main() {\n    let (tx, rx) = Channel<Box>(1)\n}\n";
    fails("check", src, "unsendable", "a value sent over a channel must be Sendable");
}

#[test]
fn a_sender_and_a_receiver_are_different_types() {
    let src = "fn main() {\n    let (tx, rx) = Channel<Int>(1)\n    let r: Receiver<Int> = tx\n}\n";
    fails("check", src, "mixed", "Sender<Int>");
}

#[test]
fn channel_is_not_a_type() {
    let src = "fn main() {\n    let c: Channel<Int> = Channel(1)\n}\n";
    fails("check", src, "not_a_type", "`Channel` is not a type; write `Sender<T>` or `Receiver<T>`");
}
