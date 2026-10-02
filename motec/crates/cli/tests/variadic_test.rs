//! Variadic parameters — `...xs: T` packs trailing call arguments into a `List<T>`.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_variadic_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn check(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_variadic_chk_{}_{}", tag, std::process::id()));
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
fn zero_one_and_many_trailing_args() {
    let src = "fn total(...xs: Int) -> Int {\n    var sum = 0\n    for x in xs { sum = sum + x }\n    return sum\n}\npub fn main() {\n    println(total().to_string())\n    println(total(5).to_string())\n    println(total(1, 2, 3, 4).to_string())\n}\n";
    let (ok, text) = run(src, "sum");
    assert!(ok && text == "0\n5\n10\n", "{text}");
}

#[test]
fn fixed_params_before_the_variadic_tail() {
    let src = "fn join_all(sep: String, ...parts: String) -> String {\n    var out = \"\"\n    var first = true\n    for p in parts {\n        if first { out = p; first = false } else { out = out + sep + p }\n    }\n    return out\n}\npub fn main() {\n    println(join_all(\",\", \"a\", \"b\", \"c\"))\n}\n";
    let (ok, text) = run(src, "join");
    assert!(ok && text == "a,b,c\n", "{text}");
}

#[test]
fn untyped_variadic_defaults_to_any() {
    let src = "fn count(...xs) -> Int {\n    var n = 0\n    for x in xs { n = n + 1 }\n    return n\n}\npub fn main() {\n    println(count(1, \"two\", 3.0, true).to_string())\n}\n";
    let (ok, text) = run(src, "any_count");
    assert!(ok && text == "4\n", "{text}");
}

#[test]
fn instance_method_variadic() {
    let src = "class Bag {\n    fn total(self, ...xs: Int) -> Int {\n        var sum = 0\n        for x in xs { sum = sum + x }\n        return sum\n    }\n}\npub fn main() {\n    let b = Bag {}\n    println(b.total(1, 2, 3).to_string())\n}\n";
    let (ok, text) = run(src, "instance");
    assert!(ok && text == "6\n", "{text}");
}

#[test]
fn static_method_variadic() {
    let src = "class Bag {\n    fn total(...xs: Int) -> Int {\n        var sum = 0\n        for x in xs { sum = sum + x }\n        return sum\n    }\n}\npub fn main() {\n    println(Bag.total(1, 2, 3).to_string())\n}\n";
    let (ok, text) = run(src, "static");
    assert!(ok && text == "6\n", "{text}");
}

#[test]
fn rejects_a_type_mismatched_trailing_argument() {
    rejected(
        "fn total(...xs: Int) -> Int {\n    return 0\n}\npub fn main() {\n    total(1, \"two\")\n}\n",
        "type_mismatch",
        "argument 2 expects 'Int'",
    );
}

#[test]
fn rejects_a_non_trailing_variadic() {
    rejected(
        "fn f(...xs: Int, y: Int) -> Int {\n    return 0\n}\n",
        "not_trailing",
        "must be the last parameter",
    );
}

#[test]
fn rejects_mixing_defaults_and_variadic() {
    rejected(
        "fn f(a: Int = 1, ...xs: Int) -> Int {\n    return 0\n}\n",
        "mixed",
        "cannot follow a default parameter",
    );
}

#[test]
fn rejects_a_default_on_the_variadic_itself() {
    let (ok, text) = check("fn f(...xs: Int = 1) -> Int {\n    return 0\n}\n", "self_default");
    assert!(!ok && text.contains("cannot have a default value"), "{text}");
}
