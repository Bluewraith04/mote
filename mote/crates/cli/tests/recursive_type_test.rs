//! A type can name itself or a type declared later in its fields and payloads.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_recursive_type_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn self_and_forward_references_type_check_and_run() {
    let src = "enum J {\n    Num(Int)\n    Arr(List<J>)\n    Box(Later)\n}\nclass Later {\n    n: Int\n}\nclass Node {\n    v: Int\n    next: Node?\n}\nclass Tree<T> {\n    value: T\n    kids: List<Tree<T>>\n}\nfn total(j: J) -> Int {\n    match j {\n        J.Num(n) => { return n }\n        J.Box(l) => { return l.n }\n        J.Arr(xs) => {\n            var s = 0\n            for x in xs {\n                s += total(x)\n            }\n            return s\n        }\n    }\n}\nfn sum(t: Tree<Int>) -> Int {\n    var s = t.value\n    for k in t.kids {\n        s += sum(k)\n    }\n    return s\n}\nfn main() {\n    println(total(J.Arr([J.Num(1), J.Arr([J.Num(2), J.Box(Later { n: 3 })])])))\n    let list = Node { v: 1, next: Node { v: 2, next: null } }\n    let second = list.next\n    if second != null {\n        println(second.v)\n    }\n    let leaf = Tree { value: 2, kids: [] }\n    println(sum(Tree { value: 1, kids: [leaf, leaf] }))\n}\n";
    let (ok, text) = run(src, "ok");
    assert!(ok && text == "6\n2\n5\n", "{text}");
}

#[test]
fn a_struct_that_contains_itself_is_an_error() {
    let (ok, text) = run("struct S {\n    x: Int\n    s: S?\n}\nfn main() {\n}\n", "self_struct");
    assert!(!ok && text.contains("struct `S` contains itself through field `s`"), "{text}");
    let (ok, text) = run("struct A {\n    b: B\n}\nstruct B {\n    names: List<String>\n}\nfn main() {\n}\n", "later_heap");
    assert!(!ok && text.contains("struct `A` field `b` is 'B', which can change"), "{text}");
}

#[test]
fn json_payloads_are_typed() {
    let src = "import std.data.json as json\nimport { Json } from std.data.json\nfn count(j: Json) -> Int {\n    match j {\n        Json.Array(xs) => {\n            var n = 0\n            for x in xs {\n                n += count(x)\n            }\n            return n\n        }\n        Json.Object(ms) => { return ms.len() }\n        _ => { return 1 }\n    }\n}\nfn main() {\n    println(count(json.parse(\"[1, [2, 3], {\\\"a\\\": 1, \\\"b\\\": 2}]\").unwrap()))\n}\n";
    let (ok, text) = run(src, "json");
    assert!(ok && text == "5\n", "{text}");
    let (ok, text) = run("import { Json } from std.data.json\nfn main() {\n    let bad = Json.Array([1, 2])\n}\n", "json_bad");
    assert!(!ok && text.contains("expects 'List<Json>', found 'List<Int>'"), "{text}");
}
