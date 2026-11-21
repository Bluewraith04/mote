use std::fs;
use std::path::{Path, PathBuf};

use mote_packages::testing::Scratch;

use crate::cli::{execute_at, Outcome, FAILED, OK};

fn run(line: &str, dir: &Path, home: &Path) -> Outcome {
    let words: Vec<String> = line.split_whitespace().map(str::to_string).collect();
    execute_at(&words, dir, home)
}

/// A library `gadget` with one function, packed into `target/gadget-0.1.0.zip` of its directory.
fn packed_gadget(scratch: &Scratch, home: &Path) -> PathBuf {
    let dir = scratch.package("gadget", "gadget", &[], &[("lib.mote", "pub fn hello() -> String {\n    return \"hello from gadget\"\n}\n")]);
    assert_eq!(run("package", &dir, home).code, OK);
    dir.join("target/gadget-0.1.0.zip")
}

/// An app that prints what `gadget` says.
fn app(scratch: &Scratch) -> PathBuf {
    scratch.package("app", "app", &[], &[("main.mote", "import { hello } from gadget\n\nfn main() {\n    println(hello())\n}\n")])
}

fn manifest_of(dir: &Path) -> String {
    fs::read_to_string(dir.join("mote.toml")).unwrap()
}

#[test]
fn cli_package_writes_the_zip_below_target_and_says_what_it_holds() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    let dir = scratch.package("gadget", "gadget", &[], &[("lib.mote", "a"), ("util.mote", "b")]);
    let outcome = run("package", &dir, &home);
    assert_eq!(outcome.code, OK);
    let zip = dir.join("target/gadget-0.1.0.zip");
    assert!(outcome.stdout.starts_with(&format!("wrote {} (3 files, ", zip.display())) && outcome.stdout.ends_with(" bytes)\n"), "{}", outcome.stdout);
    assert!(zip.is_file());
}

#[test]
fn cli_package_of_a_directory_without_a_manifest_is_an_error() {
    let scratch = Scratch::new();
    scratch.write("none/src/lib.mote", "");
    let outcome = run("package none", &scratch.dir, &scratch.path("home"));
    assert_eq!(outcome.code, FAILED);
    assert!(outcome.stderr.starts_with("error: no mote.toml in "), "{}", outcome.stderr);
}

#[test]
fn cli_add_copies_a_zip_into_deps_writes_the_entry_and_the_app_runs_it() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    let zip = packed_gadget(&scratch, &home);
    let app = app(&scratch);
    let outcome = run(&format!("add {}", zip.display()), &app, &home);
    assert_eq!(outcome, Outcome::ok("added gadget 0.1.0 (deps/gadget-0.1.0.zip)\n"));
    assert!(app.join("deps/gadget-0.1.0.zip").is_file());
    assert!(manifest_of(&app).ends_with("[dependencies]\ngadget = { zip = \"deps/gadget-0.1.0.zip\" }\n"), "{}", manifest_of(&app));
    assert_eq!(run("run", &app, &home), Outcome::ok("hello from gadget\n"));
}

#[test]
fn cli_add_a_zip_already_in_the_project_is_not_copied() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    let zip = packed_gadget(&scratch, &home);
    let app = app(&scratch);
    fs::create_dir_all(app.join("vendor")).unwrap();
    fs::copy(&zip, app.join("vendor/gadget.zip")).unwrap();
    assert_eq!(run("add vendor/gadget.zip", &app, &home), Outcome::ok("added gadget 0.1.0 (vendor/gadget.zip)\n"));
    assert!(!app.join("deps").exists());
    assert!(manifest_of(&app).contains("gadget = { zip = \"vendor/gadget.zip\" }"));
}

#[test]
fn cli_add_replaces_the_entry_of_the_same_name() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    let zip = packed_gadget(&scratch, &home);
    let app = app(&scratch);
    run(&format!("add {}", zip.display()), &app, &home);
    let newer = scratch.package("newer", "gadget", &[], &[("lib.mote", "pub fn hello() -> String {\n    return \"newer gadget\"\n}\n")]);
    fs::write(newer.join("mote.toml"), "[package]\nname = \"gadget\"\nversion = \"0.2.0\"\n").unwrap();
    assert_eq!(run("package", &newer, &home).code, OK);
    let outcome = run(&format!("add {}", newer.join("target/gadget-0.2.0.zip").display()), &app, &home);
    assert_eq!(outcome, Outcome::ok("added gadget 0.2.0 (deps/gadget-0.2.0.zip)\n"));
    assert_eq!(manifest_of(&app).matches("gadget = ").count(), 1);
    assert!(manifest_of(&app).contains("gadget = { zip = \"deps/gadget-0.2.0.zip\" }"));
    assert_eq!(run("run", &app, &home), Outcome::ok("newer gadget\n"));
}

#[test]
fn cli_add_a_directory_writes_a_path_entry_relative_to_the_project() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    scratch.package("gadget", "gadget", &[], &[("lib.mote", "pub fn hello() -> String {\n    return \"from a directory\"\n}\n")]);
    let app = app(&scratch);
    assert_eq!(run("add ../gadget", &app, &home), Outcome::ok("added gadget 0.1.0 (../gadget)\n"));
    assert!(manifest_of(&app).contains("gadget = { path = \"../gadget\" }"));
    assert_eq!(run("run", &app, &home), Outcome::ok("from a directory\n"));
}

