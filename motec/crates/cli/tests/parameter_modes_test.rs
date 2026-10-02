//! Parameter modes and receivers: read-only parameters, `var` parameters with unmarked arguments, `var self`.

use std::process::Command;

fn mote(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_param_modes_{}_{}", tag, std::process::id()));
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
struct P {
    var x: Int
    var y: Int
    pub fn sum(self) -> Int { return self.x + self.y }
    pub fn move_by(var self, d: Int) { self.x += d }
}
struct Holder { var p: P }
fn bump(var n: Int) { n += 1 }
fn swap(var a: Int, var b: Int) {
    let t = a
    a = b
    b = t
}
fn reset(var p: P) { p = P { x: 0, y: 0 } }
fn shift(var p: P) { p.x += 10 }
fn fill(var xs: List<Int>) { xs.push(9) }
fn twice(var n: Int) -> Int {
    bump(n)
    bump(n)
    return n * 100
}
fn later(var n: Int) {
    let f = || { n += 5 }
    f()
}
fn total(p: P) -> Int { return p.sum() }
fn apply(var xs: List<Int>, g: (var List<Int>) -> Null) { g(xs) }
pub fn main() {
    var c = 1
    bump(c)
    println(c)
    var a = 1
    var b = 2
    swap(a, b)
    println("${a} ${b}")
    var p = P { x: 1, y: 2 }
    p.move_by(3)
    println(total(p))
    shift(p)
    println(p.x)
    reset(p)
    println(p.x)
    var xs = [1]
    fill(xs)
    println(xs)
    var k = 1
    println(twice(k))
    println(k)
    var h = Holder { p: P { x: 5, y: 5 } }
    shift(h.p)
    println(h.p.x)
    reset(h.p)
    println(h.p.x)
    var m = 0
    later(m)
    println(m)
    apply(xs, |ys| { println(ys.len()) })
    apply(xs, |var ys| { ys.push(3) })
    println(xs)
}
"#;

#[test]
fn var_parameters_write_the_callers_place() {
    let (ok, out) = mote(PROGRAM, "program");
    assert!(ok, "{out}");
    assert_eq!(out, "2\n2 1\n6\n14\n0\n[1, 9]\n300\n3\n15\n0\n5\n2\n[1, 9, 3]\n");
}

#[test]
fn a_plain_parameter_is_read_only() {
    fails_with("struct P { var x: Int }\nfn f(p: P) { p.x = 1 }\npub fn main() { }\n", "field", "`p` is a read-only parameter; declare it `var p`");
    fails_with("fn f(xs: List<Int>) { xs.push(1) }\npub fn main() { }\n", "method", "`xs` is a read-only parameter; declare it `var xs`");
    fails_with("fn f(xs: List<Int>) { xs[0] = 1 }\npub fn main() { }\n", "index", "`xs` is a read-only parameter; declare it `var xs`");
    fails_with("fn f(n: Int) { n = 2 }\npub fn main() { }\n", "assign", "`n` is a read-only parameter; declare it `var n`");
    fails_with("fn f(xs: List<Int>) {\n let ys = xs\n ys.push(1)\n}\npub fn main() { }\n", "derived", "`ys` is read-only");
    fails_with("pub fn main() { let f = |xs: List<Int>| { xs.push(1) } }\n", "lambda", "`xs` is read-only");
}

#[test]
fn a_receiver_that_writes_says_var_self() {
    fails_with("struct P { var x: Int\n pub fn f(self) { self.x = 1 } }\npub fn main() { }\n", "self_field", "`self` is read-only; declare the method with `var self`");
    let src = "struct P { var x: Int\n pub fn g(var self) { }\n pub fn f(self) { self.g() } }\npub fn main() { }\n";
    fails_with(src, "self_call", "`self` is read-only; declare the method with `var self`");
    fails_with("struct P { var x: Int\n pub fn f(var self) { self = P { x: 1 } } }\npub fn main() { }\n", "self_assign", "cannot assign to `self`");
}

#[test]
fn function_types_carry_var() {
    let (ok, out) = mote("pub fn main() {\n var xs = [1]\n let f = |var ys: List<Int>| { ys.push(2) }\n f(xs)\n println(xs)\n let g = |var ys: List<Int>| { ys.push(3) }\n let h = [1]\n g(h)\n}\n", "value_call");
    assert!(!ok && out.contains("`h` is a `let`, and this parameter is `var`"), "{out}");
    let src = "fn run(g: (List<Int>) -> Null) { }\npub fn main() { run(|var ys: List<Int>| { ys.push(2) }) }\n";
    fails_with(src, "fits", "expects '(List<Int>) -> Null', found 'Send (var List<Int>) -> Null'");
}

#[test]
fn old_receivers_and_paths_are_parse_errors() {
    fails_with("struct P { x: Int\n pub fn f(&self) { } }\npub fn main() { }\n", "ref_self", "write `self`");
    fails_with("struct P { x: Int\n pub fn f(&mut self) { } }\npub fn main() { }\n", "ref_mut_self", "write `var self`");
    fails_with("struct P { x: Int\n pub fn f(mut self) { } }\npub fn main() { }\n", "mut_self", "write `var self`");
    fails_with("fn f(mut n: Int) { }\npub fn main() { }\n", "mut_param", "write `var n`");
    fails_with("pub fn main() { let f = |mut x: Int| { } }\n", "mut_lambda", "write `x`, not `mut x`");
    let src = "struct P { x: Int\n pub fn o() -> P { return P { x: 1 } } }\npub fn main() { let p = P::o() }\n";
    fails_with(src, "path", "write `P.o`, not `::`");
}

#[test]
fn a_var_argument_needs_no_mark() {
    let (ok, out) = mote("fn b(var n: Int) { n += 1 }\nfn l(var xs: List<Int>) { xs.push(9) }\npub fn main() {\n var c = 1\n b(c)\n var xs = [1]\n l(xs)\n l([5])\n b(1 + 2)\n println(c)\n println(xs)\n}\n", "unmarked");
    assert!(ok && out == "2\n[1, 9]\n", "{out}");
    fails_with("fn b(var n: Int) { }\npub fn main() {\n var c = 1\n b(var c)\n}\n", "marked", "an argument takes no `var`; remove it");
    fails_with("fn b(var n: Int) { }\npub fn main() {\n let c = 1\n b(c)\n}\n", "let", "`c` is a `let`, and this parameter is `var`; declare it `var c`");
    fails_with("fn b(var n: Int) { }\nfn c(n: Int) { b(n) }\npub fn main() { }\n", "read_only", "`n` is a read-only parameter; declare it `var n`");
    fails_with("fn b(var n: Int = 3) { }\npub fn main() { }\n", "default", "`var n` cannot have a default");
    fails_with("fn b(var n: Int) { }\npub fn main() { let f = b }\n", "value", "`b` has a `var` parameter, so it can only be called directly");
}

#[test]
fn a_var_lambda_changes_a_struct_argument_in_place() {
    let src = "struct Counter { var n: Int }\nfn main() {\n    var c = Counter { n: 0 }\n    let f = |var r: Counter, q: Int| { r.n += q }\n    f(c, 5)\n    println(c.n.to_string())\n}\n";
    let (ok, text) = mote(src, "var_lambda_struct");
    assert!(ok && text == "5\n", "{text}");
}
