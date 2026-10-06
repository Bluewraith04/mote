//! `xs[i]`, `xs[i] = v` and `xs[i] op= v` on `List`, `Map`, `Bytes` and `Any`.

use std::process::Command;

fn mote(verb: &str, source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_index_{}_{}_{}", verb, tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg(verb).arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn rejected(body: &str, tag: &str, message: &str) {
    let (ok, text) = mote("check", &format!("pub fn main() {{\n{body}\n}}\n"), tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

#[test]
fn reads_and_writes_lists_maps_and_bytes() {
    let src = r#"import { Any } from std.experimental.types
pub fn main() {
    var xs: List<Int> = [10, 20, 30]
    xs[1] = 99
    println(xs[0] + xs[1])
    let m: Map<String, Int> = Map()
    m["a"] = 4
    println(m["a"])
    let b = "hi".bytes()
    b[0] = 72
    println(b[0])
    let grid: List<List<Int>> = [[1, 2], [3, 4]]
    grid[1][0] = 7
    println(grid[1][0])
    let a: Any = xs
    println(a[2])
}
"#;
    let (ok, text) = mote("run", src, "rw");
    assert!(ok && text == "109\n4\n72\n7\n30\n", "{text}");
}

#[test]
fn compound_assignment_evaluates_receiver_and_index_once() {
    let src = r#"fn at() -> Int {
    println("at")
    return 1
}
pub fn main() {
    let xs: List<Int> = [1, 2]
    xs[at()] += 40
    let m: Map<String, Int> = Map()
    m["k"] = 3
    m["k"] *= 5
    println(xs[1])
    println(m["k"])
}
"#;
    let (ok, text) = mote("run", src, "compound");
    assert!(ok && text == "at\n42\n15\n", "{text}");
}

#[test]
fn out_of_range_faults_at_the_index() {
    let (ok, text) = mote("run", "pub fn main() {\n    let xs: List<Int> = [1]\n    println(xs[-1])\n}\n", "oob");
    assert!(!ok && text.contains("out of bounds") && text.contains("main.mote:3:13"), "{text}");
}

#[test]
fn misuse_is_a_compile_error() {
    rejected("    let s = \"hi\"\n    println(s[0])", "string", "a `String` has no integer indexing");
    rejected("    let n = 5\n    println(n[0])", "int", "`Int` cannot be indexed");
    rejected("    let xs: List<Int> = [1]\n    xs[\"a\"] = 2", "idx_type", "`[]` index expects 'Int'");
    rejected("    let xs: List<Int> = [1]\n    xs[0] = \"a\"", "val_type", "`[]` value expects 'Int'");
    rejected("    let xs: List<Int> = [1]\n    let s: String = xs[0]", "elem_type", "of type 'Int'");
    rejected("    var t = Shared([1])\n    t[0] = 5", "shared", "a `Shared` cannot be indexed");
}
