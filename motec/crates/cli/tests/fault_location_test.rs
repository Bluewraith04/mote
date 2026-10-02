//! A runtime fault names its file, line and the node behind it; `--release` prints the message alone.

use std::process::Command;

const PROGRAM: &str = "fn get(l: List<Int>) -> Int {\n    return l.get(9)\n}\n\nfn main() {\n    let l: List<Int> = [1, 2]\n    println(get(l).to_string())\n}\n";

fn run(dir: &std::path::Path, flags: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).args(flags).output().unwrap();
    assert!(!out.status.success());
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn a_fault_prints_its_location_and_release_strips_it() {
    let dir = std::env::temp_dir().join(format!("mote_fault_loc_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), PROGRAM).unwrap();

    let debug = run(&dir, &[]);
    assert!(debug.contains("list index 9 out of bounds"), "{debug}");
    assert!(debug.contains("main.mote:2:12"), "{debug}");
    assert!(debug.contains("2 |     return l.get(9)"), "{debug}");
    assert!(debug.contains("  |            ^^^^^^^^"), "{debug}");

    let release = run(&dir, &["--release"]);
    std::fs::remove_dir_all(&dir).ok();
    assert!(release.contains("list index 9 out of bounds"), "{release}");
    assert!(!release.contains("main.mote") && !release.contains('^'), "{release}");
}

#[test]
fn a_fault_in_an_imported_module_names_that_module() {
    let dir = std::env::temp_dir().join(format!("mote_fault_mod_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("lib.mote"), "pub fn pick(l: List<Int>) -> Int {\n    return l.get(5)\n}\n").unwrap();
    std::fs::write(dir.join("main.mote"), "import { pick } from .lib\n\nfn main() {\n    let l: List<Int> = [1]\n    println(pick(l).to_string())\n}\n").unwrap();
    let err = run(&dir, &[]);
    std::fs::remove_dir_all(&dir).ok();
    assert!(err.contains("lib.mote:2:12"), "{err}");
    assert!(err.contains("return l.get(5)"), "{err}");
}

#[test]
fn a_fault_lists_its_callers_and_folds_recursion() {
    let dir = std::env::temp_dir().join(format!("mote_fault_callers_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = "fn inner(z: Int) -> Int {\n    return 10 / z\n}\n\nfn deep(n: Int) -> Int {\n    if n == 0 { return inner(0) }\n    return deep(n - 1)\n}\n\nfn main() {\n    println(deep(3))\n}\n";
    std::fs::write(dir.join("main.mote"), src).unwrap();
    let debug = run(&dir, &[]);
    let release = run(&dir, &["--release"]);
    std::fs::remove_dir_all(&dir).ok();
    assert!(debug.contains("main.mote:2:12"), "{debug}");
    assert!(debug.contains("= called from ") && debug.contains("main.mote:6:24\n"), "{debug}");
    assert!(debug.contains("main.mote:7:12 (3 times)"), "{debug}");
    assert!(debug.contains("main.mote:11:13"), "{debug}");
    assert!(!release.contains("called from"), "{release}");
}
