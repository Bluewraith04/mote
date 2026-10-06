//! Std.string.rfind — byte offset of the last match.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_rfind_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn rfind_answers_the_last_match_offset() {
    let src = "pub fn main() {\n    println(\"abcabcabc\".rfind(\"bc\").unwrap().to_string())\n}\n";
    let (ok, text) = run(src, "last_match");
    assert!(ok && text == "7\n", "{text}");
}

#[test]
fn rfind_is_none_when_the_substring_is_absent() {
    let src = "pub fn main() {\n    println(\"hello\".rfind(\"z\").is_none())\n}\n";
    let (ok, text) = run(src, "absent");
    assert!(ok && text == "true\n", "{text}");
}

#[test]
fn rfind_differs_from_find_on_repeated_substrings() {
    let src = "pub fn main() {\n    let s = \"hello\"\n    println(s.find(\"l\").unwrap().to_string())\n    println(s.rfind(\"l\").unwrap().to_string())\n}\n";
    let (ok, text) = run(src, "differs_from_find");
    assert!(ok && text == "2\n3\n", "{text}");
}
