//! `std.task.all` / `std.task.any` join-combinators over `Task<T>` handles.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_taskcomb_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn check(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_taskcomb_chk_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("check").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn rejected(source: &str, tag: &str, message: &str) {
    let (ok, text) = check(source, tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

#[test]
fn all_joins_every_task_in_list_order() {
    let src = "import std.task\nfn main() {\n    scope {\n        let a = spawn { 1 }\n        let b = spawn { 2 }\n        let c = spawn { 3 }\n        match task.all([a, b, c]) {\n            Ok(vs) => { for v in vs { print(v.to_string()); print(\" \") } }\n            Err(e) => println(e)\n        }\n    }\n    println(\"\")\n}\n";
    let (ok, text) = run(src, "all_ok");
    assert!(ok && text == "1 2 3 \n", "{text}");
}

#[test]
fn all_short_circuits_on_the_first_error() {
    let src = "import std.task\nfn main() {\n    scope {\n        let a = spawn { 1 }\n        let b = spawn { let xs: List<Int> = []; xs.get(0) }\n        match task.all([a, b]) {\n            Ok(vs) => println(\"ok\")\n            Err(e) => println(e)\n        }\n    }\n}\n";
    let (ok, text) = run(src, "all_err");
    assert!(ok && text.contains("out of bounds"), "{text}");
}

#[test]
fn any_returns_whichever_finishes_first_and_cancels_the_other() {
    let src = "import std.task\nfn slow() -> Int {\n    var i = 0\n    while i < 100000000 { i = i + 1 }\n    return i\n}\nfn main() {\n    scope {\n        let a = spawn { slow() }\n        let b = spawn { 42 }\n        match task.any([a, b]) {\n            Ok(v) => println(v.to_string())\n            Err(e) => println(e)\n        }\n    }\n    println(\"done\")\n}\n";
    let (ok, text) = run(src, "any_ok");
    assert!(ok && text == "42\ndone\n", "{text}");
}

#[test]
fn any_on_an_empty_list_is_a_runtime_error() {
    let src = "import std.task\nfn main() {\n    scope {\n        let xs: List<Task<Int>> = []\n        task.any(xs)\n    }\n}\n";
    let (ok, text) = run(src, "any_empty");
    assert!(!ok && text.contains("task.any: the list is empty"), "{text}");
}

#[test]
fn any_is_refused_over_a_plain_list_not_of_tasks() {
    rejected(
        "import std.task\nfn main() {\n    task.any([1, 2, 3])\n}\n",
        "any_wrong_type",
        "argument",
    );
}

#[test]
fn an_owner_task_answers_requests_from_a_channel() {
    let src = "struct Total { var sum: Int }\nstruct Add {\n    n: Int\n    reply: Sender<Int>\n}\n\nfn main() {\n    let (requests, inbox) = Channel<Add>(8)\n    spawn {\n        var total = Total { sum: 0 }\n        var open = true\n        while open {\n            let next = inbox.recv()\n            if next.is_some() {\n                let add = next.unwrap()\n                total.sum += add.n\n                add.reply.send(total.sum)\n            } else { open = false }\n        }\n    }\n    let (reply, answers) = Channel<Int>(1)\n    requests.send(Add { n: 4, reply: reply })\n    println(answers.recv().unwrap().to_string())\n    requests.send(Add { n: 5, reply: reply })\n    println(answers.recv().unwrap().to_string())\n    requests.close()\n}\n";
    let (ok, text) = run(src, "owner_task");
    assert!(ok && text == "4\n9\n", "{text}");
}

#[test]
fn is_ready_is_false_until_the_task_finishes_and_never_waits() {
    let src = "fn main() {\n    scope {\n        let (tx, rx) = Channel<Int>(1)\n        let t = spawn { rx.recv() }\n        println(t.is_ready().to_string())\n        tx.send(7)\n        let r = t.join()\n        println(t.is_ready().to_string())\n    }\n}\n";
    let (ok, text) = run(src, "ready_flag");
    assert!(ok && text == "false\ntrue\n", "{text}");
}

#[test]
fn is_ready_is_true_after_a_fault_and_leaves_it_unobserved() {
    let src = "fn main() {\n    scope {\n        let t = spawn { let xs: List<Int> = []; xs.get(0) }\n        while !t.is_ready() { }\n        println(t.is_ready().to_string())\n    }\n}\n";
    let (ok, text) = run(src, "ready_fault");
    assert!(!ok && text.contains("true") && text.contains("out of bounds"), "{text}");
}
