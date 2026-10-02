//! `==` compares lists, tuples, enum variants, maps and sets by content; classes stay identity.

use std::process::Command;

fn run(source: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_structural_eq_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn lists_and_tuples_compare_by_content() {
    let src = "fn main() {\n    println([1, 2] == [1, 2])\n    println([1, 2] == [1, 3])\n    println([1, 2] != [1])\n    println((1, \"a\") == (1, \"a\"))\n    println(((1, 2), [3]) == ((1, 2), [3]))\n    println([[1], [2]] == [[1], [2]])\n    let e: List<Int> = []\n    println(e == [])\n}\n";
    assert_eq!(run(src, "content"), "true\nfalse\ntrue\ntrue\ntrue\ntrue\ntrue\n");
}

#[test]
fn contains_uses_the_same_equality() {
    assert_eq!(run("fn main() {\n    println([(1, 2)].contains((1, 2)))\n    println([[1]].contains([1]))\n}\n", "contains"), "true\ntrue\n");
}

#[test]
fn classes_still_compare_by_identity() {
    let src = "class C {\n    n: Int\n}\nfn main() {\n    let a = C { n: 1 }\n    println(a == C { n: 1 })\n    println(a == a)\n}\n";
    assert_eq!(run(src, "class"), "false\ntrue\n");
}

#[test]
fn a_list_that_holds_itself_does_not_hang() {
    let src = "import { Any } from std.experimental.types\nfn main() {\n    var a: List<Any> = []\n    a.push(a)\n    var b: List<Any> = []\n    b.push(b)\n    println(a == a)\n    println(a == b)\n}\n";
    assert_eq!(run(src, "cycle"), "true\nfalse\n");
}

#[test]
fn enum_variants_maps_and_sets_compare_by_content() {
    let src = "import { set_from } from std.collections\nenum K {\n    A\n    B(Int)\n}\nfn main() {\n    println(K.A == K.A)\n    println(K.B(1) == K.B(1))\n    println(K.B(1) == K.B(2))\n    println(K.A == K.B(1))\n    println(Some([1, 2]) == Some([1, 2]))\n    println({\"a\": 1, \"b\": 2} == {\"b\": 2, \"a\": 1})\n    println({\"a\": 1} == {\"a\": 2})\n    println({\"a\": [1]} == {\"a\": [1]})\n    println(set_from([1, 2]) == set_from([2, 1]))\n    println(set_from([1, 2]) == set_from([1, 2, 3]))\n}\n";
    assert_eq!(run(src, "enum_map_set"), "true\ntrue\nfalse\nfalse\ntrue\ntrue\nfalse\ntrue\ntrue\nfalse\n");
}

#[test]
fn tuple_and_variant_keys_match_by_content() {
    let src = "import { set_from } from std.collections\nenum K {\n    A\n    B(Int)\n}\nfn main() {\n    var m: Map<(Int, Int), String> = {(1, 2): \"a\"}\n    println(m.get((1, 2)))\n    m.set((1, 2), \"b\")\n    println(m.len())\n    let e = {K.B(1): 1, K.A: 2}\n    println(e.get(K.B(1)))\n    println(e.contains_key(K.A))\n    println(set_from([(1, \"x\"), (1, \"x\"), (2, \"y\")]).len())\n}\n";
    assert_eq!(run(src, "keys"), "a\n1\n1\ntrue\n2\n");
}

#[test]
fn a_list_in_a_key_type_is_an_error() {
    let dir = std::env::temp_dir().join(format!("mote_structural_eq_listkey_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), "fn main() {\n    let n = {[1]: 2}\n}\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success() && err.contains("a `List<Int>` can't be part of a Map or Set key"), "{err}");
}
