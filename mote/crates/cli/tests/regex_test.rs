//! `std.regex` — a byte-wise backtracking regex engine.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_regex_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn check(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_regex_chk_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("check").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn with_prelude(body: &str) -> String {
    format!("import {{ Regex }} from std.regex\n\n{body}")
}

#[test]
fn literal_and_class_and_plus() {
    let src = with_prelude(
        "pub fn main() {\n    let re = Regex.compile(\"a[bc]+d\").unwrap()\n    println(re.is_match(\"xabccd\").to_string())\n    println(re.is_match(\"xyz\").to_string())\n}\n",
    );
    let (ok, text) = run(&src, "class_plus");
    assert!(ok && text == "true\nfalse\n", "{text}");
}

#[test]
fn anchors() {
    let src = with_prelude(
        "pub fn main() {\n    let re = Regex.compile(\"^ab$\").unwrap()\n    println(re.is_match(\"ab\").to_string())\n    println(re.is_match(\"xab\").to_string())\n    println(re.is_match(\"abx\").to_string())\n}\n",
    );
    let (ok, text) = run(&src, "anchors");
    assert!(ok && text == "true\nfalse\nfalse\n", "{text}");
}

#[test]
fn numbered_capture_groups() {
    let src = with_prelude(
        "pub fn main() {\n    let re = Regex.compile(\"(\\\\d+)-(\\\\d+)\").unwrap()\n    let caps = re.captures(\"id 12-345 end\").unwrap()\n    println(caps.get(0).unwrap().text)\n    println(caps.get(1).unwrap().text)\n    println(caps.get(2).unwrap().text)\n}\n",
    );
    let (ok, text) = run(&src, "captures");
    assert!(ok && text == "12-345\n12\n345\n", "{text}");
}

#[test]
fn named_capture_group() {
    let src = with_prelude(
        "pub fn main() {\n    let re = Regex.compile(\"(?<word>[a-z]+)\").unwrap()\n    let caps = re.captures(\"  hello \").unwrap()\n    println(caps.get_named(\"word\").unwrap().text)\n}\n",
    );
    let (ok, text) = run(&src, "named");
    assert!(ok && text == "hello\n", "{text}");
}

#[test]
fn alternation() {
    let src = with_prelude(
        "pub fn main() {\n    let re = Regex.compile(\"cat|dog|bird\").unwrap()\n    println(re.is_match(\"a cat sat\").to_string())\n    println(re.is_match(\"a fish sat\").to_string())\n}\n",
    );
    let (ok, text) = run(&src, "alt");
    assert!(ok && text == "true\nfalse\n", "{text}");
}

#[test]
fn greedy_vs_lazy_star() {
    let src = with_prelude(
        "pub fn main() {\n    println(Regex.compile(\"a.*b\").unwrap().find(\"a1b2b\").unwrap().text)\n    println(Regex.compile(\"a.*?b\").unwrap().find(\"a1b2b\").unwrap().text)\n}\n",
    );
    let (ok, text) = run(&src, "greedy_lazy");
    assert!(ok && text == "a1b2b\na1b\n", "{text}");
}

#[test]
fn brace_repeat_range() {
    let src = with_prelude("pub fn main() {\n    println(Regex.compile(\"a{2,3}\").unwrap().find(\"aaaa\").unwrap().text)\n}\n");
    let (ok, text) = run(&src, "brace");
    assert!(ok && text == "aaa\n", "{text}");
}

#[test]
fn find_all_and_split_and_replace_all() {
    let src = with_prelude(
        "pub fn main() {\n    let re = Regex.compile(\"\\\\d+\").unwrap()\n    let all = re.find_all(\"a1 b22 c333\")\n    println(all.len().to_string())\n    var i = 0\n    while i < all.len() { println(all.get(i).text); i = i + 1 }\n    let parts = re.split(\"a1 b22 c333\")\n    println(parts.len().to_string())\n    println(re.replace_all(\"a1 b22 c333\", \"#\"))\n}\n",
    );
    let (ok, text) = run(&src, "find_all");
    assert!(ok && text == "3\n1\n22\n333\n4\na# b# c#\n", "{text}");
}

#[test]
fn word_boundary() {
    let src = with_prelude(
        "pub fn main() {\n    let re = Regex.compile(\"\\\\bcat\\\\b\").unwrap()\n    println(re.is_match(\"a cat sat\").to_string())\n    println(re.is_match(\"catalog\").to_string())\n}\n",
    );
    let (ok, text) = run(&src, "word_boundary");
    assert!(ok && text == "true\nfalse\n", "{text}");
}

#[test]
fn non_capturing_group() {
    let src = with_prelude("pub fn main() {\n    println(Regex.compile(\"(?:ab)+c\").unwrap().is_match(\"ababc\").to_string())\n}\n");
    let (ok, text) = run(&src, "noncap");
    assert!(ok && text == "true\n", "{text}");
}

#[test]
fn literal_bytes_match_non_ascii_text() {
    let src = with_prelude("pub fn main() {\n    println(Regex.compile(\"caf\\u{00e9}\").unwrap().is_match(\"caf\\u{00e9} au lait\").to_string())\n}\n");
    let (ok, text) = run(&src, "unicode_literal");
    assert!(ok && text == "true\n", "{text}");
}

#[test]
fn unterminated_group_is_a_compile_error() {
    let src = with_prelude("pub fn main() {\n    match Regex.compile(\"a(b\") {\n        Ok(r) => { println(\"unexpected\") }\n        Err(e) => { println(e.message) }\n    }\n}\n");
    let (ok, text) = run(&src, "unterminated");
    assert!(ok && text.contains("unterminated group"), "{text}");
}

#[test]
fn bad_class_range_is_a_compile_error() {
    let (ok, text) = check(&with_prelude("pub fn main() {\n    Regex.compile(\"[z-a]\").unwrap()\n}\n"), "bad_range");
    assert!(ok, "{text}");
    let src = with_prelude("pub fn main() {\n    match Regex.compile(\"[z-a]\") {\n        Ok(r) => { println(\"unexpected\") }\n        Err(e) => { println(e.message) }\n    }\n}\n");
    let (run_ok, run_text) = run(&src, "bad_range_run");
    assert!(run_ok && run_text.contains("end before start"), "{run_text}");
}

#[test]
fn repeat_count_too_large_is_a_compile_error() {
    let src = with_prelude("pub fn main() {\n    match Regex.compile(\"a{5000}\") {\n        Ok(r) => { println(\"unexpected\") }\n        Err(e) => { println(e.message) }\n    }\n}\n");
    let (ok, text) = run(&src, "too_large");
    assert!(ok && text.contains("too large"), "{text}");
}
