//! Std.iter becomes generic (T-preserving functions), plus find/find_index/any/all/flat_map.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_iter_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn filter_preserves_element_type_as_a_typed_list() {
    let src = "import std.iter as iter\nfn even(x: Int) -> Bool { return x % 2 == 0 }\npub fn main() {\n    let xs: List<Int> = iter.range(6)\n    let evens: List<Int> = iter.filter(xs, even)\n    println(iter.sum(evens).to_string())\n}\n";
    let (ok, text) = run(src, "filter_typed");
    assert!(ok && text == "6\n", "{text}");
}

#[test]
fn find_and_find_index_answer_option() {
    let src = "import std.iter as iter\nfn even(x: Int) -> Bool { return x % 2 == 0 }\npub fn main() {\n    let xs = iter.range(5)\n    println(iter.find(xs, even).unwrap().to_string())\n    println(iter.find_index(xs, even).unwrap().to_string())\n    println(iter.find(xs, |x| { return x > 100 }).is_none())\n}\n";
    let (ok, text) = run(src, "find");
    assert!(ok && text == "0\n0\ntrue\n", "{text}");
}

#[test]
fn any_and_all_short_circuit_correctly() {
    let src = "import std.iter as iter\nfn even(x: Int) -> Bool { return x % 2 == 0 }\npub fn main() {\n    let xs = iter.range(5)\n    println(iter.any(xs, even))\n    println(iter.all(xs, even))\n}\n";
    let (ok, text) = run(src, "any_all");
    assert!(ok && text == "true\nfalse\n", "{text}");
}

#[test]
fn flat_map_flattens_one_level() {
    let src = "import std.iter as iter\npub fn main() {\n    let xs = [1, 2, 3]\n    let ys = iter.flat_map(xs, |x| { return [x, x * 10] })\n    println(ys.len().to_string())\n    println(ys.get(3).to_string())\n}\n";
    let (ok, text) = run(src, "flat_map");
    assert!(ok && text == "6\n20\n", "{text}");
}

#[test]
fn enumerate_answers_real_tuples() {
    let src = "import std.iter as iter\npub fn main() {\n    let pairs = iter.enumerate([\"a\", \"b\"])\n    println(pairs.get(1).0.to_string())\n    println(pairs.get(1).1)\n}\n";
    let (ok, text) = run(src, "enumerate");
    assert!(ok && text == "1\nb\n", "{text}");
}

#[test]
fn fold_accumulator_type_comes_from_init_not_f() {
    let src = "import std.iter as iter\nfn add(acc: Int, x: Int) -> Int { return acc + x }\npub fn main() {\n    let total: Int = iter.fold(iter.range(5), 100, add)\n    println(total.to_string())\n}\n";
    let (ok, text) = run(src, "fold");
    assert!(ok && text == "110\n", "{text}");
}
