//! `T?` and `Option<T>` are one type; `None` is `null`, `Some(v)` is `v` or a `Some` cell; `??`, `??=` and `?.`.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_optional_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn some_is_the_value_and_none_is_null() {
    let src = r#"import { Any } from std.experimental.types
pub fn main() {
    let n: Option<Int> = None
    let a: Any = Some(5)
    println([Some(5).to_string(), n.to_string(), typeof(a)])
    println([a is Int, a is Option<Int>, a is Option<String>])
}
"#;
    let (ok, text) = run(src, "flat");
    assert!(ok && text == "[\"5\", \"null\", \"Int\"]\n[true, true, false]\n", "{text}");
}

#[test]
fn some_none_stays_distinct_from_none() {
    let src = r#"import { Any } from std.experimental.types
fn describe(o: Option<Option<Int>>) -> String {
    match o {
        Some(inner) => {
            match inner {
                Some(v) => { return "some(some(${v}))" }
                None => { return "some(none)" }
            }
        }
        None => { return "none" }
    }
}

fn wrap<T>(x: T) -> Option<T> {
    return Some(x)
}

fn flatten(o: Option<Option<Int>>) -> Option<Int> {
    let v = o?
    return v
}

pub fn main() {
    let n: Option<Int> = None
    let nn: Option<Option<Int>> = Some(None)
    println([describe(nn), describe(None), describe(Some(Some(3))), describe(wrap(n))])
    println([nn == Some(None), nn.is_some(), flatten(nn).is_none(), flatten(None).is_none()])
    let a: Any = nn
    println([a is Option<Option<Int>>, a is Option<Int>])
    println(nn)
}
"#;
    let (ok, text) = run(src, "nested");
    assert!(
        ok && text == "[\"some(none)\", \"none\", \"some(some(3))\", \"some(none)\"]\n[true, true, true, true]\n[true, false]\nSome(null)\n",
        "{text}"
    );
}

#[test]
fn optional_elements_come_back_as_some_none() {
    let src = r#"pub fn main() {
    let xs: List<Option<Int>> = [None, Some(2)]
    var seen: List<Bool> = []
    for x in xs {
        seen.push(x.is_none())
    }
    println(seen)
    let ys: List<Option<Int>> = [None]
    match ys.pop() {
        Some(v) => { println("popped ${v.is_none()}") }
        None => { println("empty") }
    }
    println(ys.pop().is_none())
}
"#;
    let (ok, text) = run(src, "elements");
    assert!(ok && text == "[true, false]\npopped true\ntrue\n", "{text}");
}

#[test]
fn combinators_work_on_the_new_values() {
    let src = r#"pub fn main() {
    let n: Option<Int> = None
    println([Some(1).map(|v| v * 10), n.map(|v| v * 10)])
    println(Some(4).unwrap_or(0) + n.unwrap_or(7))
    println([Some(4).ok_or("no"), n.ok_or("no")])
    println(n!)
}
"#;
    let (ok, text) = run(src, "combinators");
    assert!(!ok && text.starts_with("[10, null]\n11\n[Ok(4), Err(\"no\")]\n"), "{text}");
    assert!(text.contains("called `!` on an empty Option") && text.contains("main.mote:6:13"), "{text}");
}

fn check(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_optional_check_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("check").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn rejected(src: &str, tag: &str, message: &str) {
    let (ok, text) = check(src, tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

#[test]
fn t_optional_and_option_t_are_one_type() {
    let src = r#"fn half(n: Int) -> Int? {
    if n % 2 == 0 { return Some(n / 2) }
    return None
}

fn quarter(n: Int) -> Option<Int> {
    let h = half(n)?
    return half(h)
}

fn name(o: Int?) -> String {
    match o {
        Some(v) => { return "some ${v}" }
        null => { return "none" }
    }
}

pub fn main() {
    let a: Int? = 4
    let b: Option<Int> = a
    let c: Int?? = Some(None)
    let d: Option<Option<Int>> = c
    println([b.unwrap_or(0), a.map(|v| v + 1).unwrap(), quarter(8)!, quarter(6).unwrap_or(-1)])
    println([name(Some(3)), name(None), name(Option.Some(5)), name(Option.None)])
    println([d.is_some(), d!.is_none()])
    let w: List<Int??> = [Some(None), None, 3]
    println(w)
}
"#;
    let (ok, text) = run(src, "one_type");
    assert!(
        ok && text == "[4, 5, 2, -1]\n[\"some 3\", \"none\", \"some 5\", \"none\"]\n[true, true]\n[Some(null), null, 3]\n",
        "{text}"
    );
    rejected("pub fn main() {\n    let x: Option<Int> = \"s\"\n}\n", "shown", "variable 'x' of type 'Int?'");
}

#[test]
fn a_type_parameter_goes_into_an_optional_as_some() {
    let src = r#"fn wrap<T>(x: T) -> T? {
    return x
}

fn head<T>(xs: List<T>) -> T? {
    if xs.len() == 0 { return None }
    return xs.get(0)
}

pub fn main() {
    let n: Int? = None
    let xs: List<Int?> = [None]
    let ys: List<Int?> = []
    let deep: Int?? = n
    println([wrap(n).is_some(), head(xs).is_some(), head(ys).is_some(), deep.is_some()])
}
"#;
    let (ok, text) = run(src, "lift");
    assert!(ok && text == "[true, true, false, false]\n", "{text}");
}

#[test]
fn an_optional_is_sendable_when_its_payload_is() {
    let src = r#"pub fn main() {
    let o: Int? = 7
    var got: Int? = None
    scope {
        let t = spawn { return o.unwrap_or(0) + 1 }
        got = t.join().unwrap()
    }
    println(got)
}
"#;
    let (ok, text) = run(src, "send");
    assert!(ok && text == "8\n", "{text}");
}

