//! Reads a JSON file, filters and extends its records, orders them, writes the result back pretty-printed and reads it again.

use std::process::Command;

#[test]
fn a_program_transforms_a_json_file_and_writes_it_back() {
    let dir = std::env::temp_dir().join(format!("mote_json_done_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("people.json");
    let output = dir.join("adults.json");
    std::fs::write(
        &input,
        r#"[
  {"name": "Wren", "age": 34, "tags": ["ops", "on-call"]},
  {"name": "Ash", "age": 17, "tags": []},
  {"name": "Sol", "age": 41, "score": 9.5},
  {"name": "Bay", "age": "unknown"},
  {"name": "Cy", "age": 18, "note": "café 😀"}
]"#,
    )
    .unwrap();
    let source = format!(
        r#"import std.sys.io as io
import std.iter as iter
import std.data.json as json
import {{ Json, Member }} from std.data.json

fn age_of(person: Json) -> Int {{
    match json.get(person, "age") {{
        Some(a) => {{
            match json.as_int(a) {{
                Some(n) => {{ return n }}
                None => {{ return -1 }}
            }}
        }}
        None => {{ return -1 }}
    }}
}}

fn name_of(person: Json) -> String {{
    return json.as_str(json.get(person, "name").unwrap()).unwrap()
}}

fn with_adult_flag(person: Json) -> Json {{
    let members: List<Member> = json.as_object(person).unwrap()
    let out: List<Member> = []
    for m in members {{ out.push(m) }}
    out.push(json.member("adult", Json.Bool(true)))
    return Json.Object(out)
}}

fn by_name(a: Json, b: Json) -> Bool {{
    return name_of(a) < name_of(b)
}}

fn main() {{
    let text: String = io.read_file("{input}").unwrap()
    let doc = json.parse(text).unwrap()
    let people: List<Json> = json.as_array(doc).unwrap()
    println("read " + people.len().to_string())
    let adults: List<Json> = []
    for p in people {{
        let person: Json = p
        if age_of(person) >= 18 {{ adults.push(with_adult_flag(person)) }}
    }}
    let ordered = iter.sort_by(adults, by_name)
    io.write_file("{output}", json.pretty(Json.Array(ordered), 2).unwrap()).unwrap()

    let back = json.parse(io.read_file("{output}").unwrap()).unwrap()
    let kept: List<Json> = json.as_array(back).unwrap()
    println("kept " + kept.len().to_string())
    for p in kept {{
        let person: Json = p
        println(name_of(person) + " " + json.to_text(person).unwrap().len().to_string())
    }}
    println(json.as_str(json.get(json.at(back, 0).unwrap(), "note").unwrap()).unwrap())
}}
"#,
        input = input.display(),
        output = output.display()
    );
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    let written = std::fs::read_to_string(&output).unwrap_or_default();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "read 5\nkept 3\nCy 55\nSol 48\nWren 62\ncafé 😀\n");
    let expected = concat!(
        "[\n",
        "  {\n    \"name\": \"Cy\",\n    \"age\": 18,\n    \"note\": \"café 😀\",\n    \"adult\": true\n  },\n",
        "  {\n    \"name\": \"Sol\",\n    \"age\": 41,\n    \"score\": 9.5,\n    \"adult\": true\n  },\n",
        "  {\n    \"name\": \"Wren\",\n    \"age\": 34,\n    \"tags\": [\n      \"ops\",\n      \"on-call\"\n    ],\n    \"adult\": true\n  }\n",
        "]"
    );
    assert_eq!(written, expected);
}
