//! `c ? x : y`.

use std::process::Command;

fn mote(verb: &str, source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_ternary_{}_{}_{}", verb, tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg(verb).arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn rejected(src: &str, tag: &str, message: &str) {
    let (ok, text) = mote("check", src, tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

#[test]
fn only_the_chosen_branch_runs() {
    let src = r#"fn side(tag: String, v: Int) -> Int {
    println(tag)
    return v
}

pub fn main() {
    let a = true ? side("then", 1) : side("else", 2)
    let b = false ? side("then", 1) : side("else", 2)
    println(a + b)
}
"#;
    let (ok, text) = mote("run", src, "branch");
    assert!(ok && text == "then\nelse\n3\n", "{text}");
}

#[test]
fn nests_to_the_right_and_works_anywhere_an_expression_does() {
    let src = r#"fn sign(n: Int) -> String {
    return n > 0 ? "pos" : n < 0 ? "neg" : "zero"
}

pub fn main() {
    println(sign(3) + sign(-3) + sign(0))
    let xs = [1 > 0 ? 10 : 20, 30]
    println(xs)
    var s = 0
    s += xs[0] == 10 ? 5 : 6
    println(s)
}
"#;
    let (ok, text) = mote("run", src, "nest");
    assert!(ok && text == "posnegzero\n[10, 30]\n5\n", "{text}");
}

#[test]
fn a_null_branch_makes_the_result_optional() {
    let src = r#"fn half(n: Int) -> Int? {
    return n % 2 == 0 ? n / 2 : null
}

pub fn main() {
    let n = 4 > 5 ? 4 : null
    println(n)
    println(half(8))
}
"#;
    let (ok, text) = mote("run", src, "null");
    assert!(ok && text == "null\n4\n", "{text}");
}

#[test]
fn unrelated_branches_need_an_annotation() {
    rejected(
        "pub fn main() {\n    let x = 1 > 0 ? 1 : \"one\"\n    println(x)\n}\n",
        "mixed",
        "the branches of `?:` have different types, `Int` and `String`",
    );
    let (ok, text) = mote("run", "import { Any } from std.experimental.types\npub fn main() {\n    let x: Any = 1 > 0 ? 1 : \"one\"\n    println(x)\n}\n", "any");
    assert!(ok && text == "1\n", "{text}");
}

#[test]
fn a_branch_that_does_not_fit_the_context_is_named() {
    rejected(
        "fn pick(c: Bool) -> String {\n    return c ? 1 : \"a\"\n}\n\npub fn main() {\n    println(pick(true))\n}\n",
        "context",
        "this `?:` branch is `Int`, but `String` is expected",
    );
}

#[test]
fn the_condition_must_be_bool() {
    rejected(
        "pub fn main() {\n    let a = 1\n    println(a ? 1 : 2)\n}\n",
        "cond",
        "the condition of `?:` must be `Bool`, found `Int`",
    );
}
