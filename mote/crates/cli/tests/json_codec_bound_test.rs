//! `T.from_json` through a `JsonCodec` bound.

use std::process::Command;

const HEAD: &str = "import std.json as json\nimport { JsonCodec, Json } from std.json\n\nstruct Item: (JsonCodec) {\n    name: String\n    qty: Int\n}\n\nfn show<T: JsonCodec>(v: T) -> String {\n    return json.to_text(json.encode(v)).unwrap()\n}\n\nfn back<T: JsonCodec>(text: String) -> Result<T, Error> {\n    let j = json.parse(text)?\n    return json.decode(j)\n}\n\n";

fn go(verb: &str, source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_jsonbound_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), format!("{HEAD}{source}")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg(verb).arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn rejected(source: &str, tag: &str, message: &str) {
    let (ok, text) = go("check", source, tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

#[test]
fn builtin_shapes_write_themselves_as_json() {
    let src = "fn main() {\n    println(show(Item { name: \"a\", qty: 1 }))\n    println(show([Item { name: \"a\", qty: 1 }]))\n    println(show(3))\n    println(show(\"hi\"))\n    let m: Map<String, List<Int>> = {\"x\": [1], \"y\": [2, 3]}\n    println(show(m))\n    let o: Item? = null\n    println(show(o))\n}\n";
    let (ok, text) = go("run", src, "encode");
    assert!(ok && text == "{\"name\":\"a\",\"qty\":1}\n[{\"name\":\"a\",\"qty\":1}]\n3\n\"hi\"\n{\"x\":[1],\"y\":[2,3]}\nnull\n", "{text}");
}

#[test]
fn builtin_shapes_read_themselves_from_json() {
    let src = "fn main() {\n    let xs: Result<List<Item>, Error> = back(\"[{\\\"name\\\":\\\"a\\\",\\\"qty\\\":1},{\\\"name\\\":\\\"b\\\",\\\"qty\\\":2}]\")\n    println(xs.unwrap().get(1).name)\n    let n: Result<Int, Error> = back(\"7\")\n    println(n.unwrap())\n    let mm: Result<Map<String, Int>, Error> = back(\"{\\\"a\\\":1,\\\"b\\\":2}\")\n    println(mm.unwrap().get(\"b\"))\n    let opt: Result<Item?, Error> = back(\"null\")\n    println(opt.unwrap() == null)\n}\n";
    let (ok, text) = go("run", src, "decode");
    assert!(ok && text == "b\n7\n2\ntrue\n", "{text}");
}

#[test]
fn an_annotation_reaches_the_call_under_question_mark() {
    let src = "fn run() -> Result<Null, Error> {\n    let j = json.parse(\"{\\\"name\\\":\\\"x\\\",\\\"qty\\\":1}\")?\n    let item: Item = json.decode(j)?\n    let xs: List<Int> = json.decode(json.parse(\"[1,2]\")?)?\n    println(item.name)\n    println(xs.len())\n    return Ok(null)\n}\n\nfn main() {\n    println(run().is_ok())\n}\n";
    let (ok, text) = go("run", src, "annotated");
    assert!(ok && text == "x\n2\ntrue\n", "{text}");
}

#[test]
fn a_bad_element_names_its_place() {
    let src = "fn main() {\n    let bad: Result<List<Item>, Error> = back(\"[{\\\"name\\\":\\\"a\\\",\\\"qty\\\":1},{\\\"name\\\":\\\"b\\\"}]\")\n    match bad {\n        Ok(_) => { println(\"ok\") }\n        Err(e) => { println(e.message) }\n    }\n}\n";
    let (ok, text) = go("run", src, "badelem");
    assert!(ok && text == "[1]: missing field \"qty\"\n", "{text}");
}

#[test]
fn a_map_needs_string_keys() {
    let src = "fn main() {\n    let m: Map<Int, Int> = {1: 2}\n    println(show(m))\n}\n";
    rejected(src, "intkeys", "only a `Map` with `String` keys is JSON");
}

#[test]
fn a_list_of_a_type_without_the_bound_is_rejected() {
    let src = "class Blob {\n    n: Int\n}\nfn main() {\n    println(show([Blob { n: 1 }]))\n}\n";
    rejected(src, "blob", "cannot be used for `T`");
}

#[test]
fn a_static_call_through_a_bound_needs_a_declared_function() {
    let src = "fn f<T: JsonCodec>(j: Json) -> Result<T, Error> {\n    return T.nothing(j)\n}\nfn main() { }\n";
    rejected(src, "nofn", "nothing");
}
