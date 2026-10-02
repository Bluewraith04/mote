//! Literals typed from every entry, trailing commas, duplicate keys, ordered maps, quoted strings.

use std::process::Command;

fn mote(verb: &str, source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_literal_{}_{}_{}", verb, tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg(verb).arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn runs(body: &str, tag: &str) -> String {
    let src = format!("import {{ Any }} from std.experimental.types\nclass Box {{\n    items: List<Any>\n}}\nfn takes(xs: List<Any>) -> Int {{ return xs.len() }}\npub fn main() {{\n{body}\n}}\n");
    let (ok, text) = mote("run", &src, tag);
    assert!(ok, "{text}");
    text
}

fn rejected(body: &str, tag: &str, message: &str) {
    let (ok, text) = mote("check", &format!("pub fn main() {{\n{body}\n}}\n"), tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

#[test]
fn maps_keep_insertion_order() {
    let body = r#"    let m: Map<String, Int> = {"zeta": 1, "alpha": 2, "mid": 3}
    m["alpha"] = 20
    m["new"] = 4
    println(m.keys())
    m.remove("zeta")
    m["zeta"] = 9
    println(m)
    let big: Map<Int, Int> = Map()
    var i = 0
    while i < 50 {
        big[50 - i] = i
        i += 1
    }
    println(big.keys()[0])
    println(big.keys()[49])"#;
    assert_eq!(
        runs(body, "order"),
        "[\"zeta\", \"alpha\", \"mid\", \"new\"]\n{\"alpha\": 20, \"mid\": 3, \"new\": 4, \"zeta\": 9}\n50\n1\n"
    );
}

#[test]
fn strings_are_quoted_inside_collections_only() {
    let body = r#"    println(["a", "b"])
    println({'c': "q\"t"})
    println("top")"#;
    assert_eq!(runs(body, "quote"), "[\"a\", \"b\"]\n{'c': \"q\\\"t\"}\ntop\n");
}

#[test]
fn trailing_commas() {
    let body = "    let xs = [1, 2,]\n    let m = {\n        \"a\": 1,\n        \"b\": 2,\n    }\n    println(xs.len() + m.len())";
    assert_eq!(runs(body, "comma"), "4\n");
}

#[test]
fn context_decides_a_mixed_literal() {
    let body = r#"    let xs: List<Any> = [1, "a"]
    let m: Map<String, Any> = {"a": 1, "b": "x"}
    var ys: List<Any> = []
    ys = [true, 2]
    let b = Box { items: [1, "z"] }
    let nested: List<List<Any>> = [[1, "a"], [true]]
    println(takes([1, "a"]) + xs.len() + m.len() + ys.len() + b.items.len() + nested.len())"#;
    assert_eq!(runs(body, "context"), "12\n");
}

#[test]
fn a_common_type_is_inferred() {
    let body = "    let xs = [1, null]\n    let y: Int? = xs[1]\n    println(y)\n    let n = [[], [1]]\n    let k: List<List<Int>> = n\n    println(k)";
    assert_eq!(runs(body, "join"), "null\n[[], [1]]\n");
}

#[test]
fn a_mix_without_context_is_an_error() {
    rejected("    let xs = [1, \"a\"]", "list", "list elements have different types, `Int` and `String`");
    rejected("    let m = {\"a\": 1, \"b\": \"x\"}", "values", "map values have different types, `Int` and `String`");
    rejected("    let m = {1: 0, \"k\": 0}", "keys", "map keys have different types");
}

#[test]
fn a_literal_key_written_twice_is_an_error() {
    rejected("    let m = {\"a\": 1, \"a\": 2}", "dup_str", "key \"a\" appears twice");
    rejected("    let m = {1: \"x\", 2: \"y\", 1: \"z\"}", "dup_int", "key 1 appears twice");
}
