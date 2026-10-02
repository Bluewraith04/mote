//! Std.stream lazy map/filter/take, composable without an intermediate List.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_stream_lazy_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn map_filter_take_chain_over_a_list_stream() {
    let src = "import std.stream as stream\nimport std.iter as iter\nfn double(x: Int) -> Int { return x * 2 }\nfn is_even(x: Int) -> Bool { return x % 2 == 0 }\npub fn main() {\n    let xs = iter.range(10)\n    let s = stream.take(stream.filter(stream.map(xs.stream(), double), is_even), 3)\n    for v in s {\n        println(v.to_string())\n    }\n}\n";
    let (ok, text) = run(src, "chain");
    assert!(ok && text == "0\n2\n4\n", "{text}");
}

#[test]
fn take_over_an_infinite_stream_terminates() {
    let src = "import std.stream as stream\nfn double(x: Int) -> Int { return x * 2 }\nfn naturals() -> Stream<Int> {\n    var i = 0\n    while true {\n        yield i\n        i = i + 1\n    }\n}\npub fn main() {\n    let s = stream.take(stream.map(naturals(), double), 3)\n    for v in s {\n        println(v.to_string())\n    }\n}\n";
    let (ok, text) = run(src, "infinite");
    assert!(ok && text == "0\n2\n4\n", "{text}");
}

#[test]
fn filter_alone_skips_non_matching_elements() {
    let src = "import std.stream as stream\nimport std.iter as iter\nfn is_even(x: Int) -> Bool { return x % 2 == 0 }\npub fn main() {\n    let xs = iter.range(6)\n    var out: List<Int> = []\n    for v in stream.filter(xs.stream(), is_even) {\n        out.push(v)\n    }\n    println(out.len().to_string())\n}\n";
    let (ok, text) = run(src, "filter_only");
    assert!(ok && text == "3\n", "{text}");
}
