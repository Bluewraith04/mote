//! Function types — lambda and named-function types, checked calls, context typing, Send.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_fn_type_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn rejected(source: &str, tag: &str, want: &str) {
    let (ok, text) = run(source, tag);
    assert!(!ok && text.contains(want), "expected `{want}`, got: {text}");
}

const TWICE: &str = "fn twice(f: (Int) -> Int, x: Int) -> Int {\n    return f(f(x))\n}\n";

#[test]
fn function_values_and_lambdas_run() {
    let src = format!(
        "{TWICE}fn inc(n: Int) -> Int {{\n    return n + 1\n}}\nfn apply<T, U>(x: T, f: (T) -> U) -> U {{\n    return f(x)\n}}\nfn make_adder(k: Int) -> (Int) -> Int {{\n    return |x| x + k\n}}\nfn main() {{\n    println(twice(inc, 1))\n    println(twice(|n| n * 3, 2))\n    let s: String = apply(4, |n| \"n=${{n}}\")\n    println(s)\n    let add: (Int, Int) -> Int = |a, b| a + b\n    println(add(2, 3))\n    println(make_adder(3)(4))\n}}\n"
    );
    let (ok, text) = run(&src, "run");
    assert!(ok && text == "3\n18\nn=4\n5\n7\n", "{text}");
}

#[test]
fn a_callback_of_the_wrong_type_is_an_error() {
    rejected(&format!("{TWICE}fn main() {{\n    println(twice(|s: String| s, 1))\n}}\n"), "wrong_cb", "expects '(Int) -> Int', found 'Send (String) -> String'");
}

#[test]
fn a_call_through_a_function_value_is_checked() {
    let head = "fn main() {\n    let add: (Int, Int) -> Int = |a, b| a + b\n";
    rejected(&format!("{head}    add(1)\n}}\n"), "arity", "`add` takes 2 argument(s), but 1 were given");
    rejected(&format!("{head}    add(\"x\", 2)\n}}\n"), "arg", "`add` argument 1 expects 'Int', found 'String'");
    rejected(&format!("{head}    let s: String = add(1, 2)\n}}\n"), "ret", "of type 'String' with expression of type 'Int'");
}

#[test]
fn a_lambda_parameter_takes_its_type_from_the_call() {
    rejected(&format!("{TWICE}fn main() {{\n    println(twice(|n| n.len(), 2))\n}}\n"), "ctx", "no method `.len()` on `Int`");
    let src = "class Box {\n    var n: Int\n    pub fn apply(var self, f: (Int) -> Int) {\n        self.n = f(self.n)\n    }\n}\nfn main() {\n    var b = Box { n: 2 }\n    b.apply(|v| v * 10)\n    println(b.n)\n}\n";
    let (ok, text) = run(src, "method");
    assert!(ok && text == "20\n", "{text}");
}

#[test]
fn option_combinators_keep_their_types() {
    let src = "fn pos(x: Int) -> Option<Int> {\n    if x > 0 {\n        return Some(x)\n    }\n    return None\n}\nfn main() {\n    let v: Int = Some(3).map(|x| x * 2).unwrap()\n    println(v)\n    println(Some(5).and_then(|x| pos(x)).unwrap())\n}\n";
    let (ok, text) = run(src, "option");
    assert!(ok && text == "6\n5\n", "{text}");
    rejected("fn main() {\n    let t: String = Some(3).map(|x| x * 2).unwrap()\n}\n", "option_bad", "of type 'String' with expression of type 'Int'");
}

#[test]
fn only_a_capture_free_or_immutable_lambda_crosses_into_spawn() {
    let ok_src = "fn main() {\n    let k = 5\n    let pure = || k + 1\n    scope {\n        spawn {\n            println(pure())\n        }\n    }\n}\n";
    let (ok, text) = run(ok_src, "send_ok");
    assert!(ok && text == "6\n", "{text}");
    let want = "a function is Sendable only when";
    rejected("fn main() {\n    var n = 0\n    let bump = || { n += 1 }\n    scope {\n        spawn {\n            bump()\n        }\n    }\n}\n", "send_var", want);
    rejected("fn run(f: () -> Int) {\n    scope {\n        spawn {\n            println(f())\n        }\n    }\n}\nfn main() {\n}\n", "send_param", want);
}

#[test]
fn std_callbacks_keep_element_types() {
    let src = "import std.iter as it\nimport std.stream as st\nimport { map_values } from std.collections\nimport { Suite } from std.test\n\nstruct Job {\n    id: Int\n    weight: Int\n}\n\nfn main() {\n    let xs = [3, 1, 2]\n    let doubled: List<Int> = it.map(xs, |x| x * 2)\n    println(doubled)\n    let words: List<String> = it.map(xs, |x| \"n${x}\")\n    println(words)\n    println(it.fold(xs, 0, |acc, x| acc + x))\n    println(it.sort_by(xs, |a, b| a < b))\n    let jobs = [Job { id: 1, weight: 5 }, Job { id: 2, weight: 3 }]\n    println(it.find(jobs, |j| j.weight > 4).unwrap().id)\n    let m = map_values({\"a\": 1}, |v| v + 10)\n    let v: Int = m.get(\"a\")\n    println(v)\n    for c in st.map(st.chars(\"ab\"), |c| c + \"!\") {\n        println(c)\n    }\n    var t = Suite.new()\n    t.test(\"typed\", |var s| { s.assert_eq(it.sum(doubled), 12) })\n    t.finish()\n}\n";
    let (ok, text) = run(src, "std");
    assert!(ok && text.starts_with("[6, 2, 4]\n[\"n3\", \"n1\", \"n2\"]\n6\n[1, 2, 3]\n1\n11\na!\nb!\n  ok    typed\n"), "{text}");
    rejected("import std.iter as it\nfn main() {\n    let ys: List<String> = it.map([1, 2], |x| x * 2)\n}\n", "std_ret", "`map` argument 2 expects '(Int) -> String', found 'Send (Int) -> Int'");
    rejected("import std.iter as it\nfn main() {\n    let z = it.filter([1, 2], |x| x + 1)\n}\n", "std_pred", "`filter` argument 2 expects '(Int) -> Bool', found 'Send (Int) -> Int'");
}

#[test]
fn a_send_function_type_crosses_into_spawn() {
    let head = "fn inc(n: Int) -> Int {\n    return n + 1\n}\nfn in_task(f: Send (Int) -> Int) {\n    scope {\n        spawn {\n            println(f(41))\n        }\n    }\n}\nfn main() {\n";
    let (ok, text) = run(&format!("{head}    in_task(inc)\n    let k = 2\n    in_task(|x| x * k)\n}}\n"), "send_marker");
    assert!(ok && text == "42\n82\n", "{text}");
    rejected(&format!("{head}    var n = 0\n    in_task(|x| x + n)\n}}\n"), "send_marker_var", "expects 'Send (Int) -> Int', found '(Int) -> Int'");
    rejected("fn f(g: Send (Int)) {\n}\nfn main() {\n}\n", "send_marker_bad", "`Send` marks a function type");
}
