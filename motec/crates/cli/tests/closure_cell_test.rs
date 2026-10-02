//! A lambda shares the `var`s it captures; `spawn` still copies.

use std::process::Command;

fn run(source: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_closure_cell_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_lambda_updates_the_var_it_captures() {
    let src = "fn main() {\n    var n = 0\n    let inc = || { n += 1 }\n    inc()\n    inc()\n    println(n)\n    n = 10\n    let get = || n\n    println(get())\n}\n";
    assert_eq!(run(src, "update"), "2\n10\n");
}

#[test]
fn a_returned_counter_keeps_its_state() {
    let src = "import { Any } from std.experimental.types\nfn counter() -> Any {\n    var n = 0\n    return || {\n        n += 1\n        n\n    }\n}\nfn main() {\n    let c = counter()\n    let d = counter()\n    c()\n    c()\n    println(c())\n    println(d())\n}\n";
    assert_eq!(run(src, "counter"), "3\n1\n");
}

#[test]
fn nested_lambdas_share_one_cell() {
    let src = "fn main() {\n    var total = 0\n    let outer = || {\n        let inner = || { total += 5 }\n        inner()\n        total += 1\n    }\n    outer()\n    println(total)\n}\n";
    assert_eq!(run(src, "nested"), "6\n");
}

#[test]
fn each_loop_iteration_gets_its_own_var() {
    let src = "import { Any } from std.experimental.types\nfn main() {\n    var fs: List<Any> = []\n    for i in [1, 2, 3] {\n        var v = i\n        fs.push(|| {\n            v += 10\n            v\n        })\n    }\n    for f in fs {\n        println(f())\n    }\n}\n";
    assert_eq!(run(src, "loop"), "11\n12\n13\n");
}

#[test]
fn spawn_still_copies() {
    let src = "fn main() {\n    var k = 1\n    scope {\n        spawn {\n            k = 99\n        }\n    }\n    println(k)\n}\n";
    assert_eq!(run(src, "spawn"), "1\n");
}

const STRUCTS: &str = "struct P {\n    var x: Int\n    pub fn bump(var self) { self.x += 1 }\n}\nstruct O {\n    var inner: P\n}\n";

#[test]
fn a_lambda_writes_the_struct_var_it_captures() {
    let src = format!("{STRUCTS}fn main() {{\n    var p = P {{ x: 1 }}\n    let f = || {{ p.x = 9 }}\n    f()\n    println(p.x)\n    let q = p\n    let g = || {{ p.bump() }}\n    g()\n    println(\"${{p.x}} ${{q.x}}\")\n    var o = O {{ inner: P {{ x: 1 }} }}\n    let h = || {{ o.inner.x = 7 }}\n    h()\n    println(o.inner.x)\n}}\n");
    assert_eq!(run(&src, "struct_lambda"), "9\n10 9\n7\n");
}

#[test]
fn spawn_copies_a_struct_it_captures() {
    let plain = format!("{STRUCTS}fn main() {{\n    var p = P {{ x: 1 }}\n    scope {{\n        spawn {{ p.x = 100 }}\n    }}\n    println(p.x)\n}}\n");
    assert_eq!(run(&plain, "spawn_struct"), "1\n");
    let shared = format!("{STRUCTS}fn main() {{\n    var p = P {{ x: 1 }}\n    let f = || {{ p.x = 9 }}\n    f()\n    scope {{\n        spawn {{ p.x = 100 }}\n    }}\n    println(p.x)\n}}\n");
    assert_eq!(run(&shared, "spawn_struct_cell"), "9\n");
    let nested = format!("{STRUCTS}fn main() {{\n    var o = O {{ inner: P {{ x: 1 }} }}\n    scope {{\n        spawn {{ o.inner.x = 100 }}\n    }}\n    println(o.inner.x)\n}}\n");
    assert_eq!(run(&nested, "spawn_struct_nested"), "1\n");
}