#[test]
fn optional_misuse_is_a_compile_error() {
    rejected(
        "fn f() -> Result<Int, String> {\n    let o: Int? = None\n    let v = o?\n    return Ok(v)\n}\n",
        "try_mismatch",
        "`?` on an optional returns `None`, but this function returns a `Result`; use `.ok_or(e)?`",
    );
    rejected("fn f(o: Option) -> Int {\n    return 0\n}\n", "bare", "`Option` needs its type argument, as in `Option<T>` or `T?`");
    rejected("pub fn main() {\n    let s = Some(1, 2)\n}\n", "arity", "`Some` takes 1 value, but 2 were given");
    rejected("pub fn main() {\n    let o: Int? = 3\n    match o {\n        Some(v) => { println(v) }\n    }\n}\n", "exhaustive", "Non-exhaustive `match`");
    rejected("pub fn main() {\n    let o: Int? = 3\n    let n = o.unwrap_or(\"x\")\n}\n", "unwrap_or", "`.unwrap_or()` expects 'Int', found 'String'");
}

#[test]
fn coalesce_takes_the_payload_or_runs_the_fallback() {
    let src = r#"fn noisy(n: Int) -> Int {
    println("ran ${n}")
    return n
}
fn either<T>(x: T?, y: T?) -> T? { return x ?? y }
pub fn main() {
    let a: Int? = None
    let b: Int? = 3
    println([a ?? 7, b ?? noisy(9)])
    let c: Int? = a ?? b
    let deep: Int?? = Some(None)
    let e: Int? = deep ?? Some(8)
    let lifted: Int?? = either(deep, Some(Some(1)))
    println([c.is_some(), e.is_none(), lifted.is_some()])
}
"#;
    let (ok, text) = run(src, "coalesce");
    assert!(ok && text == "[7, 3]\n[true, true, true]\n", "{text}");
}

#[test]
fn coalesce_assign_writes_only_a_none() {
    let src = r#"class Slot { var v: Int? }
var g: Int? = None
g ??= 1
g ??= 2
fn noisy(n: Int) -> Int {
    println("ran ${n}")
    return n
}
pub fn main() {
    var v: Int? = None
    v ??= noisy(4)
    v ??= noisy(5)
    let s = Slot { v: None }
    s.v ??= 6
    var xs: List<Int?> = [None, 2]
    xs[0] ??= 10
    xs[1] ??= 20
    println([v, s.v, g])
    println(xs)
}
"#;
    let (ok, text) = run(src, "coalesce_assign");
    assert!(ok && text == "ran 4\n[4, 6, 1]\n[10, 2]\n", "{text}");
}

#[test]
fn optional_chain_reads_members_of_a_present_value() {
    let src = r#"class Node {
    val: Int
    next: Node?
    fn tag(self, s: String) -> String { return "${s}${self.val}" }
}
fn noisy(n: Int) -> Int {
    println("ran ${n}")
    return n
}
pub fn main() {
    let n1: Node? = Node { val: 1, next: Node { val: 2, next: None } }
    let none: Node? = None
    println([n1?.val, none?.val, n1?.next?.val, n1?.next?.next?.val])
    println([n1?.tag("n"), none?.tag("x${noisy(5)}")])
    let f = |x: Node?| x?.next?.val ?? 0
    println([f(n1), f(none)])
    let s: String? = "hi"
    println(s?.len())
}
"#;
    let (ok, text) = run(src, "chain");
    assert!(ok && text == "[1, null, 2, null]\n[\"n1\", null]\n[2, 0]\n2\n", "{text}");
}

#[test]
fn null_safe_misuse_is_a_compile_error() {
    rejected("pub fn main() {\n    let a = 1\n    println(a ?? 2)\n}\n", "coalesce_plain", "`??` needs an optional on its left, found `Int`");
    rejected(
        "pub fn main() {\n    let a: Int? = None\n    println(a ?? \"x\")\n}\n",
        "coalesce_type",
        "the right side of `??` must be `Int` or `Int?`, found `String`",
    );
    rejected("pub fn main() {\n    var a = 1\n    a ??= 2\n}\n", "assign_plain", "`??=` needs an optional on its left, found `Int`");
    rejected("pub fn main() {\n    var a: Int? = None\n    a ??= \"s\"\n}\n", "assign_type", "`??=` cannot assign `String` to `Int?`");
    rejected(
        "class P { x: Int }\npub fn main() {\n    let p = P { x: 1 }\n    println(p?.x)\n}\n",
        "chain_plain",
        "`?.` needs an optional on its left, found `P`",
    );
}

#[test]
fn a_nullable_type_before_a_colon_is_not_a_ternary() {
    let src = r#"import { Any } from std.experimental.types
class Pair { a: Int?  b: Int? }
fn first(x: Int?, y: Int?) -> Int? { return x ?? y }
pub fn main() {
    let p = Pair { a: None, b: 2 }
    let t: Any = 1
    println([first(p.a, p.b), t is Int? ? 1 : 0])
}
"#;
    let (ok, text) = run(src, "suffix");
    assert!(ok && text == "[2, 1]\n", "{text}");
}
