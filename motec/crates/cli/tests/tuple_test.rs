//! Tuples — anonymous structural tuples, `()`, destructuring, and tuple match patterns.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_tuple_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn check(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_tuple_chk_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("check").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn rejected(source: &str, tag: &str, message: &str) {
    let (ok, text) = check(source, tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

#[test]
fn literal_construction_and_field_access() {
    let src = "pub fn main() {\n    let p = (1, 2)\n    println(p.0.to_string())\n    println(p.1.to_string())\n}\n";
    let (ok, text) = run(src, "field_access");
    assert!(ok && text == "1\n2\n", "{text}");
}

#[test]
fn empty_tuple_prints_as_parens() {
    let src = "pub fn main() {\n    let u = ()\n    println(u)\n}\n";
    let (ok, text) = run(src, "empty");
    assert!(ok && text == "()\n", "{text}");
}

#[test]
fn printing_renders_positionally() {
    let src = "pub fn main() {\n    println((1, \"two\", true))\n}\n";
    let (ok, text) = run(src, "print");
    assert!(ok && text == "(1, \"two\", true)\n", "{text}");
}

#[test]
fn let_destructuring() {
    let src = "pub fn main() {\n    let (a, b) = (10, 32)\n    println((a + b).to_string())\n}\n";
    let (ok, text) = run(src, "destructure");
    assert!(ok && text == "42\n", "{text}");
}

#[test]
fn destructuring_a_function_result() {
    let src = "fn min_max(a: Int, b: Int) -> (Int, Int) {\n    if a < b { return (a, b) }\n    return (b, a)\n}\npub fn main() {\n    let (lo, hi) = min_max(9, 3)\n    println(lo.to_string())\n    println(hi.to_string())\n}\n";
    let (ok, text) = run(src, "minmax");
    assert!(ok && text == "3\n9\n", "{text}");
}

#[test]
fn match_tuple_pattern_with_literal_and_bindings() {
    let src = "fn classify(p: (Int, String)) -> String {\n    match p {\n        (0, y) => { return \"zero:\" + y }\n        (x, y) => { return \"other:\" + x.to_string() + \":\" + y }\n    }\n}\npub fn main() {\n    println(classify((0, \"a\")))\n    println(classify((5, \"b\")))\n}\n";
    let (ok, text) = run(src, "match_lit");
    assert!(ok && text == "zero:a\nother:5:b\n", "{text}");
}

#[test]
fn match_tuple_pattern_all_bindings_needs_no_wildcard_arm() {
    let src = "pub fn main() {\n    let p = (1, 2)\n    match p {\n        (x, y) => { println((x + y).to_string()) }\n    }\n}\n";
    let (ok, text) = run(src, "match_irrefutable");
    assert!(ok && text == "3\n", "{text}");
}

#[test]
fn structural_equality_across_declaration_sites() {
    let src = "fn first(p: (Int, Int)) -> Int {\n    return p.0\n}\npub fn main() {\n    let a = (1, 2)\n    println(first(a).to_string())\n    println(first((3, 4)).to_string())\n}\n";
    let (ok, text) = run(src, "structural");
    assert!(ok && text == "1\n3\n", "{text}");
}

#[test]
fn nested_tuple_via_intermediate_binding() {
    let src = "pub fn main() {\n    let t = (1, (2, 3))\n    let inner = t.1\n    println(inner.0.to_string())\n}\n";
    let (ok, text) = run(src, "nested");
    assert!(ok && text == "2\n", "{text}");
}

#[test]
fn tuple_field_out_of_range_is_a_checker_error() {
    rejected(
        "pub fn main() {\n    let p = (1, 2)\n    println(p.5.to_string())\n}\n",
        "range",
        "no field '.5'",
    );
}

#[test]
fn destructuring_arity_mismatch_is_a_checker_error() {
    rejected(
        "pub fn main() {\n    let (a, b, c) = (1, 2)\n    println(a.to_string())\n}\n",
        "arity",
        "expected 2",
    );
}

#[test]
fn destructuring_at_top_level_is_a_checker_error() {
    rejected(
        "let (a, b) = (1, 2)\npub fn main() {\n    println(a.to_string())\n}\n",
        "top_level",
        "only supported inside a function body",
    );
}

#[test]
fn a_struct_holding_a_tuple_that_can_change_is_an_error() {
    rejected(
        "struct Bad {\n    pair: (Int, List<Int>)\n}\npub fn main() {\n    println(1)\n}\n",
        "struct_field",
        "struct `Bad` field `pair` is '(Int, List<Int>)', which can change",
    );
}

#[test]
fn match_tuple_pattern_shape_mismatch_is_a_checker_error() {
    rejected(
        "pub fn main() {\n    let n = 1\n    match n {\n        (a, b) => { println(a.to_string()) }\n        _ => {}\n    }\n}\n",
        "shape_mismatch",
        "cannot match a tuple pattern against",
    );
}

#[test]
fn chained_tuple_indices_on_one_line() {
    let src = "pub fn main() {\n    let t = ((1, 2), (3, (4, 5)))\n    println(t.1.0)\n    println(t.1.1.0)\n    println(1.5 + 0.5)\n}\n";
    let (ok, text) = run(src, "chained");
    assert!(ok && text == "3\n4\n2.0\n", "{text}");
}