#[test]
fn cli_add_keeps_the_rest_of_the_manifest_and_adds_after_the_last_entry() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    let zip = packed_gadget(&scratch, &home);
    let app = app(&scratch);
    scratch.package("wire", "wire", &[], &[("lib.mote", "")]);
    fs::write(app.join("mote.toml"), "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nwire = { path = \"../wire\" }\n\n").unwrap();
    run(&format!("add {}", zip.display()), &app, &home);
    assert_eq!(manifest_of(&app), "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nwire = { path = \"../wire\" }\ngadget = { zip = \"deps/gadget-0.1.0.zip\" }\n\n");
    let bare = scratch.package("bare", "bare", &[], &[("main.mote", "")]);
    fs::write(bare.join("mote.toml"), "[package]\nname = \"bare\"\nversion = \"0.1.0\"\n").unwrap();
    run(&format!("add {}", zip.display()), &bare, &home);
    assert_eq!(manifest_of(&bare), "[package]\nname = \"bare\"\nversion = \"0.1.0\"\n\n[dependencies]\ngadget = { zip = \"deps/gadget-0.1.0.zip\" }\n");
}

#[test]
fn cli_add_names_the_dependencies_of_the_package_that_the_project_lacks() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    scratch.package("wire", "wire", &[], &[("lib.mote", "")]);
    let gadget = scratch.package("gadget", "gadget", &[("wire", "../wire")], &[("lib.mote", "")]);
    run("package", &gadget, &home);
    let app = app(&scratch);
    let outcome = run(&format!("add {}", gadget.join("target/gadget-0.1.0.zip").display()), &app, &home);
    assert_eq!(outcome, Outcome::ok("added gadget 0.1.0 (deps/gadget-0.1.0.zip)\ngadget needs wire; add it\n"));
    assert_eq!(run("add ../wire", &app, &home), Outcome::ok("added wire 0.1.0 (../wire)\n"));
}

#[test]
fn cli_add_refuses_what_is_not_a_package() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    let app = app(&scratch);
    scratch.write("notes.txt", "x");
    let not_either = run("add ../notes.txt", &app, &home);
    assert_eq!((not_either.code, not_either.stderr.ends_with("notes.txt is not a zip file or a package directory\n")), (FAILED, true), "{}", not_either.stderr);
    scratch.write("bad.zip", "not a zip");
    let bad = run("add ../bad.zip", &app, &home);
    assert!(bad.stderr.contains("bad.zip: not a zip file"), "{}", bad.stderr);
    assert_eq!(run("add .", &app, &home).stderr, "error: `app` is this package\n");
    let std_named = scratch.package("std_like", "std", &[], &[]);
    assert!(run(&format!("add {}", std_named.display()), &app, &home).stderr.contains("`std` is not a package name"));
    assert!(!manifest_of(&app).contains("gadget"));
}

#[test]
fn cli_remove_deletes_the_entry_and_leaves_the_zip() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    let zip = packed_gadget(&scratch, &home);
    let app = app(&scratch);
    run(&format!("add {}", zip.display()), &app, &home);
    assert_eq!(run("remove gadget", &app, &home), Outcome::ok("removed gadget; deps/gadget-0.1.0.zip is still there\n"));
    assert!(app.join("deps/gadget-0.1.0.zip").is_file());
    assert!(!manifest_of(&app).contains("gadget"));
    assert_eq!(run("remove gadget", &app, &home).stderr, "error: `gadget` is not a dependency\n");
    run("add ../gadget", &app, &home);
    assert_eq!(run("remove gadget", &app, &home), Outcome::ok("removed gadget\n"));
}

#[test]
fn cli_install_links_a_package_directory_into_the_bin_of_the_home() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    let tool = scratch.package("tool", "tool", &[], &[("main.mote", "fn main() {\n    println(\"from the tool\")\n}\n")]);
    let outcome = run("install", &tool, &home);
    assert_eq!(outcome.code, OK, "{}", outcome.stderr);
    let exe = home.join("bin/tool");
    assert_eq!(outcome.stdout, format!("installed {}\n", exe.display()));
    let ran = std::process::Command::new(&exe).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "from the tool\n");
}

#[test]
fn cli_install_of_a_zip_builds_it_from_the_cache() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    let tool = scratch.package("tool", "tool", &[], &[("main.mote", "fn main() {\n    println(\"packed tool\")\n}\n")]);
    assert_eq!(run("package", &tool, &home).code, OK);
    let outcome = run("install tool/target/tool-0.1.0.zip", &scratch.dir, &home);
    assert_eq!(outcome.code, OK, "{}", outcome.stderr);
    let ran = std::process::Command::new(home.join("bin/tool")).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "packed tool\n");
}

#[test]
fn cli_install_of_a_library_is_an_error_and_writes_nothing() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    let lib = scratch.package("lib", "lib", &[], &[("lib.mote", "pub fn f() {}\n")]);
    let outcome = run("install", &lib, &home);
    assert_eq!(outcome.code, FAILED);
    assert!(!home.join("bin/lib").exists());
}
