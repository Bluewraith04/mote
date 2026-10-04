//! The packages `compress`, `archive` and `yaml`.

mod common;

use std::process::Command;

fn run(source: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_compress_{}_{}", tag, std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    common::install_native_packages(&dir, &["compress", "archive", "yaml"]);
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).current_dir(&dir).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    text
}

fn lines(source: &str, tag: &str) -> Vec<String> {
    run(source, tag).lines().map(String::from).collect()
}

const COMPRESS_HEAD: &str = r#"import compress as compress

fn same(a: Bytes, b: Bytes) -> Bool {
    if a.len() != b.len() { return false }
    var i = 0
    while i < a.len() {
        if a.get(i) != b.get(i) { return false }
        i = i + 1
    }
    return true
}

fn text(n: Int) -> Bytes {
    var b = Bytes()
    var i = 0
    while i < n {
        b.push(97 + i % 7)
        i = i + 1
    }
    return b
}
"#;

#[test]
fn each_format_round_trips_and_shrinks_repeating_data() {
    let src = format!(
        "{COMPRESS_HEAD}
fn main() {{
    let data = text(5000)
    let g = compress.gzip(data)
    let z = compress.zlib(data)
    let d = compress.deflate(data)
    println(same(compress.gunzip(g).unwrap(), data))
    println(same(compress.unzlib(z).unwrap(), data))
    println(same(compress.inflate(d).unwrap(), data))
    println(g.len() < 200 && z.len() < 200 && d.len() < 200)
    println(g.get(0) == 31 && g.get(1) == 139)
    println(z.get(0) == 120)
    println(compress.gunzip(compress.gzip(Bytes())).unwrap().len())
}}
"
    );
    assert_eq!(lines(&src, "rounds"), ["true", "true", "true", "true", "true", "true", "0"]);
}

#[test]
fn levels_trade_size_for_effort_and_clamp() {
    let src = format!(
        "{COMPRESS_HEAD}
fn main() {{
    let data = text(20000)
    let stored = compress.gzip_level(data, 0)
    let best = compress.gzip_level(data, 9)
    println(stored.len() > data.len())
    println(best.len() < stored.len())
    println(same(compress.gunzip(stored).unwrap(), data))
    println(same(compress.gunzip(compress.gzip_level(data, 99)).unwrap(), data))
    println(same(compress.gunzip(compress.gzip_level(data, -5)).unwrap(), data))
}}
"
    );
    assert_eq!(lines(&src, "levels"), ["true", "true", "true", "true", "true"]);
}

#[test]
fn bad_data_is_an_error() {
    let src = format!(
        "{COMPRESS_HEAD}
fn main() {{
    let junk = \"not compressed\".bytes()
    println(compress.gunzip(junk).is_err())
    println(compress.unzlib(junk).is_err())
    println(compress.inflate(junk).is_err())
    println(compress.gunzip(compress.zlib(junk)).is_err())
    let g = compress.gzip(text(1000))
    println(compress.gunzip(g.slice(0, g.len() - 6)).is_err())
}}
"
    );
    assert_eq!(lines(&src, "bad"), ["true", "true", "true", "true", "true"]);
}

const ARCHIVE_HEAD: &str = r#"import archive as archive
import { Entry } from archive

fn show(entries: List<Entry>) {
    for e in entries {
        println("${e.name} ${e.is_dir()} ${e.data.len()}")
    }
}
"#;

