//! `@derive(Json)` adds `to_json` and a static `from_json`.

use std::process::Command;

fn run_files(files: &[(&str, &str)], command: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_derive_json_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, src) in files {
        std::fs::write(dir.join(name), src).unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg(command).arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn run(source: &str, tag: &str) -> String {
    let (ok, text) = run_files(&[("main.mote", source)], "run", tag);
    assert!(ok, "{text}");
    text
}

fn rejected(source: &str, tag: &str, message: &str) {
    let (ok, text) = run_files(&[("main.mote", source)], "check", tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

fn read_error(decls: &str, ty: &str, text: &str, tag: &str) -> String {
    let src = format!(
        "import {{ parse }} from std.data.json\n{decls}\nfn main() {{\n    match {ty}.from_json(parse(\"{}\").unwrap()) {{\n        Ok(v) => {{ println(\"read\") }}\n        Err(e) => {{ println(e.message) }}\n    }}\n}}\n",
        text.replace('\\', "\\\\").replace('"', "\\\"")
    );
    run(&src, tag).trim().to_string()
}

const ADDRESS: &str = "@derive(Json)\nstruct Address {\n    city: String\n    zip: Int\n}\n";

const RECORDS: &str = include_str!("programs_mote/json_records.mote");

#[test]
fn done_bar_nested_records_round_trip_through_pretty_text() {
    let text = run(RECORDS, "records");
    let tail: Vec<&str> = text.lines().rev().take(3).collect();
    assert_eq!(tail, ["role.Member: expected an int, found a string", "Bo \"Guest\"", "Ann {\"Admin\":{\"level\":2,\"scopes\":[\"read\",\"write\"]}}"], "{text}");
    assert!(text.contains("    \"limits\": {\n      \"rate\": 5,\n      \"ratio\": 0.5\n    },"), "{text}");
}

#[test]
fn a_struct_writes_an_object_and_reads_it_back() {
    let src = format!(
        "import {{ to_text, parse }} from std.data.json\n{ADDRESS}\nfn main() {{\n    let a = Address {{ city: \"Oslo\", zip: 150 }}\n    let text = to_text(a.to_json()).unwrap()\n    println(text)\n    match Address.from_json(parse(text).unwrap()) {{\n        Ok(b) => {{ println(b == a) }}\n        Err(e) => {{ println(e.message) }}\n    }}\n}}\n"
    );
    assert_eq!(run(&src, "struct").trim(), "{\"city\":\"Oslo\",\"zip\":150}\ntrue");
}

#[test]
fn a_class_with_every_field_kind_round_trips() {
    let src = "import { to_text, parse } from std.data.json\n@derive(Json)\nstruct Address {\n    city: String\n    zip: Int\n}\n@derive(Json)\nclass Person {\n    name: String\n    age: Int?\n    score: Float\n    ok: Bool\n    tags: List<String>\n    homes: List<Address>\n    counts: Map<String, Int>\n    pair: (Int, String)\n    var seen: Set<Int>\n}\nfn main() {\n    var p = Person { name: \"Ann\", age: 3, score: 1.5, ok: true, tags: [\"a\", \"b\"], homes: [Address { city: \"x\", zip: 1 }], counts: {\"k\": 2}, pair: (4, \"four\"), seen: Set() }\n    p.seen.add(7)\n    let text = to_text(p.to_json()).unwrap()\n    println(text)\n    match Person.from_json(parse(text).unwrap()) {\n        Ok(q) => {\n            println(q.name)\n            println(q.age)\n            println(q.homes.get(0).city)\n            println(q.counts.get(\"k\"))\n            println(q.pair.1)\n            println(q.seen.contains(7))\n        }\n        Err(e) => { println(e.message) }\n    }\n}\n";
    let want = "{\"name\":\"Ann\",\"age\":3,\"score\":1.5,\"ok\":true,\"tags\":[\"a\",\"b\"],\"homes\":[{\"city\":\"x\",\"zip\":1}],\"counts\":{\"k\":2},\"pair\":[4,\"four\"],\"seen\":[7]}\nAnn\n3\nx\n2\nfour\ntrue";
    assert_eq!(run(src, "class").trim(), want);
}

#[test]
fn an_optional_field_reads_null_and_a_missing_key_as_null() {
    let decls = "@derive(Json)\nstruct Label {\n    text: String\n    note: String?\n}\n";
    let src = format!(
        "import {{ to_text, parse }} from std.data.json\n{decls}\nfn main() {{\n    println(to_text(Label {{ text: \"t\", note: null }}.to_json()).unwrap())\n    for t in [\"{{\\\"text\\\": \\\"a\\\"}}\", \"{{\\\"text\\\": \\\"a\\\", \\\"note\\\": null}}\", \"{{\\\"text\\\": \\\"a\\\", \\\"note\\\": \\\"n\\\"}}\"] {{\n        match Label.from_json(parse(t).unwrap()) {{\n            Ok(l) => {{ println(l.note) }}\n            Err(e) => {{ println(e.message) }}\n        }}\n    }}\n}}\n"
    );
    assert_eq!(run(&src, "optional").trim(), "{\"text\":\"t\",\"note\":null}\nnull\nnull\nn");
}

#[test]
fn numbers_read_by_type() {
    let decls = "@derive(Json)\nstruct N {\n    i: Int\n    f: Float\n}\n";
    assert_eq!(read_error(decls, "N", "{\"i\": 1, \"f\": 2}", "n_ok"), "read");
    assert_eq!(read_error(decls, "N", "{\"i\": 1.0, \"f\": 2}", "n_int"), "i: expected an int, found a float");
    assert_eq!(read_error(decls, "N", "{\"i\": 1, \"f\": \"x\"}", "n_float"), "f: expected a float, found a string");
}

#[test]
fn an_unknown_key_is_ignored() {
    assert_eq!(read_error(ADDRESS, "Address", "{\"city\": \"a\", \"zip\": 1, \"other\": [1]}", "extra"), "read");
}

#[test]
fn an_error_names_the_path() {
    let decls = format!("{ADDRESS}@derive(Json)\nclass Person {{\n    name: String\n    homes: List<Address>\n    by_name: Map<String, Address>\n}}\n");
    assert_eq!(read_error(&decls, "Person", "{}", "p_missing"), "missing field \"name\"");
    assert_eq!(read_error(&decls, "Person", "[]", "p_array"), "expected an object, found an array");
    assert_eq!(
        read_error(&decls, "Person", "{\"name\": \"a\", \"homes\": [{\"city\": \"x\", \"zip\": 1}, {\"city\": \"y\", \"zip\": \"no\"}], \"by_name\": {}}", "p_list"),
        "homes[1].zip: expected an int, found a string"
    );
    assert_eq!(
        read_error(&decls, "Person", "{\"name\": \"a\", \"homes\": [], \"by_name\": {\"k\": {\"city\": \"x\"}}}", "p_map"),
        "by_name.k: missing field \"zip\""
    );
}

const SHAPE: &str = "@derive(Json)\nenum Shape {\n    Dot\n    Circle(Int)\n    Rect { w: Int, h: Int }\n    Pair(Int, String)\n}\n";

#[test]
fn an_enum_is_externally_tagged() {
    let src = format!(
        "import {{ to_text, parse }} from std.data.json\n{SHAPE}\nfn main() {{\n    let all = [Shape.Dot, Shape.Circle(4), Shape.Rect {{ w: 2, h: 3 }}, Shape.Pair(7, \"x\")]\n    for s in all {{\n        let text = to_text(s.to_json()).unwrap()\n        println(text)\n        match Shape.from_json(parse(text).unwrap()) {{\n            Ok(q) => {{ println(q == s) }}\n            Err(e) => {{ println(e.message) }}\n        }}\n    }}\n}}\n"
    );
    let want = "\"Dot\"\ntrue\n{\"Circle\":4}\ntrue\n{\"Rect\":{\"w\":2,\"h\":3}}\ntrue\n{\"Pair\":[7,\"x\"]}\ntrue";
    assert_eq!(run(&src, "enum").trim(), want);
}

#[test]
fn an_enum_refuses_the_wrong_shape() {
    let cases = [
        ("\\\"Cube\\\"", "unknown variant \"Cube\" of Shape"),
        ("{\\\"Cube\\\": 1}", "unknown variant \"Cube\" of Shape"),
        ("{\\\"Rect\\\": {\\\"w\\\": 1}}", "Rect: missing field \"h\""),
        ("{\\\"Pair\\\": [1]}", "Pair: expected an array of 2, found 1"),
        ("{\\\"Pair\\\": [1, 2]}", "Pair[1]: expected a string, found an int"),
        ("{\\\"Circle\\\": \\\"x\\\"}", "Circle: expected an int, found a string"),
        ("[1]", "expected a string or an object, found an array"),
        ("{}", "expected an object with one key, found 0"),
    ];
    for (i, (text, want)) in cases.iter().enumerate() {
        let src = format!(
            "import {{ parse }} from std.data.json\n{SHAPE}\nfn main() {{\n    match Shape.from_json(parse(\"{text}\").unwrap()) {{\n        Ok(v) => {{ println(\"read\") }}\n        Err(e) => {{ println(e.message) }}\n    }}\n}}\n"
        );
        assert_eq!(run(&src, &format!("shape{i}")).trim(), *want);
    }
}

#[test]
fn a_unit_variant_with_a_discriminant_is_written_as_its_name() {
    let src = "import { to_text } from std.data.json\n@derive(Json)\nenum Level {\n    Low = 1\n    High = 5\n}\nfn main() {\n    println(to_text(Level.High.to_json()).unwrap())\n}\n";
    assert_eq!(run(src, "discriminant").trim(), "\"High\"");
}

#[test]
fn a_union_is_read_member_by_member() {
    let decls = "@derive(Json)\nstruct U {\n    v: Int | String | Float\n    w: (Int | String)?\n}\n";
    let src = format!(
        "import {{ to_text, parse }} from std.data.json\n{decls}\nfn main() {{\n    println(to_text(U {{ v: \"hi\", w: 5 }}.to_json()).unwrap())\n    for t in [\"{{\\\"v\\\": 1, \\\"w\\\": null}}\", \"{{\\\"v\\\": 2.5}}\", \"{{\\\"v\\\": \\\"s\\\", \\\"w\\\": \\\"t\\\"}}\", \"{{\\\"v\\\": true}}\"] {{\n        match U.from_json(parse(t).unwrap()) {{\n            Ok(u) => {{ println(u.v) println(u.w) }}\n            Err(e) => {{ println(e.message) }}\n        }}\n    }}\n}}\n"
    );
    let want = "{\"v\":\"hi\",\"w\":5}\n1\nnull\n2.5\nnull\ns\nt\nv: expected an int, a string or a float, found a bool";
    assert_eq!(run(&src, "union").trim(), want);
}

#[test]
fn a_json_field_passes_through() {
    let src = "import { Json, to_text, parse } from std.data.json\n@derive(Json)\nstruct Event {\n    name: String\n    data: Json\n}\nfn main() {\n    let e = Event { name: \"n\", data: Json.Array([Json.Int(1), Json.Null]) }\n    let text = to_text(e.to_json()).unwrap()\n    println(text)\n    match Event.from_json(parse(text).unwrap()) {\n        Ok(q) => { println(to_text(q.data).unwrap()) }\n        Err(x) => { println(x.message) }\n    }\n}\n";
    assert_eq!(run(src, "passthrough").trim(), "{\"name\":\"n\",\"data\":[1,null]}\n[1,null]");
}

#[test]
fn a_type_can_hold_itself() {
    let src = "import { to_text, parse } from std.data.json\n@derive(Json)\nclass Node {\n    value: Int\n    next: Node?\n}\nfn main() {\n    let n = Node { value: 1, next: Node { value: 2, next: null } }\n    let text = to_text(n.to_json()).unwrap()\n    println(text)\n    match Node.from_json(parse(text).unwrap()) {\n        Ok(q) => { println(q.next.unwrap().value) }\n        Err(e) => { println(e.message) }\n    }\n}\n";
    assert_eq!(run(src, "self").trim(), "{\"value\":1,\"next\":{\"value\":2,\"next\":null}}\n2");
}

#[test]
fn an_empty_struct_is_an_empty_object() {
    let src = "import { to_text, parse } from std.data.json\n@derive(Json)\nstruct Empty {\n}\nfn main() {\n    println(to_text(Empty {}.to_json()).unwrap())\n    match Empty.from_json(parse(\"{}\").unwrap()) {\n        Ok(e) => { println(\"read\") }\n        Err(e) => { println(e.message) }\n    }\n}\n";
    assert_eq!(run(src, "empty").trim(), "{}\nread");
}

#[test]
fn json_derives_beside_the_others_and_without_an_import() {
    let src = "@derive(Display, Json)\nstruct P {\n    x: Int\n}\nfn main() {\n    println(P { x: 1 }.to_string())\n    println(P.from_json(P { x: 2 }.to_json()).unwrap().x)\n}\n";
    assert_eq!(run(src, "beside").trim(), "P(x: 1)\n2");
}

#[test]
fn a_derived_type_crosses_modules() {
    let shapes = "@derive(Json)\npub struct Pt {\n    pub x: Int\n    pub y: Int\n\n    pub fn new(x: Int, y: Int) -> Pt { return Pt { x: x, y: y } }\n}\n@derive(Json)\npub enum Mark {\n    Spot(Pt)\n    Pair(Int | String, (Int, Int))\n}\n";
    let main = "import { Pt, Mark } from .shapes\nimport { to_text, parse } from std.data.json\n@derive(Json)\nstruct Holder {\n    at: Pt\n    mark: Mark\n}\nfn main() {\n    let h = Holder { at: Pt.new(1, 2), mark: Mark.Pair(\"a\", (3, 4)) }\n    let text = to_text(h.to_json()).unwrap()\n    println(text)\n    match Holder.from_json(parse(text).unwrap()) {\n        Ok(q) => { println(q.at.y) println(q.mark == h.mark) }\n        Err(e) => { println(e.message) }\n    }\n}\n";
    let (ok, text) = run_files(&[("shapes.mote", shapes), ("main.mote", main)], "run", "modules");
    assert!(ok && text.trim() == "{\"at\":{\"x\":1,\"y\":2},\"mark\":{\"Pair\":[\"a\",[3,4]]}}\n2\ntrue", "{text}");
}

#[test]
fn a_field_with_no_json_form_is_an_error() {
    rejected("@derive(Json)\nstruct B {\n    c: Char\n}\nfn main() { }\n", "char", "`@derive(Json)` has no JSON form for field `c` of type `Char`");
    rejected("@derive(Json)\nclass B {\n    m: Map<Int, Int>\n}\nfn main() { }\n", "mapkey", "has no JSON form for field `m` of type `Map<Int, Int>`");
    rejected("@derive(Json)\nenum E {\n    A(Char)\n}\nfn main() { }\n", "payload", "no JSON form for payload 1 of `A` of type `Char`");
}

#[test]
fn a_generic_type_cannot_derive_json() {
    rejected("@derive(Json)\nstruct Box<T> {\n    v: T\n}\nfn main() { }\n", "generic", "`@derive(Json)` does not support a generic type");
}

#[test]
fn deriving_over_an_existing_method_is_an_error() {
    rejected("@derive(Json)\nstruct P {\n    x: Int\n\n    fn to_json(self) -> Int { return 1 }\n}\nfn main() { }\n", "clash", "already has a method named `to_json`");
}

#[test]
fn a_field_type_without_the_derive_is_reported_at_the_attribute() {
    let src = "struct Plain {\n    n: Int\n}\n\n@derive(Json)\nstruct Outer {\n    p: Plain\n}\nfn main() { }\n";
    let (ok, text) = run_files(&[("main.mote", src)], "check", "plain");
    assert!(!ok && text.contains("no method `.to_json()` on `Plain`") && text.contains("main.mote:5:1"), "{text}");
}
