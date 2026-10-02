//! `std.data.toml`.

use std::process::Command;

fn run(source: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_toml_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    text
}

fn lit(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

const HEAD: &str = r#"import std.data.toml as toml
import std.data.json as json

fn show(src: String) {
    match toml.parse(src) {
        Ok(j) => {
            match json.to_text(j) {
                Ok(t) => { println(t) }
                Err(e) => { println("json: ${e.message}") }
            }
        }
        Err(e) => { println("error: ${e.message}") }
    }
}

fn write(src: String) {
    match json.parse(src) {
        Ok(j) => {
            match toml.to_text(j) {
                Ok(t) => { println(t + "---") }
                Err(e) => { println("write error: ${e.message}") }
            }
        }
        Err(e) => { println("json error: ${e.message}") }
    }
}

fn exact(src: String) {
    let j = toml.parse(src).unwrap()
    let k = toml.parse(toml.to_text(j).unwrap()).unwrap()
    println(json.to_text(j).unwrap() == json.to_text(k).unwrap())
}

fn again(src: String) {
    let first = toml.to_text(toml.parse(src).unwrap()).unwrap()
    let second = toml.to_text(toml.parse(first).unwrap()).unwrap()
    println(first == second)
}
"#;

fn lines(func: &str, cases: &[&str], tag: &str) -> Vec<String> {
    let calls: Vec<String> = cases.iter().map(|c| format!("    {func}({})\n", lit(c))).collect();
    let src = format!("{HEAD}\nfn main() {{\n{}}}\n", calls.concat());
    run(&src, tag).lines().map(String::from).collect()
}

#[test]
fn scalars_read_as_json_values() {
    let got = lines(
        "show",
        &[
            "title = \"x\"\nn = 1_000\nf = 3.14\nb = true\nh = 0xFF\ne = 5e+2\n",
            "x = +1\ny = -0\nz = 1e3\nw = +1.5e-3\nv = 6.02e23\nu = 1_0.0_1\n",
            "x = 0b1010\ny = 0o17\nz = 0xdead_beef\nw = -9223372036854775808\n",
            "x = \"\\u00e9 \\U0001F600\"\n",
            "x = [1, \"a\", 2.5, [true]]\n",
            "x = {}\ny = []\n",
        ],
        "scalars",
    );
    assert_eq!(
        got,
        [
            "{\"title\":\"x\",\"n\":1000,\"f\":3.14,\"b\":true,\"h\":255,\"e\":500.0}",
            "{\"x\":1,\"y\":0,\"z\":1000.0,\"w\":0.0015,\"v\":602000000000000000000000.0,\"u\":10.01}",
            "{\"x\":10,\"y\":15,\"z\":3735928559,\"w\":-9223372036854775808}",
            "{\"x\":\"é 😀\"}",
            "{\"x\":[1,\"a\",2.5,[true]]}",
            "{\"x\":{},\"y\":[]}",
        ]
    );
}

#[test]
fn dates_and_times_read_as_their_text() {
    let got = lines("show", &["d = 1979-05-27T07:32:00Z\nt = 07:32:00.5\ndt = 1979-05-27 07:32:00-07:00\nld = 1979-05-27\n"], "dates");
    assert_eq!(got, ["{\"d\":\"1979-05-27T07:32:00Z\",\"t\":\"07:32:00.5\",\"dt\":\"1979-05-27T07:32:00-07:00\",\"ld\":\"1979-05-27\"}"]);
}

#[test]
fn strings_of_every_kind() {
    let got = lines(
        "show",
        &["s = \"\"\"\nhello \\\n   world\"\"\"\nt = '''\nraw\\n'''\nu = 'lit'\nv = \"tab\there\"\nw = \"\"\"a\"\"\"\"\n"],
        "strings",
    );
    assert_eq!(got, ["{\"s\":\"hello world\",\"t\":\"raw\\\\n\",\"u\":\"lit\",\"v\":\"tab\\there\",\"w\":\"a\\\"\"}"]);
}

#[test]
fn tables_dotted_keys_and_arrays_of_tables() {
    let got = lines(
        "show",
        &[
            "[a]\nx = 1\n[a.b]\ny = [1, 2, [3]]\n[[c]]\nz = 1\n[[c]]\nz = 2\n[c.sub]\nw = 3\n",
            "a.b.c = 1\na.b.d = { p = 1, q.r = 2 }\n\"k k\" = 'lit'\n",
            "[a.b]\n[a]\nx=1\n",
            "\"a\".'b'.c = 1\n",
            "[a . b]\nc = 1\n",
            "[a]\nb = 1\n\n[c]\nd = 2 # trailing\n",
            "x = [{a = 1}, {a = 2}]\n",
            "[[a]]\nx = 1\n[a.b]\ny = 2\n[[a]]\nx = 3\n",
        ],
        "tables",
    );
    assert_eq!(
        got,
        [
            "{\"a\":{\"x\":1,\"b\":{\"y\":[1,2,[3]]}},\"c\":[{\"z\":1},{\"z\":2,\"sub\":{\"w\":3}}]}",
            "{\"a\":{\"b\":{\"c\":1,\"d\":{\"p\":1,\"q\":{\"r\":2}}}},\"k k\":\"lit\"}",
            "{\"a\":{\"b\":{},\"x\":1}}",
            "{\"a\":{\"b\":{\"c\":1}}}",
            "{\"a\":{\"b\":{\"c\":1}}}",
            "{\"a\":{\"b\":1},\"c\":{\"d\":2}}",
            "{\"x\":[{\"a\":1},{\"a\":2}]}",
            "{\"a\":[{\"x\":1,\"b\":{\"y\":2}},{\"x\":3}]}",
        ]
    );
}

#[test]
fn layout_comments_and_line_ends() {
    let got = lines("show", &["x = [ # c\n 1, # d\n 2,\n]\n", "a = 1\r\nb = 2\r\n", "", "# only a comment\n", "a = 1"], "layout");
    assert_eq!(got, ["{\"x\":[1,2]}", "{\"a\":1,\"b\":2}", "{}", "{}", "{\"a\":1}"]);
}

#[test]
fn a_bad_document_is_an_error_with_a_position() {
    let got = lines(
        "show",
        &[
            "a = 1\na = 2\n",
            "a = {x=1}\n[a.y]\n",
            "x = 9223372036854775808\n",
            "x = 1 y = 2\n",
            "x = 01\n",
            "x = 1__0\n",
            "x = [1, 2\n",
            "d = 2021-02-30\n",
            "[]\n",
            "a = true\nb = false\nc = trues\n",
        ],
        "errors",
    );
    assert_eq!(
        got,
        [
            "error: line 2, column 1: duplicate key",
            "error: line 2, column 2: cannot extend value of type inline table with a dotted key",
            "error: line 1, column 5: u64 value was too large",
            "error: line 1, column 9: unexpected key or value, expected newline, `#`",
            "error: line 1, column 5: unexpected leading zero, expected nothing",
            "error: line 1, column 6: `_` may only go between digits, expected nothing",
            "error: line 1, column 10: unclosed array, expected `]`",
            "error: line 1, column 5: invalid date, expected day between 01 and 28",
            "error: line 1, column 2: unquoted keys cannot be empty, expected letters, numbers, `-`, `_`",
            "error: line 3, column 5: invalid boolean, expected `true`",
        ]
    );
}

#[test]
fn an_object_prints_as_a_document() {
    let got = lines(
        "write",
        &["{\"a\": 1, \"b\": {\"c\": \"x\\ty\\\"\", \"d e\": [1, 2.5, \"s\"]}, \"f\": [{\"g\": 1}, {\"g\": 2, \"h\": {}}], \"m\": [{\"a\": 1}, 2], \"e\": {}}"],
        "write",
    );
    assert_eq!(
        got,
        [
            "a = 1",
            "m = [{ a = 1 }, 2]",
            "",
            "[b]",
            "c = 'x\ty\"'",
            "\"d e\" = [1, 2.5, \"s\"]",
            "",
            "[[f]]",
            "g = 1",
            "",
            "[[f]]",
            "g = 2",
            "",
            "[f.h]",
            "",
            "[e]",
            "---",
        ]
    );
}

#[test]
fn keys_floats_and_the_empty_object() {
    let got = lines("write", &["{\"\": 1, \"é\": 2, \"z\": 3.0, \"w\": 1e30}", "{}"], "keys");
    assert_eq!(got, ["\"\" = 1", "\"é\" = 2", "z = 3.0", "w = 1000000000000000000000000000000.0", "---", "---"]);
}

#[test]
fn infinity_and_nan_read_as_strings() {
    let got = lines("show", &["a = inf\nb = -inf\nc = nan\nd = +inf\n"], "inf");
    assert_eq!(got, ["{\"a\":\"inf\",\"b\":\"-inf\",\"c\":\"nan\",\"d\":\"inf\"}"]);
}

#[test]
fn what_cannot_be_written_is_an_error() {
    let got = lines("write", &["{\"a\": null}", "[1]", "{\"a\": {\"b\": [null]}}"], "refused");
    assert_eq!(
        got,
        [
            "write error: TOML has no null (at a)",
            "write error: a TOML document is a table, not a value",
            "write error: TOML has no null (at a.b[0])",
        ]
    );
}

#[test]
fn a_document_survives_a_write_and_a_read() {
    let got = lines(
        "again",
        &[
            "[a]\nx = 1\n[a.b]\ny = [1, 2, [3]]\n[[c]]\nz = 1\n[[c]]\nz = 2\n[c.sub]\nw = 3\n",
            "a.b.c = 1\na.b.d = { p = 1, q.r = 2 }\n\"k k\" = 'lit'\n",
            "[[a]]\nx = 1\n[a.b]\ny = 2\n[[a]]\nx = 3\n",
            "s = \"\"\"\nline one\nline \\\"two\\\"\"\"\"\nd = 1979-05-27\nf = 1.5e-7\nn = -17\n\"weird key\" = \"tab\\there\"\n",
        ],
        "again",
    );
    assert_eq!(got, ["true", "true", "true", "true"]);
}

#[test]
fn a_document_in_writer_order_reads_back_as_the_same_value() {
    let got = lines(
        "exact",
        &[
            "[a]\nx = 1\n[a.b]\ny = [1, 2, [3]]\n[[c]]\nz = 1\n[[c]]\nz = 2\n[c.sub]\nw = 3\n",
            "[[a]]\nx = 1\n[a.b]\ny = 2\n[[a]]\nx = 3\n",
            "s = \"\"\"\nline one\nline \\\"two\\\"\"\"\"\nd = 1979-05-27\nf = 1.5e-7\nn = -17\n\"weird key\" = \"tab\\there\"\nm = [{ a = 1 }, 2]\n",
        ],
        "exact",
    );
    assert_eq!(got, ["true", "true", "true"]);
}

#[test]
fn done_bar_a_config_read_into_derived_types() {
    let out = run(include_str!("programs_mote/config_data.mote"), "done_bar");
    let expected = "demo on localhost:8080
16
2026-10-02T12:30:00Z
7
36
true
true
name = \"demo\"
tags = [\"edge\", \"blue\"]
started = \"2026-10-02T12:30:00Z\"

[server]
host = \"localhost\"
port = 8080
tls = false

[limits]
rate = 5
burst = 16

";
    assert_eq!(out, expected);
}
