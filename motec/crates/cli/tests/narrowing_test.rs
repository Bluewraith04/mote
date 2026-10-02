//! Checks narrow `let` bindings and parameters; reading a member of an unchecked optional is a compile error.

use std::process::Command;

fn mote(cmd: &str, source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_narrowing_{}_{}_{}", cmd, tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg(cmd).arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

const NODE: &str = "class Node { val: Int  next: Node? }\n";
const UNCHECKED: &str = "`Node?` may be `None`, so `.val` needs `?.`, `!`, `??`, `match`, or a `!= null` check first";

fn rejected(body: &str, tag: &str, message: &str) {
    let (ok, text) = mote("check", &format!("{NODE}pub fn main() {{\n{body}\n}}\n"), tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

#[test]
fn checks_narrow_in_branches_guards_and_operators() {
    let src = r#"import { Any } from std.experimental.types
fn a(n: Node?) -> Int {
    if n != null { return n.val }
    return -1
}
fn b(n: Node?) -> Int {
    if n == null { return -1 }
    return n.val
}
fn c(n: Node?) -> Int {
    if n == None { return -1 } else { return n.val }
}
fn d(n: Node?) -> Int {
    return n != null && n.val > 0 ? n.val : 0
}
fn e(n: Node?) -> Bool {
    return n == null || n.val == 0
}
fn f(x: Any) -> Int {
    if x is Int { return x + 1 }
    return 0
}
fn g(s: String?) -> Int {
    if !(s is String) { return 0 }
    return s.len()
}
fn total(xs: List<Node?>) -> Int {
    var t = 0
    for x in xs {
        if x == null { continue }
        t = t + x.val
    }
    return t
}
pub fn main() {
    let n = Node { val: 5, next: None }
    println([a(n), a(None), b(n), b(None), c(n), c(None), d(n), d(None)])
    println([e(n), e(None), f(3) == 4, f("s") == 0, g("abc") == 3, g(None) == 0])
    println(total([n, None, n]))
}
"#;
    let (ok, text) = mote("run", &format!("{NODE}{src}"), "forms");
    assert!(ok && text == "[5, -1, 5, -1, 5, -1, 5, 0]\n[false, true, true, true, true, true]\n10\n", "{text}");
}

#[test]
fn narrowing_a_nested_optional_unwraps_one_level_at_a_time() {
    let src = r#"fn levels(x: Int???) -> Int {
    if x == null { return 0 }
    if x == null { return 1 }
    if x == null { return 2 }
    return x + 10
}
fn or_default<T>(x: T?, d: T) -> T {
    if x != null { return x }
    return d
}
pub fn main() {
    let a: Int??? = None
    let b: Int??? = Some(None)
    let c: Int??? = Some(Some(None))
    let d: Int??? = Some(Some(Some(5)))
    println([levels(a), levels(b), levels(c), levels(d)])
    let inner: Int? = None
    let r: Int? = or_default(Some(inner), 9)
    println([r == null, or_default(None, 4) == 4])
}
"#;
    let (ok, text) = mote("run", src, "nested");
    assert!(ok && text == "[0, 1, 2, 15]\n[true, true]\n", "{text}");
}

#[test]
fn unchecked_optional_access_is_a_compile_error() {
    rejected("    let n: Node? = None\n    println(n.val)", "field", UNCHECKED);
    rejected(
        "    let s: String? = None\n    println(s.len())",
        "method",
        "`String?` may be `None`, so `.len()` needs `?.`, `!`, `??`, `match`, or a `!= null` check first",
    );
    rejected("    var v: Node? = None\n    if v != null { println(v.val) }", "var", UNCHECKED);
    rejected("    let n: Node? = None\n    if n != null { println(1) }\n    println(n.val)", "after_branch", UNCHECKED);
    rejected(
        "    let n: Node? = Node { val: 1, next: None }\n    if n != null {\n        let n: Node? = None\n        println(n.val)\n    }",
        "shadowed",
        UNCHECKED,
    );
    rejected(
        "    let n = Node { val: 1, next: None }\n    if n.next != null { println(n.next.val) }",
        "field_never_narrows",
        UNCHECKED,
    );
}

#[test]
fn option_methods_and_to_string_need_no_check() {
    let src = "pub fn main() {\n    let s: String? = None\n    println([s.is_none(), s.unwrap_or(\"d\") == \"d\"])\n    println(s.to_string())\n}\n";
    let (ok, text) = mote("run", src, "methods");
    assert!(ok && text == "[true, true]\nnull\n", "{text}");
}
