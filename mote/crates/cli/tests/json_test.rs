//! `std.json` and the number parsing it needs, through `mote run`.

use std::process::Command;

fn run(name: &str, body: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_json_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), body).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn parse_int_is_strict_and_checks_overflow() {
    let body = r#"import std.string as string

fn main() {
    println(string.parse_int("42").unwrap())
    println(string.parse_int("-7").unwrap())
    println(string.parse_int("0").unwrap())
    println(string.parse_int("9223372036854775807").unwrap())
    println(string.parse_int("-9223372036854775808").unwrap())
    println(string.parse_int("9223372036854775808").is_err())
    println(string.parse_int("+1").is_err())
    println(string.parse_int(" 1").is_err())
    println(string.parse_int("1 ").is_err())
    println(string.parse_int("").is_err())
    println(string.parse_int("-").is_err())
    println(string.parse_int("1.5").is_err())
    println(string.parse_int("0x10").is_err())
    println(string.parse_int("1_000").is_err())
}
"#;
    assert_eq!(
        run("parse_int", body),
        "42\n-7\n0\n9223372036854775807\n-9223372036854775808\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue"
    );
}

#[test]
fn parse_float_reads_decimals_and_rejects_everything_else() {
    let body = r#"import std.string as string

fn main() {
    println(string.parse_float("1.5").unwrap() == 1.5)
    println(string.parse_float("-0.25").unwrap() == -0.25)
    println(string.parse_float("1e3").unwrap() == 1000.0)
    println(string.parse_float("2E-2").unwrap() == 0.02)
    println(string.parse_float("12").unwrap() == 12.0)
    println(string.parse_float("6.02e23").unwrap() > 6.0e23)
    println(string.parse_float("inf").is_err())
    println(string.parse_float("NaN").is_err())
    println(string.parse_float(".5").is_err())
    println(string.parse_float("+1").is_err())
    println(string.parse_float("1e999").is_err())
    println(string.parse_float("1.5x").is_err())
    println(string.parse_float(" 1.5").is_err())
    println(string.parse_float("").is_err())
    println(string.parse_float("-").is_err())
}
"#;
    assert_eq!(run("parse_float", body), "true\n".repeat(15).trim_end());
}

const TREE: &str = r#"import std.json as json
import { Json, Member } from std.json

fn sample() -> Json {
    let inner = Json.Object([json.member("ok", Json.Bool(true)), json.member("none", Json.Null)])
    return Json.Object([
        json.member("name", Json.Str("mote")),
        json.member("count", Json.Int(3)),
        json.member("ratio", Json.Float(0.5)),
        json.member("tags", Json.Array([Json.Str("a"), Json.Int(2), inner])),
        json.member("empty", Json.Array([])),
        json.member("nothing", Json.Object([]))
    ])
}
"#;

#[test]
fn a_tree_prints_compact_and_pretty() {
    let body = format!(
        r#"{TREE}
fn main() {{
    println(json.to_text(sample()).unwrap())
    println(json.pretty(sample(), 2).unwrap())
}}
"#
    );
    let expected = concat!(
        "{\"name\":\"mote\",\"count\":3,\"ratio\":0.5,\"tags\":[\"a\",2,{\"ok\":true,\"none\":null}],\"empty\":[],\"nothing\":{}}\n",
        "{\n",
        "  \"name\": \"mote\",\n",
        "  \"count\": 3,\n",
        "  \"ratio\": 0.5,\n",
        "  \"tags\": [\n",
        "    \"a\",\n",
        "    2,\n",
        "    {\n",
        "      \"ok\": true,\n",
        "      \"none\": null\n",
        "    }\n",
        "  ],\n",
        "  \"empty\": [],\n",
        "  \"nothing\": {}\n",
        "}"
    );
    assert_eq!(run("print", &body), expected);
}

#[test]
fn strings_escape_only_what_json_requires() {
    let body = format!(
        r#"{TREE}
fn main() {{
    let text = "quote\" slash\\ tab\t nl\n cr\r bell" + "\u{{7}}" + " caf\u{{e9}} \u{{1f600}}"
    println(json.to_text(Json.Str(text)).unwrap())
    println(json.to_text(Json.Str("")).unwrap())
    println(json.to_text(Json.Object([json.member("k\"ey", Json.Int(1))])).unwrap())
}}
"#
    );
    assert_eq!(
        run("escape", &body),
        "\"quote\\\" slash\\\\ tab\\t nl\\n cr\\r bell\\u0007 café 😀\"\n\"\"\n{\"k\\\"ey\":1}"
    );
}

