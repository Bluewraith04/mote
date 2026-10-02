//! Strings order byte-wise, so `<`, `<=`, `>` and `>=` and `sort` work on them.

use std::process::Command;

#[test]
fn strings_compare_lexicographically_by_bytes() {
    let source = r#"import std.iter as iter

fn main() {
    println("apple" < "banana")
    println("b" < "a")
    println("a" < "ab")
    println("" < "a")
    println("a" <= "a")
    println("b" >= "a")
    println("b" > "b")
    println("Z" < "a")
    println("z" < "\u{e9}")
    println("same" < "same")
    let names = ["Wren", "ash", "Sol", "Cy", "Bay"]
    println(iter.sort(names))
}
"#;
    let dir = std::env::temp_dir().join(format!("mote_string_order_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "true\nfalse\ntrue\ntrue\ntrue\ntrue\nfalse\ntrue\ntrue\nfalse\n[\"Bay\", \"Cy\", \"Sol\", \"Wren\", \"ash\"]"
    );
}
