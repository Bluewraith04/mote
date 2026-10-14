//! `std.term.width` and `std.term.glyphs`: the columns a text takes on a terminal.

use std::process::Command;

fn run(name: &str, body: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_term_width_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), body).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn width_counts_columns_not_characters() {
    let body = r#"import std.term as term

fn main() {
    println(term.width(""))
    println(term.width("hello"))
    println(term.width("é"))
    println(term.width("日本語"))
    println(term.width("a😀b"))
    println(term.width("e\u{301}"))
    println(term.width("👨\u{200d}👩\u{200d}👧"))
    println(term.width("🇯🇵"))
    println(term.width("a\tb"))
}
"#;
    assert_eq!(run("width", body), "0\n5\n1\n6\n4\n1\n2\n2\n3");
}

#[test]
fn glyphs_put_an_empty_entry_after_a_wide_character() {
    let body = r#"import std.term as term

fn main() {
    let g = term.glyphs("日a")
    println(g.len())
    println(g.get(0) == "日")
    println(g.get(1) == "")
    println(g.get(2) == "a")
    println(term.glyphs("").len())
    println(term.glyphs("x\r\ny").len())
}
"#;
    assert_eq!(run("glyphs", body), "3\ntrue\ntrue\ntrue\n0\n4");
}
