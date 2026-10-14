//! Traits listed on a type work without the program importing their derive modules.

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
fn listed_traits_work_without_naming_the_derive_modules() {
    let src = "import { JsonCodec } from std.json
import { Args } from std.args
struct P: (JsonCodec) {
    x: Int
}

struct Opts: (Args) {
    verbose: Bool
}

fn main() {
    println(P.from_json(P { x: 2 }.to_json()).unwrap().x)
    println(Opts.parse([\"prog\", \"--verbose\"]).unwrap().verbose)
}
";
    assert_eq!(run(src, "derives"), "2\ntrue\n");
}

#[test]
fn std_modules_import_by_flat_path_and_by_name() {
    let src = "import std.base64 as b64
import std.json as json
import { Json } from std.json

fn main() {
    let j: Json = json.parse(\"{\\\"a\\\": 1}\").unwrap()
    println(json.to_text(j).unwrap())
    println(b64.encode(\"a\".bytes()))
}
";
    assert_eq!(run(src, "flat"), "{\"a\":1}\nYQ==\n");
}
