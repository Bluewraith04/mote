//! `mote test` in a package also runs the `.mote` files in its `tests/` directory.

use std::path::Path;
use std::process::Command;

fn package(name: &str, test: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mote_pkg_tests_{name}_{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("tests")).unwrap();
    std::fs::write(dir.join("mote.toml"), "[package]\nname = \"kit\"\nversion = \"0.1.0\"\nedition = \"2026\"\nentry = \"src/lib.mote\"\n").unwrap();
    std::fs::write(dir.join("src/lib.mote"), "pub fn double(n: Int) -> Int { return n * 2 }\n\ntest \"an inline test\" {\n    assert_eq(double(1), 2)\n}\n").unwrap();
    std::fs::write(dir.join("tests/doubling.mote"), test).unwrap();
    dir
}

fn run_tests(dir: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("test").current_dir(dir).output().unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn the_tests_directory_runs_with_the_tests_of_the_entry() {
    let dir = package("pass", "import ..src.lib as kit\n\ntest \"doubles\" {\n    assert_eq(kit.double(4), 8)\n}\n");
    let (ok, text) = run_tests(&dir);
    std::fs::remove_dir_all(&dir).ok();
    assert!(ok, "{text}");
    assert!(text.contains("doubles") && text.contains("an inline test") && text.contains("2 passed, 0 failed"), "{text}");
}

#[test]
fn a_failing_test_in_the_tests_directory_fails_the_run() {
    let dir = package("fail", "import ..src.lib as kit\n\ntest \"wrong\" {\n    assert_eq(kit.double(4), 9)\n}\n");
    let (ok, text) = run_tests(&dir);
    std::fs::remove_dir_all(&dir).ok();
    assert!(!ok && text.contains("FAIL") && text.contains("wrong"), "{text}");
}
