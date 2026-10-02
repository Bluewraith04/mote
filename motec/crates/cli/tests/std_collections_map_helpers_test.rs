//! `std.collections` Map helpers: `map_from`, `merge`, `map_values`, `filter_keys`.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_coll_map_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn map_from_builds_a_map_from_tuple_pairs() {
    let src = "import std.collections as coll\npub fn main() {\n    let m: Map<String, Int> = coll.map_from([(\"a\", 1), (\"b\", 2)])\n    println(m.get(\"a\").to_string())\n    println(m.get(\"b\").to_string())\n    println(m.len().to_string())\n}\n";
    let (ok, text) = run(src, "map_from");
    assert!(ok && text == "1\n2\n2\n", "{text}");
}

#[test]
fn merge_lets_the_second_map_win_on_collision() {
    let src = "import std.collections as coll\npub fn main() {\n    let a: Map<String, Int> = coll.map_from([(\"x\", 1), (\"y\", 2)])\n    let b: Map<String, Int> = coll.map_from([(\"y\", 20), (\"z\", 3)])\n    let m = coll.merge(a, b)\n    println(m.get(\"x\").to_string())\n    println(m.get(\"y\").to_string())\n    println(m.get(\"z\").to_string())\n    println(m.len().to_string())\n}\n";
    let (ok, text) = run(src, "merge");
    assert!(ok && text == "1\n20\n3\n3\n", "{text}");
}

#[test]
fn map_values_transforms_values_and_keeps_keys() {
    let src = "import std.collections as coll\npub fn main() {\n    let a: Map<String, Int> = coll.map_from([(\"x\", 1), (\"y\", 2)])\n    let scaled = coll.map_values(a, |v| { return v * 100 })\n    println(scaled.get(\"x\").to_string())\n    println(scaled.get(\"y\").to_string())\n}\n";
    let (ok, text) = run(src, "map_values");
    assert!(ok && text == "100\n200\n", "{text}");
}

#[test]
fn filter_keys_keeps_only_matching_entries() {
    let src = "import std.collections as coll\npub fn main() {\n    let a: Map<String, Int> = coll.map_from([(\"x\", 1), (\"y\", 2)])\n    let kept = coll.filter_keys(a, |k| { return k == \"x\" })\n    println(kept.len().to_string())\n    println(kept.contains_key(\"y\"))\n}\n";
    let (ok, text) = run(src, "filter_keys");
    assert!(ok && text == "1\nfalse\n", "{text}");
}
