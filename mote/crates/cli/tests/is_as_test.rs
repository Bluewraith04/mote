//! `e is T`; `as` only renames, and the checks that replace a checking `as`.

use std::process::Command;

fn mote(verb: &str, source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_is_as_{}_{}_{}", verb, tag, std::process::id()));
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
fn is_tests_the_whole_runtime_type() {
    let src = r#"import { Any } from std.experimental.types
enum Shape {
    Circle(Int)
    Square(Int)
}

fn holds<T>(x: T, v: Any) -> Bool {
    return v is List<T>
}

pub fn main() {
    let a: Any = [1, 2]
    let seen = [
        a is List<Int>, a is List<String>, a is List<Any>, 3 is Int?, null is Int?, null is Int,
        Shape.Circle(1) is Shape, (1, "s") is (Int, String), holds(1, a), holds("s", a),
    ]
    println(seen)
}
"#;
    let (ok, text) = mote("run", src, "is");
    assert!(ok && text == "[true, false, false, true, true, false, true, true, true, false]\n", "{text}");
}

#[test]
fn as_in_an_expression_is_rejected() {
    rejected(
        "import { Any } from std.experimental.types\npub fn main() {\n    let a: Any = 1\n    println(a as Int)\n}\n",
        "as",
        "`as` only renames an import. To convert, write `T.from(e)`. To check a type, use `is`, a `match` type pattern, or a typed `let`",
    );
}

#[test]
fn the_replacements_for_a_checking_as_work() {
    let src = r#"import { Any } from std.experimental.types
struct P {
    x: Int
}

pub fn main() {
    let a: Any = [1, 2]
    let xs: List<Int> = a
    let n: Int? = 4
    let p = P { x: 1 }
    let e: List<String> = []
    let v: Any = 5
    println(xs.get(1) + n! + p.x)
    println(typeof(e))
    println(v)
}
"#;
    let (ok, text) = mote("run", src, "replaced");
    assert!(ok && text == "7\nList<String>\n5\n", "{text}");
}

#[test]
fn a_failed_check_out_of_any_is_a_located_fault() {
    let (ok, text) = mote("run", "import { Any } from std.experimental.types\npub fn main() {\n    let a: Any = \"s\"\n    let n: Int = a\n    println(n + 1)\n}\n", "any");
    assert!(!ok && text.contains("expected `Int`, found `String`") && text.contains("main.mote:4"), "{text}");
}

#[test]
fn an_enum_variant_is_not_a_type() {
    let (ok, text) = mote(
        "check",
        "enum Shape {\n    Circle(Int)\n    Square(Int)\n}\n\npub fn main() {\n    println(Shape.Circle(1) is Shape.Circle)\n}\n",
        "variant",
    );
    assert!(!ok && text.contains("`Shape.Circle` is an enum variant, not a type; test for it with `match`"), "{text}");
}
