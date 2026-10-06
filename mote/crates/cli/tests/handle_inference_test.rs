//! `Shared(x)` infers `Shared<T>` from `x`; `spawn { }` infers `Task<T>` from its body.

use std::process::Command;

fn run(src: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_handle_infer_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn cell_and_task_types_are_inferred() {
    let src = "fn main() {\n    var a: Shared<Int> = Shared(0)\n    var b = Shared(\"hi\")\n    let s: String = b.get()\n    b.set(s + \"!\")\n    a.update(|n| { n += 1 })\n    var out = 0\n    scope {\n        let t: Task<Int> = spawn { 20 + 1 }\n        let u = spawn { return 21 }\n        out = t.join().unwrap() + u.join().unwrap()\n    }\n    println(\"${out} ${b.get()} ${a.get()}\")\n}\n";
    let (ok, text) = run(src, "run");
    assert!(ok && text == "42 hi! 1\n", "{text}");
}

#[test]
fn mismatched_handle_types_are_errors() {
    let body = |b: &str| format!("fn main() {{\n{b}\n}}\n");
    for (b, want) in [
        ("    var a = Shared(0)\n    a.set(\"x\")", "`.set()` expects 'Int', found 'String'"),
        ("    var a = Shared(0)\n    let c: Shared<String> = a", "of type 'Shared<String>' with expression of type 'Shared<Int>'"),
        ("    scope {\n        let t = spawn { \"s\" }\n        let n: Int = t.join().unwrap()\n    }", "of type 'Int' with expression of type 'String'"),
    ] {
        let (ok, text) = run(&body(b), "err");
        assert!(!ok && text.contains(want), "{b}: {text}");
    }
}
