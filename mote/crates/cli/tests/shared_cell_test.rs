//! `Shared<T>` cells: `Shared(v)`, `into_shared`, `get`, `update`, `set`, `wait_until` and `.clone()`.

use std::process::Command;

fn mote(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_shared_cell_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn fails_with(source: &str, tag: &str, want: &str) {
    let (ok, out) = mote(source, tag);
    assert!(!ok && out.contains(want), "{tag}: expected `{want}`, got:\n{out}");
}

const PROGRAM: &str = r#"
class Stats {
    var hits: Int
    var names: List<String>
}
class Counter {
    hits: Shared<Int>
    pub fn bump(self) { self.hits.update(|n| { n += 1 }) }
}
pub fn main() {
    var hits = Shared(0)
    hits.update(|n| { n += 1 })
    hits.update(|n| { n += 2 })
    println(hits.get())
    var st = Shared(Stats { hits: 0, names: [] })
    st.update(|s| {
        s.hits += 1
        s.names.push("a")
    })
    let snap = st.get()
    st.update(|s| { s.names.push("b") })
    println(snap.names)
    println(st.get().names)
    let copy = snap.clone()
    copy.names.push("z")
    println(copy.names)
    st.set(Stats { hits: 9, names: ["x"] })
    println(st.get().hits)
    let other = st
    println(other == st)
    var xs = [1, 2]
    let cell = xs.into_shared()
    println(cell.get())
    let c = Counter { hits: Shared(0) }
    c.bump()
    c.bump()
    println(c.hits.get())
    var total = Shared(0)
    scope {
        for i in 0..10 {
            spawn { total.update(|t| { t += i }) }
        }
    }
    println(total.get())
}
"#;

#[test]
fn cells_read_snapshots_and_commit_writes() {
    let (ok, out) = mote(PROGRAM, "program");
    assert!(ok, "{out}");
    assert_eq!(out, "3\n[\"a\"]\n[\"a\", \"b\"]\n[\"a\", \"z\"]\n9\ntrue\n[1, 2]\n2\n45\n");
}

#[test]
fn a_copy_leaves_the_original_writable() {
    let src = "pub fn main() {\n    var xs = [1]\n    let s = Shared(xs)\n    xs.push(2)\n    println(s.get())\n    println(xs)\n}\n";
    let (ok, out) = mote(src, "copy");
    assert!(ok && out == "[1]\n[1, 2]\n", "{out}");
}

#[test]
fn a_fault_inside_update_commits_nothing() {
    let src = "pub fn main() {\n    var s = Shared([1])\n    let t = spawn {\n        s.update(|v| {\n            v.push(2)\n            let z = [1][5]\n        })\n    }\n    println(t.join().is_err())\n    s.update(|v| { v.push(3) })\n    println(s.get())\n}\n";
    let (ok, out) = mote(src, "fault");
    assert!(ok && out == "true\n[1, 3]\n", "{out}");
}

#[test]
fn an_update_inside_its_own_update_faults() {
    fails_with("pub fn main() {\n    var s = Shared(1)\n    s.update(|n| { s.update(|m| { m += 1 }) })\n}\n", "nested", "a `Shared` was written inside its own `update`");
}

#[test]
fn writing_needs_a_var() {
    fails_with("pub fn main() {\n    let s = Shared(0)\n    s.update(|n| { n += 1 })\n}\n", "let", "`s` is a `let`; only a `var` can write a `Shared`");
    fails_with("fn f(s: Shared<Int>) { s.set(1) }\npub fn main() { }\n", "param", "`s` is a read-only parameter; declare it `var s`");
    let (ok, out) = mote("fn f(var s: Shared<Int>) { s.set(4) }\npub fn main() {\n    var s = Shared(0)\n    f(s)\n    println(s.get())\n}\n", "var_param");
    assert!(ok && out == "4\n", "{out}");
}

#[test]
fn a_snapshot_is_read_only() {
    let p = "class P {\n    var x: Int\n}\n";
    fails_with(&format!("{p}pub fn main() {{\n    var s = Shared(P {{ x: 1 }})\n    let v = s.get()\n    v.x = 2\n}}\n"), "binding", "`v` is a snapshot; `.clone()` it to change it");
    fails_with("pub fn main() {\n    var s = Shared([1])\n    var v = s.get()\n    v.push(2)\n}\n", "var_binding", "`v` is a snapshot");
    fails_with("pub fn main() {\n    var s = Shared([1])\n    s.get().push(2)\n}\n", "direct", "a snapshot is read-only; `.clone()` it to change it");
}

#[test]
fn the_cell_is_read_only_through_get() {
    let p = "class P {\n    var x: Int\n}\n";
    fails_with(&format!("{p}pub fn main() {{\n    var s = Shared(P {{ x: 1 }})\n    println(s.x)\n}}\n"), "field", "a `Shared` has no field `x`; read it from a snapshot: `.get().x`");
    fails_with("pub fn main() {\n    var s = Shared([1])\n    println(s.len())\n}\n", "method", "a `Shared` has no method `.len()`");
    fails_with("pub fn main() {\n    var s = Shared([1])\n    println(s[0])\n}\n", "index", "a `Shared` cannot be indexed");
    fails_with("pub fn main() {\n    let s = [1].shared()\n}\n", "old", "no method `.shared()`; write `Shared(x)`");
}

#[test]
fn a_cell_value_must_be_sealable() {
    fails_with("pub fn main() {\n    var s = Shared(|x: Int| x)\n}\n", "fn", "a `Shared` cannot hold a function");
    fails_with("pub fn main() {\n    var s = Shared(1)\n    s.update(|n| n + 1)\n}\n", "result", "`.update()`'s function returns nothing; change its parameter instead");
    fails_with("pub fn main() {\n    var xs = [1]\n    let s = xs.into_shared()\n    xs.push(2)\n}\n", "moved", "consumed by `into_shared`");
}

#[test]
fn a_cell_is_invariant() {
    fails_with("import { Any } from std.experimental.types\npub fn main() {\n    var a = Shared(1)\n    var b: Shared<Any> = a\n}\n", "invariant", "of type 'Shared<Any>' with expression of type 'Shared<Int>'");
}

#[test]
fn a_snapshot_behind_any_is_sealed_at_run_time() {
    let src = "import { Any } from std.experimental.types\npub fn main() {\n    var s = Shared([1])\n    var bag: List<Any> = []\n    bag.push(s.get())\n    var l: List<Int> = bag[0]\n    l.push(3)\n}\n";
    fails_with(src, "any", "cannot change a collection: it is shared and read-only");
}

#[test]
fn clone_is_a_mutable_deep_copy() {
    let src = "class C {\n    var n: Int\n    pub fn clone(self) -> C { return C { n: self.n + 100 } }\n}\npub fn main() {\n    println(C { n: 1 }.clone().n)\n    let m = {\"a\": [1]}\n    let m2 = m.clone()\n    m2[\"a\"].push(2)\n    println(m)\n    println(m2)\n}\n";
    let (ok, out) = mote(src, "clone");
    assert!(ok && out == "101\n{\"a\": [1]}\n{\"a\": [1, 2]}\n", "{out}");
}

#[test]
fn wait_until_returns_the_first_version_that_holds() {
    let src = "class Job {\n    var done: Int\n}\npub fn main() {\n    var n = Shared(0)\n    println(n.wait_until(|v| v == 0))\n    var job = Shared(Job { done: 0 })\n    scope {\n        spawn { println(job.wait_until(|j| j.done >= 3).done >= 3) }\n        for i in 0..5 {\n            spawn { job.update(|j| { j.done += 1 }) }\n        }\n    }\n    var f = Shared(0.0)\n    scope {\n        spawn { println(f.wait_until(|x| x > 1.0)) }\n        spawn { f.set(2.5) }\n    }\n}\n";
    let (ok, out) = mote(src, "wait");
    assert!(ok && out == "0\ntrue\n2.5\n", "{out}");
}

#[test]
fn wait_until_is_a_cancellation_point() {
    let src = "pub fn main() {\n    var s = Shared(0)\n    let t = spawn { s.wait_until(|v| v > 5) }\n    t.cancel()\n    println(t.join().is_err())\n    let u = spawn { s.wait_until(|v| v > 5) }\n    s.set(9)\n    println(u.join().unwrap())\n}\n";
    let (ok, out) = mote(src, "wait_cancel");
    assert!(ok && out == "true\n9\n", "{out}");
}

#[test]
fn wait_until_gives_a_snapshot_and_needs_a_bool() {
    let p = "class P {\n    var x: Int\n}\n";
    fails_with(&format!("{p}pub fn main() {{\n    var s = Shared(P {{ x: 1 }})\n    let v = s.wait_until(|p| p.x > 0)\n    v.x = 2\n}}\n"), "wait_snap", "`v` is a snapshot");
    fails_with("pub fn main() {\n    var s = Shared(0)\n    s.wait_until(|v| v + 1)\n}\n", "wait_bool", "`.wait_until()`'s function must return 'Bool', found 'Int'");
}

#[test]
fn a_cancelled_writer_gives_up_its_turn() {
    let src = "pub fn main() {\n    var s = Shared(5)\n    let t = spawn {\n        s.update(|v| {\n            v += 1\n            while true { v += 0 }\n        })\n    }\n    t.cancel()\n    println(t.join().is_err())\n    s.update(|v| { v += 1 })\n    println(s.get())\n}\n";
    let (ok, out) = mote(src, "cancel_writer");
    assert!(ok && out == "true\n6\n", "{out}");
}

#[test]
fn the_lock_types_are_gone() {
    fails_with("pub fn main() {\n    let m = Mutex(0)\n}\n", "mutex", "there is no `Mutex`; use `Shared(x)` and `.update()`");
    fails_with("pub fn main() {\n    let a: Atomic = Shared(0)\n}\n", "atomic", "there is no `Atomic`; use `Shared(0)` and `.update()`");
    fails_with("pub fn main() {\n    let x = nosuch(1)\n}\n", "nofn", "no function `nosuch`");
}
