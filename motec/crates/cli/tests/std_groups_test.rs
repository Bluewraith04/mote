//! `std.data`, `std.sys` and `std.dev` are groups: a member is imported by path, through the group, or by name from the group.

use std::process::Command;

fn run(source: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_std_groups_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    text
}

#[test]
fn a_group_opens_its_members_as_a_namespace() {
    let src = "import std.data

fn main() {
    println(data.base64.encode(\"hi\".bytes()))
    println(data.json.to_text(data.yaml.parse(\"a: [1, 2]\").unwrap()).unwrap())
    println(data.uuid.v4().version())
}
";
    assert_eq!(run(src, "namespace"), "aGk=\n{\"a\":[1,2]}\n4\n");
}

#[test]
fn a_group_can_be_renamed() {
    let src = "import std.sys as os

fn main() {
    println(os.fs.exists(\"/no/such/path/here\"))
    println(os.path.join(\"a\", \"b\"))
}
";
    assert_eq!(run(src, "renamed"), "false\na/b\n");
}

#[test]
fn members_import_by_name_from_the_group_or_by_path() {
    let src = "import { json, toml } from std.data
import std.data.base64 as b64
import { Json } from std.data.json

fn main() {
    let j: Json = toml.parse(\"a = 1\").unwrap()
    println(json.to_text(j).unwrap())
    println(b64.encode(\"a\".bytes()))
}
";
    assert_eq!(run(src, "by_name"), "{\"a\":1}\nYQ==\n");
}

#[test]
fn a_type_and_a_pattern_are_named_through_the_group() {
    let src = "import std.data

fn describe(j: data.json.Json) -> String {
    match j {
        data.json.Json.Str(s) => { return \"text ${s}\" }
        data.json.Json.Int(n) => { return \"number ${n}\" }
        _ => { return \"other\" }
    }
}

fn main() {
    println(describe(data.json.Json.Str(\"x\")))
    println(describe(data.json.Json.Int(4)))
    println(describe(data.json.Json.Null))
}
";
    assert_eq!(run(src, "types"), "text x\nnumber 4\nother\n");
}

#[test]
fn derives_still_work_without_naming_the_group() {
    let src = "@derive(Json)
struct P {
    x: Int
}

@derive(Args)
struct Opts {
    verbose: Bool
}

fn main() {
    println(P.from_json(P { x: 2 }.to_json()).unwrap().x)
    println(Opts.parse([\"prog\", \"--verbose\"]).unwrap().verbose)
}
";
    assert_eq!(run(src, "derives"), "2\ntrue\n");
}
