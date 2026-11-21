use mote_base::{Interner, Severity};

use super::analyze;
use crate::check::check;
use crate::defs::{DefId, Resolved};
use crate::instantiate::instantiate;
use crate::program::{Init, Instance, Program};
use crate::resolve::resolve;
use crate::testing::{parse_sources, path_text};

const SYNC: &str = "@lang(\"list\")\npub class List<T> {}\n@lang(\"stream\")\npub class Stream<T> {}\n@lang(\"task\")\npub class Task<T> {\n pub fn await(self) {}\n}\n@lang(\"sender\")\npub class Sender<T> {\n pub fn send(self, v: T) {}\n}\n@lang(\"receiver\")\npub class Receiver<T> {\n pub fn recv(self) {}\n}\n@lang(\"mutex\")\npub class Mutex<T> {\n pub fn lock(self, work: Send (var T) -> Null) {}\n pub fn get(self) {}\n pub fn set(self, v: T) {}\n}\n@link(\"lib\")\n@blocking\npub native fn sleep()";

struct Built {
    program: Program,
    interner: Interner,
    errors: Vec<String>,
}

/// Checks the sources, instantiates every plain function of `main`, then analyzes.
fn build(sources: &[(&str, &str)]) -> Built {
    let mut parsed = parse_sources(sources);
    let mut program = resolve(parsed.input, &mut parsed.interner, &mut parsed.sink);
    check(&mut program, &mut parsed.interner, &mut parsed.sink);
    let module = program.modules.iter().find(|(_, module)| module.path.len() == 1 && parsed.interner.resolve(module.path[0]) == "main").map(|(id, _)| id).expect("a main module");
    let mut entries: Vec<DefId> = program.modules[module]
        .scope
        .values()
        .filter_map(|resolved| match resolved {
            Resolved::Def(def) => Some(*def),
            Resolved::Module(_) => None,
        })
        .filter(|&def| program.defs[def].module == module && program.defs[def].owner.is_none())
        .filter(|&def| program.defs[def].function().is_some_and(|function| function.has_body && function.generics.is_empty()))
        .collect();
    entries.sort_by_key(|&def| program.defs[def].span.start);
    let entries: Vec<Instance> = entries.into_iter().map(|def| Instance { def, args: Vec::new() }).collect();
    instantiate(&mut program, &parsed.interner, &mut parsed.sink, &entries);
    analyze(&mut program, &parsed.interner, &mut parsed.sink);
    let errors = parsed.sink.iter().filter(|diagnostic| diagnostic.severity == Severity::Error).map(|diagnostic| diagnostic.message.clone()).collect();
    Built { program, interner: parsed.interner, errors }
}

fn main(text: &str) -> Built {
    build(&[("main", text)])
}

/// `main` with std's sync types imported.
fn with_sync(text: &str) -> Built {
    build(&[("std.sync", SYNC), ("main", &format!("import {{ List, Stream, Task, Sender, Receiver, Mutex, sleep }} from .std.sync\n{text}"))])
}

impl Built {
    /// The start-up steps as the global's name or `top(module)`.
    fn init(&self) -> Vec<String> {
        self.program
            .init
            .iter()
            .map(|step| match step {
                Init::Global(def) => self.interner.resolve(self.program.defs[*def].name).to_string(),
                Init::TopLevel(module) => format!("top({})", path_text(&self.interner, &self.program.modules[*module].path)),
            })
            .collect()
    }
}

#[test]
fn globals_run_in_source_order_unless_one_reads_another() {
    let run = main("let a = 1\nlet b = 2\nlet c = d + 1\nlet d = 3");
    assert!(run.errors.is_empty(), "{:?}", run.errors);
    assert_eq!(run.init(), ["a", "b", "d", "c"]);
}

#[test]
fn a_read_through_a_function_orders_the_global() {
    let run = main("let a: Int = f()\nlet b = 2\nfn f() -> Int { return g() }\nfn g() -> Int { return b }");
    assert!(run.errors.is_empty(), "{:?}", run.errors);
    assert_eq!(run.init(), ["b", "a"]);
}

#[test]
fn a_global_that_reads_itself_through_functions_is_a_cycle() {
    let run = main("let a: Int = f()\nfn f() -> Int { return a }");
    assert_eq!(run.errors, ["`a` depends on itself"]);
    let run = main("let a: Int = f()\nlet b: Int = g()\nfn f() -> Int { return b }\nfn g() -> Int { return a }");
    assert_eq!(run.errors, ["initialization cycle: `a` -> `b` -> `a`"]);
}

#[test]
fn a_module_starts_after_the_modules_it_imports() {
    let run = build(&[("main", "import .util\nlet a = util.base + 1\nvar n = 0\nn += a"), ("util", "pub let base = 1\nvar m = 0\nm += 1")]);
    assert!(run.errors.is_empty(), "{:?}", run.errors);
    assert_eq!(run.init(), ["base", "m", "top(util)", "a", "n", "top(main)"]);
}

