//! `std.iter.sort` and `sort_by` are stable merge sorts that leave the input alone and handle a long list.

use std::process::Command;

fn run(name: &str, source: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn sorts_are_stable_leave_the_input_and_take_a_long_list() {
    let source = r#"import std.iter as iter

fn main() {
    let xs = [5, 3, 9, 1, 3, 7]
    println(iter.sort(xs))
    println(xs)
    println(iter.sort([1]))
    let none: List<Int> = []
    println(iter.sort(none))
    let pairs = [(2, "a"), (1, "b"), (2, "c"), (1, "d"), (0, "e"), (2, "f")]
    let by_key = iter.sort_by(pairs, |a, b| a.0 < b.0)
    var tags = ""
    for p in by_key { tags = tags + p.1 }
    println(tags)
    var big: List<Int> = []
    var seed = 12345
    var i = 0
    while i < 20000 {
        seed = (seed * 1103515245 + 12345) % 2147483648
        big.push(seed % 1000)
        i += 1
    }
    let sorted = iter.sort(big)
    var ordered = true
    var j = 1
    while j < sorted.len() {
        if sorted.get(j - 1) > sorted.get(j) { ordered = false }
        j += 1
    }
    var total_in = 0
    var total_out = 0
    for x in big { total_in += x }
    for x in sorted { total_out += x }
    println("${sorted.len()} ${ordered} ${total_in == total_out}")
    let down = iter.sort_by(sorted, |a, b| a > b)
    println("${down.get(0)} ${down.get(down.len() - 1)}")
}
"#;
    assert_eq!(run("iter_sort", source), "[1, 3, 3, 5, 7, 9]\n[5, 3, 9, 1, 3, 7]\n[1]\n[]\nebdacf\n20000 true true\n999 0");
}
