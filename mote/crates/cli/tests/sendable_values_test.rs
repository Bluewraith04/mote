//! An enum, a tuple and a `Result` cross a task boundary when everything they can hold does.

use std::process::Command;

fn mote(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_sendable_{}_{}", tag, std::process::id()));
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
fn an_enum_is_sent_over_a_channel_to_a_task() {
    let src = "\
enum Event { Key(String) Click(Int, Int) Quit }

fn describe(e: Event) -> String {
    match e {
        Event.Key(k) => { return \"key ${k}\" }
        Event.Click(x, y) => { return \"click ${x},${y}\" }
        Event.Quit => { return \"quit\" }
    }
}

fn main() {
    let (tx, rx) = Channel<Event>(4)
    scope {
        spawn {
            tx.send(Event.Key(\"a\"))
            tx.send(Event.Click(3, 4))
            tx.send(Event.Quit)
        }
        for e in rx { println(describe(e)) }
    }
}
";
    runs(src, "enum_channel", "key a\nclick 3,4\nquit\n");
}

#[test]
fn an_enum_is_captured_by_spawn_and_held_by_a_struct() {
    let src = "\
enum Level { Low High(Int) }

struct Job {
    level: Level
    name: String
}

fn weight(l: Level) -> Int {
    match l {
        Level.High(n) => { return n }
        Level.Low => { return 1 }
    }
}

fn main() {
    let level = Level.High(7)
    let job = Job { level: Level.Low, name: \"x\" }
    let t = spawn { weight(level) }
    let u = spawn { weight(job.level) }
    println(t.join().unwrap() + u.join().unwrap())
}
";
    runs(src, "enum_capture", "8\n");
}

#[test]
fn a_recursive_and_a_generic_enum_are_sendable() {
    let src = "\
enum Tree { Leaf Node(Tree, Int, Tree) }
enum Pair<T> { One(T) Two(T, T) }

fn sum(t: Tree) -> Int {
    match t {
        Tree.Leaf => { return 0 }
        Tree.Node(l, n, r) => { return sum(l) + n + sum(r) }
    }
}

fn main() {
    let (tx, rx) = Channel<Tree>(2)
    let (px, py) = Channel<Pair<Int>>(2)
    scope {
        spawn {
            tx.send(Tree.Node(Tree.Node(Tree.Leaf, 1, Tree.Leaf), 2, Tree.Leaf))
            px.send(Pair.Two(3, 4))
        }
        println(sum(rx.recv().unwrap()))
        match py.recv().unwrap() {
            Pair.Two(a, b) => { println(a + b) }
            Pair.One(a) => { println(a) }
        }
    }
}
";
    runs(src, "enum_recursive", "3\n7\n");
}

#[test]
fn a_tuple_and_a_result_are_sent_over_channels() {
    let src = "\
fn main() {
    let (tx, rx) = Channel<(Int, String)>(2)
    let (rtx, rrx) = Channel<Result<Int, String>>(2)
    scope {
        spawn {
            tx.send((1, \"a\"))
            rtx.send(Ok(5))
            rtx.send(Err(\"bad\"))
        }
        let (n, s) = rx.recv().unwrap()
        println(\"${n}${s}\")
        println(rrx.recv().unwrap())
        println(rrx.recv().unwrap())
    }
}
";
    runs(src, "tuple_result", "1a\nOk(5)\nErr(\"bad\")\n");
}

#[test]
fn many_enums_survive_collections_between_tasks() {
    let src = "\
enum Event { Num(Int) Text(String) }

fn weight(e: Event) -> Int {
    match e {
        Event.Num(n) => { return n }
        Event.Text(s) => { return s.len() }
    }
}

fn main() {
    let (tx, rx) = Channel<Event>(64)
    scope {
        for w in 0..4 {
            let out = tx.clone()
            spawn {
                var i = 0
                while i < 5000 {
                    if i % 2 == 0 { out.send(Event.Num(1)) } else { out.send(Event.Text(\"ab${i}\")) }
                    i = i + 1
                }
            }
        }
        tx.close()
        var total = 0
        var count = 0
        for e in rx {
            total = total + weight(e)
            count = count + 1
        }
        println(count)
        println(total > 0)
    }
}
";
    runs(src, "enum_churn", "20000\ntrue\n");
}

#[test]
fn an_enum_holding_a_list_is_not_sendable() {
    let src = "\
enum Msg { Many(List<Int>) None }

fn main() {
    let (tx, rx) = Channel<Msg>(2)
    tx.send(Msg.None)
}
";
    fails(src, "enum_list", "a value sent over a channel must be Sendable");
}

#[test]
fn a_tuple_or_a_result_holding_a_class_is_not_sendable() {
    let src = "\
class Counter {
    var n: Int
}

fn main() {
    let (tx, rx) = Channel<(Int, Counter)>(2)
}
";
    fails(src, "tuple_class", "a value sent over a channel must be Sendable");
    let src = "\
class Counter {
    var n: Int
}

fn main() {
    let (tx, rx) = Channel<Result<Counter, String>>(2)
}
";
    fails(src, "result_class", "a value sent over a channel must be Sendable");
}

#[test]
fn a_generic_enum_over_an_unsendable_argument_is_not_sendable() {
    let src = "\
enum Pair<T> { One(T) Two(T, T) }

fn main() {
    let (tx, rx) = Channel<Pair<List<Int>>>(2)
}
";
    fails(src, "generic_list", "a value sent over a channel must be Sendable");
}
