//! `std.regex` — regular expressions over the `regex` crate, matching code points.

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
fn a_long_bounded_repeat_that_fails_does_not_blow_up() {
    let src = with_prelude(
        "pub fn main() {\n    let re = Regex.compile(\"^[a-z][a-z0-9_]{1,39}$\").unwrap()\n    println(re.is_match(\"a\".repeat(40)).to_string())\n    println(re.is_match(\"a\".repeat(41)).to_string())\n    println(Regex.compile(\"a{0,3}?b\").unwrap().find(\"aaab\").unwrap().text)\n    println(Regex.compile(\"a{1,3}?\").unwrap().find(\"aaa\").unwrap().text)\n}\n",
    );
    let (ok, text) = run(&src, "brace_fail");
    assert!(ok && text == "true\nfalse\naaab\na\n", "{text}");
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
    assert!(ok && text == "unclosed group\n", "{text}");
}

#[test]
fn bad_class_range_is_a_compile_error() {
    let (ok, text) = check(&with_prelude("pub fn main() {\n    Regex.compile(\"[z-a]\").unwrap()\n}\n"), "bad_range");
    assert!(ok, "{text}");
    let src = with_prelude("pub fn main() {\n    match Regex.compile(\"[z-a]\") {\n        Ok(r) => { println(\"unexpected\") }\n        Err(e) => { println(e.message) }\n    }\n}\n");
    let (run_ok, run_text) = run(&src, "bad_range_run");
    assert!(run_ok && run_text.contains("invalid character class range"), "{run_text}");
}

#[test]
fn a_repeat_too_big_to_compile_is_a_compile_error() {
    let src = with_prelude("pub fn main() {\n    match Regex.compile(\"(a{1000}){1000}\") {\n        Ok(r) => { println(\"unexpected\") }\n        Err(e) => { println(e.message) }\n    }\n}\n");
    let (ok, text) = run(&src, "too_large");
    assert!(ok && text.contains("size limit"), "{text}");
}

#[test]
fn backreferences_and_lookaround_are_compile_errors() {
    let src = with_prelude(
        r##"pub fn main() {
    for p in ["(a)\\1", "wor(?=ld)", "(?<=a)b", "(?!x)y"] {
        match Regex.compile(p) {
            Ok(r) => { println("compiled ${p}") }
            Err(e) => { println("error") }
        }
    }
}
"##,
    );
    let (ok, text) = run(&src, "unsupported");
    assert!(ok && text == "error\nerror\nerror\nerror\n", "{text}");
}

#[test]
fn dot_and_classes_match_characters_not_bytes() {
    let src = with_prelude(
        r##"pub fn main() {
    println(Regex.compile("^.{3}$").unwrap().is_match("日本語").to_string())
    println(Regex.compile("na.ve").unwrap().is_match("naïve").to_string())
    println(Regex.compile("^\\w+$").unwrap().is_match("naïve").to_string())
    println(Regex.compile("^[é-ü]+$").unwrap().is_match("éêü").to_string())
    println(Regex.compile("^\\d+$").unwrap().is_match("12٣").to_string())
    let m = Regex.compile("abc").unwrap().find("日本語 abc").unwrap()
    println("${m.start} ${m.end} ${m.text}")
}
"##,
    );
    let (ok, text) = run(&src, "code_points");
    assert!(ok && text == "true\ntrue\ntrue\ntrue\ntrue\n10 13 abc\n", "{text}");
}

#[test]
fn flags_and_posix_classes() {
    let src = with_prelude(
        r##"pub fn main() {
    println(Regex.compile("(?i)hello").unwrap().find("say HeLLo").unwrap().text)
    println(Regex.compile("(?i)straße").unwrap().is_match("STRASSE").to_string())
    println(Regex.compile("(?i)É").unwrap().is_match("é").to_string())
    let lines = Regex.compile("(?m)^b$").unwrap().find_all("a\nb\nb\nc")
    println(lines.len().to_string())
    println(Regex.compile("a.b").unwrap().is_match("a\nb").to_string())
    println(Regex.compile("(?s)a.b").unwrap().is_match("a\nb").to_string())
    println(Regex.compile("[[:alpha:]]+ world").unwrap().find("hello world").unwrap().text)
    println(Regex.compile("\\p{L}+").unwrap().find("123 héllo").unwrap().text)
}
"##,
    );
    let (ok, text) = run(&src, "flags");
    assert!(ok && text == "HeLLo\nfalse\ntrue\n2\nfalse\ntrue\nhello world\nhéllo\n", "{text}");
}

