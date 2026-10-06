//! A generator keeps its regions across a `yield` on a stack of its own, and frees them when it ends or is collected.

use std::process::Command;

fn run(source: &str, tag: &str, extra: &[&str]) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_genregions_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.mote");
    std::fs::write(&file, source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(&file).args(extra).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

const ACC: &str = "struct Acc {\n    var total: Int\n    var label: String\n}\n\n";

const CHURN: &str = "fn churn(k: Int) -> Int {\n    var t = 0\n    var i = 0\n    while i < k {\n        let xs = [i, i, i]\n        t += xs.len()\n        i += 1\n    }\n    return t\n}\n\n";

#[test]
fn a_region_lives_across_a_yield() {
    let src = format!("{ACC}fn counter(n: Int) -> Stream<Int> {{\n    var a = Acc {{ total: 0, label: \"run\" }}\n    var i = 0\n    while i < n {{\n        a.total += i\n        yield a.total\n        i += 1\n    }}\n    yield a.total * 1000\n}}\n\nfn main() {{\n    var sum = 0\n    for x in counter(5) {{\n        sum += x\n    }}\n    println(sum)\n}}\n");
    let (ok, text) = run(&src, "across", &[]);
    assert!(ok, "{text}");
    assert!(text.contains("10020"), "{text}");
}

#[test]
fn a_generator_body_is_placed_in_a_region() {
    let src = format!("{ACC}fn show(a: Acc) -> Int {{\n    let b = a\n    return b.total\n}}\n\nfn counter() -> Stream<Int> {{\n    var a = Acc {{ total: 1, label: \"x\" }}\n    yield show(a)\n}}\n\nfn main() {{\n    for x in counter() {{\n        println(x)\n    }}\n}}\n");
    let (ok, text) = run(&src, "placed", &["--mem-stats"]);
    assert!(ok, "{text}");
    assert!(text.contains("mem.regions.scopes = 1"), "{text}");
}

#[test]
fn interleaved_generators_keep_their_own_regions() {
    let src = format!("{ACC}fn numbers(tag: String, n: Int) -> Stream<String> {{\n    var a = Acc {{ total: 0, label: tag }}\n    var i = 0\n    while i < n {{\n        a.total += 1\n        a.label = tag + \"-\" + a.total.to_string()\n        yield a.label\n        i += 1\n    }}\n}}\n\nfn main() {{\n    let g1 = numbers(\"a\", 4)\n    let g2 = numbers(\"b\", 4)\n    var k = 0\n    while k < 4 {{\n        var s1 = \"\"\n        var s2 = \"\"\n        for x in g1 {{\n            s1 = x\n            break\n        }}\n        for y in g2 {{\n            s2 = y\n            break\n        }}\n        println(\"${{s1}} ${{s2}}\")\n        k += 1\n    }}\n}}\n");
    let (ok, text) = run(&src, "interleaved", &[]);
    assert!(ok, "{text}");
    assert_eq!(text.lines().take(4).collect::<Vec<_>>(), ["a-1 b-1", "a-2 b-2", "a-3 b-3", "a-4 b-4"], "{text}");
}

#[test]
fn a_generator_inside_a_generator_keeps_both_regions() {
    let src = format!("{ACC}fn inner(n: Int) -> Stream<Int> {{\n    var a = Acc {{ total: 0, label: \"in\" }}\n    var i = 0\n    while i < n {{\n        a.total += i\n        yield a.total\n        i += 1\n    }}\n}}\n\nfn outer(n: Int) -> Stream<Int> {{\n    var b = Acc {{ total: 100, label: \"out\" }}\n    for x in inner(n) {{\n        b.total += x\n        yield b.total\n    }}\n}}\n\nfn main() {{\n    var t = 0\n    for v in outer(5) {{\n        t += v\n    }}\n    println(t)\n}}\n");
    let (ok, text) = run(&src, "nested", &[]);
    assert!(ok, "{text}");
    assert!(text.contains("535"), "{text}");
}

#[test]
fn a_suspended_generator_gives_back_its_values_after_collections() {
    let src = format!("{ACC}{CHURN}fn holder(tag: Int) -> Stream<String> {{\n    let h = Acc {{ total: tag, label: \"item ${{tag}}\" }}\n    yield \"first\"\n    yield h.label\n}}\n\nfn main() {{\n    let g = holder(7)\n    var first = \"\"\n    for x in g {{\n        first = x\n        break\n    }}\n    let junk = churn(300000)\n    var second = \"\"\n    for y in g {{\n        second = y\n        break\n    }}\n    println(\"${{first}} ${{second}}\")\n}}\n");
    let (ok, text) = run(&src, "survives", &["--mem-stats"]);
    assert!(ok, "{text}");
    assert!(text.contains("first item 7"), "{text}");
    assert!(!text.contains("mem.gc.collections = 0"), "the churn must have collected:\n{text}");
}

#[test]
fn abandoned_generators_give_their_regions_back() {
    let src = format!("{ACC}fn holder(n: Int) -> Stream<Int> {{\n    var a = Acc {{ total: n, label: \"x\" }}\n    yield a.total\n    yield a.total + 1\n}}\n\nfn main() {{\n    var sum = 0\n    var i = 0\n    while i < 200000 {{\n        for x in holder(i) {{\n            sum += x\n            break\n        }}\n        i += 1\n    }}\n    println(sum)\n}}\n");
    let (ok, text) = run(&src, "abandoned", &["--mem-stats"]);
    assert!(ok, "{text}");
    let rss = text.lines().find_map(|l| l.strip_prefix("mem.peak_rss = ")).and_then(|l| l.split(' ').next()).and_then(|n| n.parse::<u64>().ok()).expect(&text);
    assert!(rss < 300 * 1024 * 1024, "200,000 abandoned generators held {rss} bytes:\n{text}");
}