#[test]
fn floats_keep_a_marker_and_non_finite_ones_are_errors() {
    let body = format!(
        r#"{TREE}
import std.math as math

fn main() {{
    println(json.to_text(Json.Float(1.0)).unwrap())
    println(json.to_text(Json.Float(-0.0)).unwrap())
    println(json.to_text(Json.Float(0.1)).unwrap())
    println(json.to_text(Json.Float(1.5e-7)).unwrap())
    println(json.to_text(Json.Float(math.inf())).is_err())
    println(json.to_text(Json.Float(math.nan())).is_err())
    println(json.to_text(Json.Array([Json.Int(1), Json.Float(math.inf())])).is_err())
    println(json.to_text(Json.Int(-9223372036854775807 - 1)).unwrap())
}}
"#
    );
    assert_eq!(run("floats", &body), "1.0\n-0.0\n0.1\n0.00000015\ntrue\ntrue\ntrue\n-9223372036854775808");
}

#[test]
fn accessors_stop_at_none_when_the_shape_is_wrong() {
    let body = format!(
        r#"{TREE}
fn main() {{
    let v = sample()
    println(json.as_str(json.get(v, "name").unwrap()).unwrap())
    println(json.as_int(json.get(v, "count").unwrap()).unwrap())
    println(json.as_float(json.get(v, "count").unwrap()).unwrap() == 3.0)
    println(json.as_float(json.get(v, "ratio").unwrap()).unwrap() == 0.5)
    let tags = json.get(v, "tags").unwrap()
    println(json.as_int(json.at(tags, 1).unwrap()).unwrap())
    println(json.as_bool(json.get(json.at(tags, 2).unwrap(), "ok").unwrap()).unwrap())
    println(json.is_null(json.get(json.at(tags, 2).unwrap(), "none").unwrap()))
    println(json.get(v, "missing").is_none())
    println(json.at(tags, 3).is_none())
    println(json.at(tags, -1).is_none())
    println(json.get(tags, "x").is_none())
    println(json.at(v, 0).is_none())
    println(json.as_int(Json.Str("3")).is_none())
    println(json.as_str(Json.Int(3)).is_none())
    println(json.as_array(Json.Null).is_none())
    println(json.as_object(Json.Null).is_none())
    let dup = Json.Object([json.member("k", Json.Int(1)), json.member("k", Json.Int(2))])
    println(json.as_int(json.get(dup, "k").unwrap()).unwrap())
}}
"#
    );
    assert_eq!(
        run("access", &body),
        "mote\n3\ntrue\ntrue\n2\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n2"
    );
}