#[test]
fn find_from_starts_at_an_offset_and_still_sees_the_text_before_it() {
    let src = with_prelude(
        r##"pub fn main() {
    let re = Regex.compile("\\bb").unwrap()
    let m = re.find_from("ab b", 1).unwrap()
    println("${m.start} ${m.end}")
    println(Regex.compile("^a").unwrap().find_from("aa", 1).is_none().to_string())
    println(Regex.compile("a").unwrap().find_from("a", 9).is_none().to_string())
    let all = Regex.compile("o").unwrap()
    var at = 0
    var n = 0
    while true {
        match all.find_from("foo boo", at) {
            Some(m) => { n += 1; at = m.end }
            None => { break }
        }
    }
    println(n.to_string())
}
"##,
    );
    let (ok, text) = run(&src, "find_from");
    assert!(ok && text == "3 4\ntrue\ntrue\n4\n", "{text}");
}

#[test]
fn matching_time_is_linear_in_the_text() {
    let src = with_prelude(
        r##"pub fn main() {
    let re = Regex.compile("(a*)*b").unwrap()
    println(re.is_match("a".repeat(5000)).to_string())
    let big = "lorem ipsum 12 dolor ".repeat(5000)
    println(Regex.compile("\\d+").unwrap().find_all(big).len().to_string())
    println(Regex.compile("zzz[0-9]").unwrap().is_match(big).to_string())
}
"##,
    );
    let started = std::time::Instant::now();
    let (ok, text) = run(&src, "linear");
    assert!(ok && text == "false\n5000\nfalse\n", "{text}");
    assert!(started.elapsed().as_secs() < 20, "took {:?}", started.elapsed());
}

#[test]
fn empty_matches_never_split_a_character() {
    let src = with_prelude(
        r##"pub fn main() {
    let all = Regex.compile("").unwrap().find_all("é日")
    println(all.len().to_string())
    println(Regex.compile("x*").unwrap().split("aéb").len().to_string())
    println(Regex.compile("\\B").unwrap().find("é").is_none().to_string())
    println(Regex.compile("a*").unwrap().find_all("baaac").len().to_string())
}
"##,
    );
    let (ok, text) = run(&src, "empty");
    assert!(ok && text == "3\n5\ntrue\n3\n", "{text}");
}

#[test]
fn split_and_replace_all_over_unicode_text() {
    let src = with_prelude(
        r##"pub fn main() {
    let re = Regex.compile("[,;] *").unwrap()
    let parts = re.split("日本, 語;abc,é")
    println("${parts}")
    println(re.replace_all("日本, 語;abc,é", "|"))
    println(Regex.compile("z").unwrap().replace_all("none", "!"))
}
"##,
    );
    let (ok, text) = run(&src, "split_unicode");
    assert!(ok && text == "[\"日本\", \"語\", \"abc\", \"é\"]\n日本|語|abc|é\nnone\n", "{text}");
}

#[test]
fn a_regex_is_found_by_its_group_names() {
    let src = with_prelude(
        r##"pub fn main() {
    let re = Regex.compile("(?<year>\\d{4})-(?P<month>\\d{2})(-(\\d{2}))?").unwrap()
    let c = re.captures("on 2026-10 ok").unwrap()
    println(c.len().to_string())
    println(c.get_named("year").unwrap().text)
    println(c.get_named("month").unwrap().text)
    println(c.get(3).is_none().to_string())
    println(c.get_named("day").is_none().to_string())
}
"##,
    );
    let (ok, text) = run(&src, "names");
    assert!(ok && text == "5\n2026\n10\ntrue\ntrue\n", "{text}");
}
