//! Deep `let` on structs, read-only loop and payload bindings, `var` fields and what structs hold.

use std::process::Command;

fn mote(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_bindings_{}_{}", tag, std::process::id()));
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
class Imm {
    a: Int
}
struct Tag {
    name: String
    owner: Imm
    note: String?
    pair: (Int, String)
}
struct Label {
    s: String
    o: Imm
}
struct P {
    var x: Int
    pub fn bump(var self) { self.x += 1 }
}
class C {
    var n: Int
    var items: List<Int>
    var pos: P
}
pub fn main() {
    var p = P { x: 1 }
    p.x = 2
    p.bump()
    println(p.x)
    let c = C { n: 1, items: [], pos: P { x: 0 } }
    c.n = 3
    c.items.push(4)
    c.items[0] = 5
    c.pos.x = 7
    c.pos.bump()
    println("${c.n} ${c.items} ${c.pos.x}")
    let t = Tag { name: "t", owner: Imm { a: 9 }, note: null, pair: (1, "one") }
    var u = t
    println(u.name + t.owner.a.to_string() + u.pair.1)
    let l = Label { s: t.name, o: t.owner }
    let task = spawn { l.s }
    println(task.join().unwrap())
}
"#;

#[test]
fn var_fields_and_var_bindings_write() {
    let (ok, out) = mote(PROGRAM, "program");
    assert!(ok, "{out}");
    assert_eq!(out, "3\n3 [5] 8\nt9one\nt\n");
}

#[test]
fn a_let_struct_is_deep() {
    let s = "struct P {\n    var x: Int\n    pub fn bump(var self) { self.x += 1 }\n}\n";
    fails_with(&format!("{s}pub fn main() {{\n    let p = P {{ x: 1 }}\n    p.x = 5\n}}\n"), "let_assign", "`p` is a `let` struct; declare it `var p`");
    fails_with(&format!("{s}pub fn main() {{\n    let p = P {{ x: 1 }}\n    p.x += 5\n}}\n"), "let_compound", "`p` is a `let` struct; declare it `var p`");
    fails_with(&format!("{s}pub fn main() {{\n    let p = P {{ x: 1 }}\n    p.bump()\n}}\n"), "let_method", "`p` is a `let` struct; declare it `var p`");
}

#[test]
fn a_field_that_is_not_var_is_immutable() {
    fails_with("struct P {\n    x: Int\n}\npub fn main() {\n    var p = P { x: 1 }\n    p.x = 2\n}\n", "struct_field", "field `x` is not `var`; declare it `var x`");
    fails_with("class C {\n    n: Int\n}\npub fn main() {\n    let c = C { n: 1 }\n    c.n = 2\n}\n", "class_field", "field `n` is not `var`; declare it `var n`");
    fails_with("class C {\n    n: Int\n    pub fn set(var self) { self.n = 2 }\n}\npub fn main() { }\n", "self_field", "field `n` is not `var`; declare it `var n`");
}

#[test]
fn what_an_immutable_field_holds_cannot_change() {
    let c = "class C {\n    items: List<Int>\n}\n";
    fails_with(&format!("{c}pub fn main() {{\n    let c = C {{ items: [] }}\n    c.items.push(1)\n}}\n"), "push", "field `items` is not `var`");
    fails_with(&format!("{c}pub fn main() {{\n    let c = C {{ items: [1] }}\n    c.items[0] = 2\n}}\n"), "index", "field `items` is not `var`");
    fails_with(&format!("{c}pub fn main() {{\n    let c = C {{ items: [] }}\n    let xs = c.items\n    xs.push(1)\n}}\n"), "alias", "`xs` is read-only");
    let nested = "struct P {\n    var x: Int\n}\nclass C {\n    pos: P\n}\npub fn main() {\n    let c = C { pos: P { x: 1 } }\n    c.pos.x = 2\n}\n";
    fails_with(nested, "nested", "field `pos` is not `var`");
}

#[test]
fn loop_and_payload_bindings_are_read_only() {
    let c = "class C {\n    var n: Int\n}\n";
    fails_with(&format!("{c}pub fn main() {{\n    for c in [C {{ n: 1 }}] {{\n        c.n = 2\n    }}\n}}\n"), "for", "`c` is read-only");
    let m = format!("{c}enum E {{\n    A(C)\n}}\npub fn main() {{\n    match E.A(C {{ n: 1 }}) {{\n        E.A(c) => {{ c.n = 2 }}\n    }}\n}}\n");
    fails_with(&m, "match", "`c` is read-only");
}

#[test]
fn a_struct_holds_only_values_that_cannot_change() {
    fails_with("struct S {\n    xs: List<Int>\n}\npub fn main() { }\n", "list", "struct `S` field `xs` is 'List<Int>', which can change; make `S` a class");
    fails_with("class M {\n    var a: Int\n}\nstruct S {\n    m: M?\n}\npub fn main() { }\n", "mutable_class", "struct `S` field `m` is");
    fails_with("struct S {\n    b: Bytes\n}\npub fn main() { }\n", "bytes", "which can change");
}

#[test]
fn a_let_struct_is_deep_through_nested_structs_and_lambdas() {
    let s = "struct P {\n    var x: Int\n}\nstruct O {\n    var inner: P\n}\n";
    fails_with(&format!("{s}pub fn main() {{\n    let o = O {{ inner: P {{ x: 1 }} }}\n    o.inner.x = 2\n}}\n"), "let_nested", "`o` is a `let` struct; declare it `var o`");
    fails_with(&format!("{s}pub fn main() {{\n    let p = P {{ x: 1 }}\n    let f = || {{ p.x = 2 }}\n    f()\n}}\n"), "let_captured", "`p` is a `let` struct; declare it `var p`");
}
