//! `mote install` builds a program into the mote home's `bin/`, against a temporary home.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("mote_install_{name}_{}", std::process::id()));
    fs::remove_dir_all(&root).ok();
    fs::create_dir_all(&root).unwrap();
    root
}

fn package(root: &Path, name: &str, entry: &str, source: &str) -> PathBuf {
    let dir = root.join(name);
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("mote.toml"), format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nentry = \"{entry}\"\n")).unwrap();
    fs::write(dir.join(entry), source).unwrap();
    dir
}

fn mote(home: &Path, dir: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).args(args).env("MOTE_HOME", home).current_dir(dir).output().unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn a_program_is_built_into_the_homes_bin_and_runs_from_anywhere() {
    let root = scratch("program");
    let home = root.join("home");
    let app = package(&root, "hello", "src/main.mote", "fn main() {\n    println(\"hello from home\")\n}\n");
    let (ok, text) = mote(&home, &app, &["install"]);
    assert!(ok, "{text}");
    assert!(text.contains("Installed hello 0.1.0"), "{text}");
    let exe = home.join("bin").join(format!("hello{}", std::env::consts::EXE_SUFFIX));
    let out = Command::new(&exe).current_dir(std::env::temp_dir()).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "hello from home\n");

    let (ok, text) = mote(&home, &app, &["install"]);
    assert!(ok, "installing the same version again replaces it: {text}");
    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_directory_argument_installs_that_package() {
    let root = scratch("directory");
    let home = root.join("home");
    let app = package(&root, "tool", "src/main.mote", "fn main() {}\n");
    let (ok, text) = mote(&home, &root, &["install", app.to_str().unwrap()]);
    assert!(ok, "{text}");
    assert!(home.join("bin").join(format!("tool{}", std::env::consts::EXE_SUFFIX)).is_file());
    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_library_is_refused() {
    let root = scratch("library");
    let home = root.join("home");
    let lib = package(&root, "shapes", "src/lib.mote", "pub fn sq(x: Int) -> Int { return x * x }\n");
    let (ok, text) = mote(&home, &lib, &["install"]);
    assert!(!ok);
    assert!(text.contains("shapes is a library; there is no program to install"), "{text}");
    assert!(!home.join("bin").exists());
    fs::remove_dir_all(&root).ok();
}