fn mote_str(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[test]
fn accepted_documents_print_back_in_canonical_form() {
    let cases: &[(&str, &str)] = &[
        ("[1,2 , 3 ]", "[1,2,3]"),
        (" \t\r\n{\"a\" : { \"b\" : [ ] } }\n", "{\"a\":{\"b\":[]}}"),
        ("-0", "0"),
        ("0", "0"),
        ("-12", "-12"),
        ("1e3", "1000.0"),
        ("1E+2", "100.0"),
        ("2.50", "2.5"),
        ("-0.0", "-0.0"),
        ("9223372036854775807", "9223372036854775807"),
        ("-9223372036854775808", "-9223372036854775808"),
        ("\"\\u00e9\\ud83d\\ude00\"", "\"é😀\""),
        ("\"a\\/b\"", "\"a/b\""),
        ("\"\\b\\f\\n\\r\\t\\\"\\\\\"", "\"\\b\\f\\n\\r\\t\\\"\\\\\""),
        ("\"\\u0001\"", "\"\\u0001\""),
        ("true", "true"),
        ("[false,null]", "[false,null]"),
        ("{\"k\":1,\"k\":2}", "{\"k\":1,\"k\":2}"),
        ("\"\"", "\"\""),
        ("[[],[[]],{}]", "[[],[[]],{}]"),
        ("\"caf\u{e9} \u{1f600}\"", "\"caf\u{e9} \u{1f600}\""),
    ];
    let mut body = String::from("import std.json as json\n\nfn main() {\n");
    for (input, _) in cases {
        body.push_str(&format!("    println(json.to_text(json.parse({}).unwrap()).unwrap())\n", mote_str(input)));
    }
    body.push_str("}\n");
    let expected: Vec<&str> = cases.iter().map(|(_, out)| *out).collect();
    assert_eq!(run("accept", &body), expected.join("\n"));
}

#[test]
fn malformed_documents_are_errors() {
    let bad: &[&str] = &[
        "", " ", "[", "]", "{", "[1,]", "[,1]", "[1 2]", "{\"a\":1,}", "{a:1}", "{'a':1}", "{\"a\" 1}", "{\"a\":}", "{\"a\"}",
        "[01]", "[1.]", "[.5]", "[-]", "[1e]", "[1e+]", "[+1]", "[--1]", "NaN", "[Infinity]", "[-Infinity]", "nul", "tru", "falsey",
        "\"abc", "\"a\nb\"", "\"a\tb\"", "\"\\x\"", "\"\\u12\"", "\"\\u12g4\"", "\"\\ud800\"", "\"\\udc00\"", "\"\\ud800\\u0041\"",
        "\"\\ud800x\"", "[1] x", "{}{}", "// c\n[]", "/* c */ []", "\u{feff}[]", "[1e999]", "\"unterminated\\", "[\"a\" \"b\"]",
        "{\"a\":1 \"b\":2}", "[}", "{]", "'single'", "0x10", "1.5.5", "--1", "[1,,2]",
    ];
    let mut body = String::from("import std.json as json\n\nfn main() {\n    var accepted = 0\n");
    for text in bad {
        body.push_str(&format!(
            "    if json.parse({}).is_ok() {{\n        accepted = accepted + 1\n        println({})\n    }}\n",
            mote_str(text),
            mote_str(text)
        ));
    }
    body.push_str("    println(accepted)\n}\n");
    assert_eq!(run("reject", &body), "0");
}

#[test]
fn errors_name_the_line_and_column_of_the_fault() {
    let body = r#"import std.json as json

fn message(text: String) -> String {
    match json.parse(text) {
        Ok(v) => { return "accepted" }
        Err(e) => {
            let er: Error = e
            return er.message
        }
    }
}

fn main() {
    println(message("[1,\n  x]"))
    println(message("{\"a\":1,}"))
    println(message("[1] x"))
    println(message(""))
    println(message("[01]"))
    println(message("\"abc"))
    println(message("[1,2\n\n"))
    println(message("{\"a\":1\n \"b\":2}"))
}
"#;
    assert_eq!(
        run("position", body),
        "unexpected character at line 2 column 3\n\
expected a string key at line 1 column 8\n\
unexpected text after the document at line 1 column 5\n\
unexpected end of text at line 1 column 1\n\
a number has a leading zero at line 1 column 3\n\
unterminated string at line 1 column 5\n\
unterminated array at line 3 column 1\n\
expected ',' or '}' at line 2 column 2"
    );
}

#[test]
fn nesting_is_limited_and_large_numbers_fall_back_to_float() {
    let body = r#"import std.json as json
import std.string as string

fn nested(n: Int) -> String {
    return "[".repeat(n) + "]".repeat(n)
}

fn main() {
    println(json.parse(nested(256)).is_ok())
    println(json.parse(nested(257)).is_err())
    let big = json.parse("12345678901234567890").unwrap()
    println(json.as_int(big).is_none())
    println(json.as_float(big).unwrap() > 1.2e19)
    let edge = json.parse("[9223372036854775807, 9223372036854775808, -9223372036854775809]").unwrap()
    println(json.as_int(json.at(edge, 0).unwrap()).is_some())
    println(json.as_int(json.at(edge, 1).unwrap()).is_none())
    println(json.as_int(json.at(edge, 2).unwrap()).is_none())
}
"#;
    assert_eq!(run("limits", body), "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue");
}

#[test]
fn pretty_output_parses_back_to_the_same_tree() {
    let body = format!(
        r#"{TREE}
fn main() {{
    let pretty = json.pretty(sample(), 4).unwrap()
    let again = json.parse(pretty).unwrap()
    println(json.to_text(again).unwrap() == json.to_text(sample()).unwrap())
    println(json.as_int(json.get(again, "count").unwrap()).unwrap())
}}
"#
    );
    assert_eq!(run("pretty_round_trip", &body), "true\n3");
}

/// A Mote string literal for `text`, any control byte written as `\u{..}`.
fn mote_lit(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if (c as u32) < 32 => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// What `to_text` must answer for a string: the escapes JSON requires and nothing else.
fn quoted(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 32 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[test]
fn every_control_byte_and_long_runs_survive_a_round_trip() {
    let run_of = |c: char, n: usize| c.to_string().repeat(n);
    let mut cases: Vec<String> = vec![String::new(), run_of('a', 300), format!("{}\"{}", run_of('a', 300), run_of('b', 300))];
    for byte in 0u8..32 {
        cases.push(format!("x{}y", byte as char));
        cases.push(format!("{}{}{}", run_of('p', 70), byte as char, run_of('q', 70)));
    }
    cases.push(format!("{}\\{}", run_of('é', 200), run_of('😀', 50)));
    cases.push("\"\"\\\\\n\n\t".to_string());
    cases.push(format!("{}\u{7f}\u{80}\u{ff}", run_of('z', 5)));
    let mut body = String::from("import std.json as json\n\nfn main() {\n");
    for text in &cases {
        body.push_str(&format!(
            "    let s = {}\n    let t = json.to_text(Json.Str(s)).unwrap()\n    println(t)\n    println(json.as_str(json.parse(t).unwrap()).unwrap() == s)\n",
            mote_lit(text)
        ));
    }
    body.push_str("}\n");
    let body = body.replace("import std.json as json\n", "import std.json as json\nimport { Json } from std.json\n");
    let expected: Vec<String> = cases.iter().flat_map(|c| [quoted(c), "true".to_string()]).collect();
    assert_eq!(run("control_bytes", &body), expected.join("\n"));
}

#[test]
fn an_escape_after_a_long_plain_run_and_a_fault_after_one_keep_their_place() {
    let body = format!(
        r#"import std.json as json

fn message(text: String) -> String {{
    match json.parse(text) {{
        Ok(v) => {{ return "accepted" }}
        Err(e) => {{
            let er: Error = e
            return er.message
        }}
    }}
}}

fn main() {{
    let a = "a".repeat(300)
    let b = "b".repeat(300)
    let v = json.parse("\"" + a + "\\n" + b + "\\u00e9\"").unwrap()
    println(json.as_str(v).unwrap() == a + "\n" + b + "\u{{e9}}")
    println(message("\"" + a + "\n"))
    println(message("\"" + a))
    println(message("[\"" + a + "\\x\"]"))
    println(message("{{\"" + a + "\":\n \"" + b))
    println(json.to_text(json.parse("[1234567890123.25, -98765432101234, 7]").unwrap()).unwrap())
    println(json.to_text(json.parse("[   \n\t [1 ,\r\n 2 ] ,{{ \"k\" :  \"v\" }} ]  \n").unwrap()).unwrap())
}}
"#
    );
    assert_eq!(
        run("long_runs", &body),
        "true\n\
control character in string at line 1 column 302\n\
unterminated string at line 1 column 302\n\
bad escape at line 1 column 304\n\
unterminated string at line 2 column 303\n\
[1234567890123.25,-98765432101234,7]\n\
[[1,2],{\"k\":\"v\"}]"
    );
}

#[test]
fn string_join_puts_the_separator_between_pieces() {
    let body = r#"import std.string as string

fn main() {
    let none: List<String> = []
    println("[" + string.join(none, ",") + "]")
    println(string.join(["a"], ", "))
    println(string.join(["a", "b", "c"], ", "))
    println(string.join(["", "", ""], "-"))
    println(string.join(["x", "y"], ""))
    println(string.join(["é", "日本", "😀"], "·"))
    var big: List<String> = []
    for i in 0..5000 {
        big.push("p")
    }
    println(string.join(big, "ab").len())
}
"#;
    assert_eq!(run("join", body), "[]\na\na, b, c\n--\nxy\né·日本·😀\n14998");
}
