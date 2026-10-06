//! `[package] mote = "X"`: the oldest toolchain that builds the package.

use std::process::Command;

fn run_package(mote_line: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_pkg_version_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("mote.toml"), format!("[package]\nname = \"demo\"\nversion = \"0.1.0\"\n{mote_line}\n")).unwrap();
    std::fs::write(dir.join("src/main.mote"), "fn main() { println(\"ran\") }\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").current_dir(&dir).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

#[test]
fn a_package_without_a_mote_version_builds() {
    let (ok, text) = run_package("", "none");
    assert!(ok && text.trim() == "ran", "got: {text}");
}

#[test]
fn a_package_that_needs_an_older_mote_builds() {
    let (ok, text) = run_package("mote = \"0.1.0\"", "older");
    assert!(ok && text.trim() == "ran", "got: {text}");
}

#[test]
fn a_package_that_needs_a_newer_mote_is_refused_with_both_versions() {
    let (ok, text) = run_package("mote = \"99.0.0\"", "newer");
    assert!(!ok && text.contains("package `demo` needs mote 99.0.0 or newer; this is mote 0."), "got: {text}");
}

#[test]
fn init_writes_the_current_version() {
    let dir = std::env::temp_dir().join(format!("mote_pkg_version_init_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).args(["init", "demo"]).current_dir(&dir).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let toml = std::fs::read_to_string(dir.join("mote.toml")).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(toml.contains(&format!("mote = \"{}\"", env!("CARGO_PKG_VERSION"))), "got: {toml}");
}
