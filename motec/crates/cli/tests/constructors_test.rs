//! `T(args)` calls `T.new(args)`, and type arguments in expressions.

use std::process::Command;

fn mote(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_constructors_{}_{}", tag, std::process::id()));
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
class P {
    x: Int
    y: Int
    pub fn new(x: Int, y: Int) -> P { return P { x: x, y: y } }
    pub fn origin() -> P { return P { x: 0, y: 0 } }
}
class Box<T> {
    v: T
    pub fn new(v: T) -> Box<T> { return Box { v: v } }
}
pub fn main() {
    let a = P(1, 2)
    println(a.x + a.y)
    println(P.origin().x)
    println(Box(5).v + 1)
    println(Box<String>("hi").v)
    println(typeof(Box(5)))
    println(typeof(Box<Int>.new(3)))
    let m = Map<String, Int>()
    m["a"] = 1
    println(m)
    println(typeof(Set<Int>()))
    println(typeof(List<Int>()))
    let (tx, rx) = Channel<Int>(2)
    tx.send(7)
    println(rx.recv())
    println(typeof(Map<String, List<Int>>()))
    let n = 3
    let k = 4
    println(n < k && k > n)
}
"#;

#[test]
fn constructors_and_type_arguments_run() {
    let (ok, out) = mote(PROGRAM, "program");
    assert!(ok, "{out}");
    let want = "3\n0\n6\nhi\nBox<Int>\nBox<Int>\n{\"a\": 1}\nSet<Int>\nList<Int>\n7\nMap<String, List<Int>>\ntrue\n";
    assert_eq!(out, want);
}

#[test]
fn a_type_without_new_is_an_error() {
    fails_with("struct Q { x: Int }\npub fn main() { let q = Q(1) }\n", "no_new", "`Q` has no `new`");
}

#[test]
fn the_wrong_number_of_type_arguments_is_an_error() {
    let src = "class Box<T> { v: T\n pub fn new(v: T) -> Box<T> { return Box { v: v } } }\npub fn main() { let b = Box<Int, Int>(1) }\n";
    fails_with(src, "box_args", "`Box` takes 1 type argument(s), but 2 were given");
    fails_with("pub fn main() { let m = Map<String>() }\n", "map_args", "`Map` takes 2 type argument(s), but 1 were given");
}

#[test]
fn a_static_method_used_as_a_value_is_an_error() {
    let src = "class P { x: Int\n pub fn origin() -> P { return P { x: 0 } } }\npub fn main() { let f = P.origin }\n";
    fails_with(src, "method_value", "`P.origin` is a method; call it");
}

#[test]
fn a_function_named_like_a_type_is_an_error() {
    fails_with("struct P { x: Int }\nfn P() -> Int { return 1 }\npub fn main() { }\n", "fn_type", "function `P` has the same name as a type");
}

#[test]
fn a_generic_function_takes_written_type_arguments() {
    let src = "import std.iter as it\nfn first<T>(xs: List<T>) -> T? { return xs.first() }\nfn empty<T>() -> List<T> { return [] }\npub fn main() {\n    println(first<Int>([1]))\n    println(typeof(empty<String>()))\n    println(it.map<Int, String>([1, 2], |x| \"n${x}\"))\n}\n";
    let (ok, out) = mote(src, "generic_fn");
    assert!(ok, "{out}");
    assert_eq!(out, "1\nList<String>\n[\"n1\", \"n2\"]\n");
    let src = "fn first<T>(xs: List<T>) -> T? { return xs.first() }\npub fn main() { println(first<Int, Int>([1])) }\n";
    fails_with(src, "generic_fn_count", "`first` takes 1 type argument(s), but 2 were given");
    let src = "fn first<T>(xs: List<T>) -> T? { return xs.first() }\npub fn main() { println(first<String>([1])) }\n";
    fails_with(src, "generic_fn_type", "`first` argument 1 expects 'List<String>', found 'List<Int>'");
}