#[test]
fn tar_and_zip_round_trip_files_and_directories() {
    let src = format!(
        "{ARCHIVE_HEAD}
fn main() {{
    let entries = [
        archive.dir(\"docs\"),
        archive.file(\"docs/a.txt\", \"hello\".bytes()),
        archive.file(\"b.bin\", Bytes(300)),
        archive.file(\"empty\", Bytes()),
    ]
    show(archive.unpack_tar(archive.pack_tar(entries).unwrap()).unwrap())
    println(\"--\")
    show(archive.unpack_zip(archive.pack_zip(entries).unwrap()).unwrap())
    let back = archive.unpack_zip(archive.pack_zip(entries).unwrap()).unwrap()
    println(back.get(1).data.decode().unwrap())
}}
"
    );
    let one = ["docs/ true 0", "docs/a.txt false 5", "b.bin false 300", "empty false 0"];
    let mut want: Vec<&str> = one.to_vec();
    want.push("--");
    want.extend(one);
    want.push("hello");
    assert_eq!(lines(&src, "archives"), want);
}

#[test]
fn the_same_entries_pack_to_the_same_bytes() {
    let src = format!(
        "{ARCHIVE_HEAD}
fn main() {{
    let entries = [archive.file(\"a\", \"x\".bytes()), archive.dir(\"d\")]
    let one = archive.pack_tar(entries).unwrap()
    let two = archive.pack_tar(entries).unwrap()
    println(one.len() == two.len() && one.len() % 512 == 0)
    var same = true
    var i = 0
    while i < one.len() {{
        if one.get(i) != two.get(i) {{ same = false }}
        i = i + 1
    }}
    println(same)
}}
"
    );
    assert_eq!(lines(&src, "determinism"), ["true", "true"]);
}

#[test]
fn unsafe_names_are_refused_on_every_path() {
    let src = format!(
        "{ARCHIVE_HEAD}
fn main() {{
    for name in [\"../x\", \"/etc/x\", \"a/../../x\", \"a\\\\b\", \"\", \"C:/x\"] {{
        match archive.pack_zip([archive.file(name, Bytes())]) {{
            Ok(b) => {{ println(\"packed\") }}
            Err(e) => {{ println(e.message) }}
        }}
    }}
    match archive.extract([archive.file(\"a/../../x\", Bytes())], \"out\") {{
        Ok(n) => {{ println(\"written\") }}
        Err(e) => {{ println(e.message) }}
    }}
}}
"
    );
    assert_eq!(
        lines(&src, "unsafe"),
        [
            "unsafe entry name ../x",
            "unsafe entry name /etc/x",
            "unsafe entry name a/../../x",
            "unsafe entry name a\\b",
            "unsafe entry name ",
            "unsafe entry name C:/x",
            "unsafe entry name a/../../x",
        ]
    );
}

#[test]
fn bad_archives_are_errors() {
    let src = format!(
        "{ARCHIVE_HEAD}
fn main() {{
    println(archive.unpack_zip(\"not a zip\".bytes()).is_err())
    println(archive.unpack_tar(\"not a tar\".bytes()).is_err())
    let z = archive.pack_zip([archive.file(\"a\", Bytes(100))]).unwrap()
    println(archive.unpack_zip(z.slice(0, z.len() - 10)).is_err())
}}
"
    );
    assert_eq!(lines(&src, "bad_archives"), ["true", "true", "true"]);
}

#[test]
fn a_directory_tree_packs_and_extracts() {
    let src = format!(
        "{ARCHIVE_HEAD}
import std.sys.fs as fs

fn main() {{
    fs.create_dir_all(\"src/sub\").unwrap()
    fs.write_text(\"src/top.txt\", \"top\").unwrap()
    fs.write_text(\"src/sub/inner.txt\", \"inner\").unwrap()
    let entries = archive.read_dir(\"src\").unwrap()
    show(entries)
    let packed = archive.pack_tar(entries).unwrap()
    archive.extract(archive.unpack_tar(packed).unwrap(), \"copy/deep\").unwrap()
    println(fs.read_text(\"copy/deep/sub/inner.txt\").unwrap())
    println(fs.read_text(\"copy/deep/top.txt\").unwrap())
}}
"
    );
    assert_eq!(lines(&src, "tree"), ["sub/ true 0", "sub/inner.txt false 5", "top.txt false 3", "inner", "top"]);
}

const YAML_HEAD: &str = r#"import yaml as yaml
import std.data.json as json

fn show(text: String) {
    match yaml.parse(text) {
        Ok(j) => {
            match json.to_text(j) {
                Ok(t) => { println(t) }
                Err(e) => { println("json: ${e.message}") }
            }
        }
        Err(e) => { println("error: ${e.message}") }
    }
}
"#;

fn lit(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn yaml_lines(cases: &[&str], tag: &str) -> Vec<String> {
    let calls: String = cases.iter().map(|c| format!("    show({})\n", lit(c))).collect();
    lines(&format!("{YAML_HEAD}\nfn main() {{\n{calls}}}\n"), tag)
}

#[test]
fn yaml_scalars_collections_and_comments_read_as_json() {
    let got = yaml_lines(
        &[
            "name: demo # note\nport: 8080\nratio: 0.5\non: true\nnone: ~\nlist:\n  - a\n  - 2\n  - [x, y]\nmap: {k: v}\n",
            "- 1\n- two\n- 3.5\n",
            "just text\n",
            "text: |\n  line one\n  line two\nfolded: >\n  a\n  b\n",
            "quoted: \"a\\tb\"\nsingle: 'it''s'\nnumber_text: \"42\"\n",
            "",
        ],
        "scalars",
    );
    assert_eq!(
        got,
        [
            "{\"name\":\"demo\",\"port\":8080,\"ratio\":0.5,\"on\":true,\"none\":null,\"list\":[\"a\",2,[\"x\",\"y\"]],\"map\":{\"k\":\"v\"}}",
            "[1,\"two\",3.5]",
            "\"just text\"",
            "{\"text\":\"line one\\nline two\\n\",\"folded\":\"a b\\n\"}",
            "{\"quoted\":\"a\\tb\",\"single\":\"it's\",\"number_text\":\"42\"}",
            "null",
        ]
    );
}

#[test]
fn yaml_anchors_aliases_merges_tags_and_odd_keys() {
    let got = yaml_lines(
        &[
            "base: &b {x: 1, y: 2}\nuse: *b\nmerged:\n  <<: *b\n  y: 3\n",
            "tagged: !custom value\nn: !!int 5\n",
            "1: one\ntrue: yes\n? null\n: nothing\n",
            "inf: .inf\nneg: -.inf\nnan: .nan\n",
        ],
        "anchors",
    );
    assert_eq!(
        got,
        [
            "{\"base\":{\"x\":1,\"y\":2},\"use\":{\"x\":1,\"y\":2},\"merged\":{\"y\":3,\"x\":1}}",
            "{\"tagged\":\"value\",\"n\":5}",
            "{\"1\":\"one\",\"true\":\"yes\",\"null\":\"nothing\"}",
            "{\"inf\":\"inf\",\"neg\":\"-inf\",\"nan\":\"nan\"}",
        ]
    );
}

#[test]
fn yaml_errors_say_where() {
    let got = yaml_lines(&["a: [1, 2\n", "a: 1\n a: 2\n", "- 1\n---\n- 2\n", "a: *missing\n", "? [1]\n: x\n"], "errors");
    assert_eq!(got.len(), 5);
    for line in &got {
        assert!(line.starts_with("error: "), "{line}");
    }
}

#[test]
fn yaml_parse_all_reads_every_document() {
    let src = format!(
        "{YAML_HEAD}
fn main() {{
    let docs = yaml.parse_all(\"a: 1\\n---\\n- x\\n- y\\n---\\nlast\\n\").unwrap()
    println(docs.len())
    for d in docs {{ println(json.to_text(d).unwrap()) }}
    println(yaml.parse_all(\"\").unwrap().len())
}}
"
    );
    assert_eq!(lines(&src, "multi"), ["3", "{\"a\":1}", "[\"x\",\"y\"]", "\"last\"", "0"]);
}

#[test]
fn yaml_writes_and_reads_back() {
    let src = format!(
        "{YAML_HEAD}
fn main() {{
    let j = json.parse(\"{{\\\"b\\\": [1, 2.5, \\\"s\\\", null, true], \\\"a\\\": {{\\\"c\\\": \\\"x: y\\\"}}, \\\"e\\\": [], \\\"n\\\": \\\"123\\\"}}\").unwrap()
    let text = yaml.to_text(j).unwrap()
    print(text)
    println(json.to_text(yaml.parse(text).unwrap()).unwrap() == json.to_text(j).unwrap())
}}
"
    );
    assert_eq!(
        lines(&src, "write"),
        ["b:", "- 1", "- 2.5", "- s", "- null", "- true", "a:", "  c: 'x: y'", "e: []", "n: '123'", "true"]
    );
}

#[test]
fn done_bar_a_site_bundled_and_restored() {
    let got = lines(include_str!("programs_mote/archive_data.mote"), "done_bar");
    assert_eq!(
        got,
        [
            "Notes: 2 pages, draft false",
            "  index: welcome",
            "  about: about this site",
            "Notes: 2 pages, draft false",
            "  index: welcome",
            "  about: about this site",
            "4",
            "title: Notes",
            "pages:",
            "- index",
            "- about",
            "draft: false",
        ]
    );
}
