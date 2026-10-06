//! Union types in the checker, `is`/`as` with union targets, narrowing and common members.

use std::process::Command;

fn mote(cmd: &str, source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_union_{}_{}_{}", cmd, tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg(cmd).arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn rejected(body: &str, tag: &str, message: &str) {
    let (ok, text) = mote("check", &format!("import {{ Any }} from std.experimental.types\npub fn main() {{\n{body}\n}}\n"), tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

#[test]
fn unions_narrow_and_settle_from_context() {
    let src = r#"import { Any } from std.experimental.types
type Id = Int | String
fn show(x: Id) -> String {
    if x is Int { return "int ${x + 1}" }
    return "str ${x.len()}"
}
fn kind(x: Int | String | Bool) -> String {
    if x is Int { return "i" }
    let rest: String | Bool = x
    return x is String ? "s:" + x : "b"
}
fn opt(x: (Int | String)?) -> String {
    if x == null { return "none" }
    return show(x)
}
fn mk(b: Bool) -> Int | String { return b ? 1 : "one" }
pub fn main() {
    println([show(3), show("hey"), kind(1), kind("q"), kind(true), opt(None), opt(4)])
    let xs: List<Int | String> = [1, "a"]
    let m: Map<Int | String, Int> = {1: 10, "a": 20}
    let f = |x: (Int | String)| x.to_string() + "!"
    let h: (Int) -> Int | String = |n: Int| n > 0 ? n : "neg"
    println([xs.len(), m["a"]])
    println([f(3), h(-1).to_string(), mk(false).to_string()])
    let a: Any = 5
    if a is (Int | String) { println("narrowed ${a}") }
    let u: Int | String = a
    println(u == 5)
}
"#;
    let (ok, text) = mote("run", src, "narrow");
    assert!(
        ok && text == "[\"int 4\", \"str 3\", \"i\", \"s:q\", \"b\", \"none\", \"int 5\"]\n[2, 20]\n[\"3!\", \"neg\", \"one\"]\nnarrowed 5\ntrue\n",
        "{text}"
    );
}

#[test]
fn common_members_dispatch_on_the_runtime_type() {
    let src = r#"class Circle { r: Int
    pub fn area(self) -> Int { return 3 * self.r * self.r }
    pub fn len(self) -> Int { return 99 }
}
class Square { side: Int  r: Int
    pub fn area(self) -> Int { return self.side * self.side }
}
class Box<T> { v: T }
fn which(b: Box<Int> | Box<String>) -> String { return b is Box<Int> ? "bi" : "bs" }
pub fn main() {
    let shapes: List<Circle | Square> = [Circle { r: 1 }, Square { side: 2, r: 0 }]
    for s in shapes { println([s.area(), s.r]) }
    let c: Circle | String = "hello"
    let d: Circle | String = Circle { r: 1 }
    println([c.len(), d.len()])
    println([which(Box { v: 1 }), which(Box { v: "s" })])
    let sq: Circle | Square = Square { side: 5, r: 0 }
    if sq is Square { println(sq.side) }
}
"#;
    let (ok, text) = mote("run", src, "members");
    assert!(ok && text == "[3, 1]\n[4, 0]\n[5, 99]\n[\"bi\", \"bs\"]\n5\n", "{text}");
}

#[test]
fn a_failed_check_into_a_union_is_a_located_fault() {
    let (ok, text) = mote("run", "import { Any } from std.experimental.types\npub fn main() {\n    let a: Any = true\n    let b: Int | String = a\n}\n", "fault");
    assert!(!ok && text.contains("expected `Int | String`, found `Bool`") && text.contains("main.mote:4"), "{text}");
}

#[test]
fn union_misuse_is_a_compile_error() {
    rejected("    let a: Int | String? = 1", "optional", "a union member can't be optional; write `(Int | String)?`");
    rejected("    let a: Int | Any = 1", "any", "`Any` can't be a union member");
    rejected(
        "    let a: (Int, Int) | (String, Bool) = (1, 1)",
        "tuples",
        "a union can't hold two tuple types of the same length, `(Int, Int)` and `(String, Bool)`",
    );
    rejected("    let a: ((Int) -> Int) | ((String) -> Int) = |x: Int| x", "fns", "a union can't hold two function types");
    rejected("    let a: Int | String = 1\n    println(a + 1)", "op", "`+` needs a narrowed operand, but this one is `Int | String`");
    rejected("    let a: Int | String = 1\n    println(a.len())", "member", "`.len()` is not on every member of `Int | String`: `Int` has none");
    rejected("    let a: Int | String = 1\n    let b: Int = a", "out", "of type 'Int' with expression of type 'Int | String'");
    rejected("    let xs = [1, \"a\"]", "list", "annotate the literal's type (for example `List<Int | String>`)");
    rejected("    let c = true ? 1 : \"a\"", "ternary", "annotate the result's type, for example `Int | String`");
    rejected("    let xs: List<Int> = [1]\n    let ys: List<Int | String> = xs", "invariant", "of type 'List<Int | String>' with expression of type 'List<Int>'");
    rejected("    var v: Int | String = 1\n    if v is Int { println(v + 1) }", "var", "`+` needs a narrowed operand");
}

#[test]
fn match_type_patterns_bind_guard_and_nest() {
    let src = r#"import { Any } from std.experimental.types
class Cat { name: String }
fn d(x: Int | String | Bool) -> String {
    match x {
        n: Int if n > 0 => { return "pos ${n + 0}" }
        n: Int => { return "int" }
        s: String => { return "str ${s.len()}" }
        _: Bool => { return "bool" }
    }
    return "?"
}
fn e(x: (Int | Cat)?) -> String {
    match x {
        c: Cat => { return c.name }
        _: Int | null => { return "int or none" }
    }
    return "?"
}
fn f(x: Any) -> String {
    match x {
        v: (Int | String) => { return "is ${v}" }
        l: List<Int> => { return "list ${l.len()}" }
        _ => { return "other" }
    }
    return "?"
}
pub fn main() {
    println([d(3), d(-1), d("ab"), d(true)])
    println([e(Cat { name: "tom" }), e(3), e(None)])
    println([f(1), f("s"), f([1, 2]), f(true)])
    let t: (Int | String, Int) = ("a", 2)
    match t {
        (s: String, n) => println("tuple ${s} ${n}")
        _ => println("no")
    }
}
"#;
    let (ok, text) = mote("run", src, "patterns");
    assert!(
        ok && text == "[\"pos 3\", \"int\", \"str 2\", \"bool\"]\n[\"tom\", \"int or none\", \"int or none\"]\n[\"is 1\", \"is s\", \"list 2\", \"other\"]\ntuple a 2\n",
        "{text}"
    );
}

#[test]
fn type_pattern_misuse_is_a_compile_error() {
    let arms = |subject: &str, arms: &str| format!("    let x: {subject} = 1\n    match x {{\n{arms}\n    }}");
    rejected(&arms("Int | String | Bool", "        n: Int => println(n)\n        s: String => println(s)"), "missing", "`Bool` is not covered");
    rejected(&arms("(Int | String)?", "        n: Int => println(n)\n        s: String => println(s)"), "none", "`None` is not covered");
    rejected(&arms("Int | String", "        n: Int if n > 0 => println(n)\n        s: String => println(s)"), "guarded", "`Int` is not covered");
    rejected(
        &arms("Int | String", "        f: Float => println(f)\n        _ => println(0)"),
        "never",
        "this pattern can never match: a value of type `Int | String` is never a `Float`",
    );
    rejected(&arms("Any", "        n: Int => println(n)"), "any", "Non-exhaustive `match`: add a `_` arm");
}