#[test]
fn a_destructured_let_is_one_step_under_its_first_name() {
    let run = main("let c = a + b\nlet (a, b) = (1, 2)");
    assert!(run.errors.is_empty(), "{:?}", run.errors);
    assert_eq!(run.init(), ["a", "c"]);
}

#[test]
fn a_channel_end_is_moved_by_the_spawn_that_captures_it() {
    let run = with_sync("fn f(tx: Sender<Int>, rx: Receiver<Int>) {\n spawn { tx.send(1) }\n tx.send(2)\n let t = spawn { rx.recv() }\n for v in rx { }\n}");
    assert_eq!(run.errors, ["`tx` was moved into a task", "`rx` was moved into a task"]);
}

#[test]
fn a_channel_end_is_moved_on_some_path_when_one_branch_spawns() {
    let run = with_sync(
        "fn a(tx: Sender<Int>, flag: Bool) {\n if flag { spawn { tx.send(1) } } else { tx.send(2) }\n}\nfn b(tx: Sender<Int>, flag: Bool) {\n if flag { spawn { tx.send(1) } }\n tx.send(2)\n}\nfn c(tx: Sender<Int>, flag: Bool) {\n if flag {\n spawn { tx.send(1) }\n return\n }\n tx.send(2)\n}",
    );
    assert_eq!(run.errors, ["`tx` was moved into a task"]);
}

#[test]
fn a_spawn_in_a_loop_moves_the_end_for_the_next_turn() {
    let run = with_sync("fn f(tx: Sender<Int>, go: Bool) {\n while go { spawn { tx.send(1) } }\n}\nfn g(tx: Sender<Int>, go: Bool) {\n while go {\n spawn { tx.send(1) }\n break\n }\n}");
    assert_eq!(run.errors, ["`tx` was moved into a task"]);
}

#[test]
fn a_lambda_over_a_moved_end_is_a_use() {
    let run = with_sync("fn f(tx: Sender<Int>) {\n spawn { tx.send(1) }\n let g = || tx.send(3)\n}");
    assert_eq!(run.errors, ["`tx` was moved into a task"]);
}

#[test]
fn other_captures_and_handles_are_not_moved() {
    let run = with_sync("fn f(n: Int, t: Task<Int>) {\n spawn { return n }\n let m = n\n t.await()\n spawn { return t.await() }\n}");
    assert!(run.errors.is_empty(), "{:?}", run.errors);
}

#[test]
fn lock_must_not_park() {
    let run = with_sync("fn f(m: Mutex<Int>, rx: Receiver<Int>, tx: Sender<Int>) {\n m.lock(|n| { rx.recv() })\n m.lock(|n| { relay(tx) })\n m.lock(|n| { for v in rx { } })\n m.lock(|n| { sleep() })\n}\nfn relay(tx: Sender<Int>) { tx.send(1) }");
    assert_eq!(
        run.errors,
        [
            "`lock` must not park; its body reaches `Receiver.recv`",
            "`lock` must not park; its body reaches `Sender.send`",
            "`lock` must not park; its body reaches `for over a Receiver`",
            "`lock` must not park; its body reaches `sleep`"
        ]
    );
}

#[test]
fn lock_inside_lock_and_lock_by_a_named_function_are_followed() {
    let run = with_sync("fn slow(var n: Int) { sleep() }\nfn quick(var n: Int) { n += 1 }\nfn f(m: Mutex<Int>, other: Mutex<Int>) {\n m.lock(|n| { other.lock(|k| { k += 1 }) })\n m.lock(slow)\n m.lock(quick)\n}");
    assert_eq!(run.errors, ["`lock` must not park; its body reaches `Mutex.lock`", "`lock` must not park; its body reaches `sleep`"]);
}

#[test]
fn get_and_set_of_a_mutex_wait_like_lock() {
    let run = with_sync("fn f(m: Mutex<Int>, other: Mutex<Int>) {\n m.lock(|n| { other.get() })\n m.lock(|n| { other.set(1) })\n}");
    assert_eq!(run.errors, ["`lock` must not park; its body reaches `Mutex.get`", "`lock` must not park; its body reaches `Mutex.set`"]);
}

#[test]
fn lock_may_spawn_and_do_plain_work() {
    let run = with_sync("fn f(m: Mutex<Int>, tx: Sender<Int>) {\n m.lock(|n| { n += 1 })\n m.lock(|n| { spawn { tx.send(1) } })\n}");
    assert!(run.errors.is_empty(), "{:?}", run.errors);
}

#[test]
fn a_task_started_at_start_up_reads_no_var_global() {
    let run = with_sync(
        "var count = 0\nlet fixed = 1\nfn read() -> Int { return count }\nfn boot() { spawn { return count } }\nfn later() { spawn { return count } }\nlet a = spawn { return count }\nlet b = spawn { return read() }\nlet c = spawn { return fixed }\nboot()",
    );
    assert_eq!(run.errors, ["a task started at start-up cannot read the `var` global `count`"; 3]);
}
