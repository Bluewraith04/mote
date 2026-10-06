//! `std.data.uuid`.

use std::process::Command;

fn run(source: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_uuid_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    text
}

const HEAD: &str = "import { Uuid, v4, v7 } from std.data.uuid\nimport std.data.json as json\n";

fn lines(body: &str, tag: &str) -> Vec<String> {
    run(&format!("{HEAD}\nfn main() {{\n{body}}}\n"), tag).lines().map(String::from).collect()
}

#[test]
fn a_uuid_prints_as_lowercase_groups() {
    let body = "    let u = Uuid.parse(\"0189F7C0-1234-7ABC-8DEF-0123456789AB\").unwrap()
    println(u.to_string())
    println(\"id: ${u}\")
    println(u.version())
    println(Uuid.nil().to_string())
    println(Uuid.parse(\"ffffffff-ffff-ffff-ffff-ffffffffffff\").unwrap().to_string())
";
    assert_eq!(
        lines(body, "print"),
        ["0189f7c0-1234-7abc-8def-0123456789ab", "id: 0189f7c0-1234-7abc-8def-0123456789ab", "7", "00000000-0000-0000-0000-000000000000", "ffffffff-ffff-ffff-ffff-ffffffffffff"]
    );
}

#[test]
fn bytes_and_text_round_trip() {
    let body = "    let u = Uuid.parse(\"0189f7c0-1234-7abc-8def-0123456789ab\").unwrap()
    let b = u.to_bytes()
    println(b.len())
    println(b.get(0))
    println(b.get(15))
    println(Uuid.from_bytes(b).unwrap() == u)
    println(Uuid.parse(u.to_string()).unwrap() == u)
    println(Uuid.nil() == Uuid.nil())
    println(u == Uuid.nil())
";
    assert_eq!(lines(body, "bytes"), ["16", "1", "171", "true", "true", "true", "false"]);
}

#[test]
fn bad_text_or_bytes_is_an_error() {
    let body = "    for t in [\"nope\", \"0189f7c0-1234-7abc-8def-0123456789a\", \"0189f7c0_1234-7abc-8def-0123456789ab\", \"0189f7c0-1234-7abc-8def-0123456789gb\", \"\"] {
        match Uuid.parse(t) {
            Ok(u) => { println(\"ok\") }
            Err(e) => { println(e.message) }
        }
    }
    match Uuid.from_bytes(Bytes(15)) {
        Ok(u) => { println(\"ok\") }
        Err(e) => { println(e.message) }
    }
";
    assert_eq!(
        lines(body, "bad"),
        [
            "invalid character, found `n` at 0",
            "invalid group length in group 4, expected 12, found 11",
            "invalid character, found `_` at 8",
            "invalid character, found `g` at 34",
            "invalid length, found 0",
            "invalid length, expected 16 bytes, found 15",
        ]
    );
}

#[test]
fn v4_sets_version_and_variant_and_is_random() {
    let body = "    var seen = Set<String>()
    var ok = true
    var i = 0
    while i < 200 {
        let u = v4()
        let s = u.to_string()
        seen.add(s)
        let variant = s.slice(19, 20)
        if u.version() != 4 || s.len() != 36 || !(variant == \"8\" || variant == \"9\" || variant == \"a\" || variant == \"b\") { ok = false }
        i = i + 1
    }
    println(ok)
    println(seen.len())
";
    assert_eq!(lines(body, "v4"), ["true", "200"]);
}

#[test]
fn v7_carries_the_time_and_sorts_by_it() {
    let src = "import { Uuid, v7 } from std.data.uuid
import { unix_millis, sleep, Duration } from std.time

fn main() {
    let before = unix_millis()
    let a = v7()
    sleep(Duration.from_millis(3))
    let b = v7()
    let after = unix_millis()
    let bytes = a.to_bytes()
    var ms = 0
    var i = 0
    while i < 6 {
        ms = ms * 256 + bytes.get(i)
        i = i + 1
    }
    println(a.version())
    println(ms >= before && ms <= after)
    println(a.to_string() < b.to_string())
    println(a == b)
}
";
    assert_eq!(run(src, "v7").lines().collect::<Vec<_>>(), ["7", "true", "true", "false"]);
}

#[test]
fn a_uuid_is_a_json_string_and_a_derived_field() {
    let src = "import { Uuid } from std.data.uuid
import std.data.json as json

@derive(Json)
struct Row {
    id: Uuid
    name: String
}

fn main() {
    let id = Uuid.parse(\"0189f7c0-1234-7abc-8def-0123456789ab\").unwrap()
    let text = json.to_text(Row { id: id, name: \"a\" }.to_json()).unwrap()
    println(text)
    let row = Row.from_json(json.parse(text).unwrap()).unwrap()
    println(row.id == id)
    match Row.from_json(json.parse(\"{\\\"id\\\": \\\"x\\\", \\\"name\\\": \\\"a\\\"}\").unwrap()) {
        Ok(r) => { println(\"ok\") }
        Err(e) => { println(e.message) }
    }
    match Row.from_json(json.parse(\"{\\\"id\\\": 1, \\\"name\\\": \\\"a\\\"}\").unwrap()) {
        Ok(r) => { println(\"ok\") }
        Err(e) => { println(e.message) }
    }
}
";
    assert_eq!(
        run(src, "json").lines().collect::<Vec<_>>(),
        [
            "{\"id\":\"0189f7c0-1234-7abc-8def-0123456789ab\",\"name\":\"a\"}",
            "true",
            "id: invalid character, found `x` at 0",
            "id: expected a UUID string",
        ]
    );
}
