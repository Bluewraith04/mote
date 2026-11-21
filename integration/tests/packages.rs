//! A package shared as a zip: made by `git archive` or `mote package`, added to a project, run.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use mote::{execute_at, Outcome};
use mote_packages::testing::Scratch;

fn run(line: &str, dir: &Path, home: &Path) -> Outcome {
    let words: Vec<String> = line.split_whitespace().map(str::to_string).collect();
    execute_at(&words, dir, home)
}

/// A library `gadget` whose `hello` says where it came from.
fn gadget(scratch: &Scratch, at: &str) -> PathBuf {
    scratch.package(at, "gadget", &[], &[("lib.mote", "pub fn hello() -> String {\n    return \"hello from gadget\"\n}\n")])
}

fn app(scratch: &Scratch) -> PathBuf {
    scratch.package("app", "app", &[], &[("main.mote", "import { hello } from gadget\n\nfn main() {\n    println(hello())\n}\n")])
}

/// Runs `git` in `dir`; false when git is not installed.
fn git(dir: &Path, args: &[&str]) -> bool {
    let identity = ["-c", "user.name=mote", "-c", "user.email=mote@example.com", "-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false"];
    match Command::new("git").args(identity).args(args).current_dir(dir).output() {
        Ok(output) => {
            assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
            true
        }
        Err(_) => false,
    }
}

/// Commits the package in `dir` to a new repository.
fn commit(dir: &Path) -> bool {
    git(dir, &["init", "-q"]) && git(dir, &["add", "."]) && git(dir, &["commit", "-q", "-m", "gadget"])
}

#[test]
fn packages_a_git_archive_of_a_package_is_added_and_run() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    let source = gadget(&scratch, "gadget");
    if !commit(&source) {
        eprintln!("skipped: git is not installed");
        return;
    }
    let zip = scratch.path("gadget.zip");
    assert!(git(&source, &["archive", "--format=zip", "-o", zip.to_str().unwrap(), "HEAD"]));
    let app = app(&scratch);
    assert_eq!(run("add ../gadget.zip", &app, &home), Outcome::ok("added gadget 0.1.0 (deps/gadget.zip)\n"));
    assert_eq!(run("run", &app, &home), Outcome::ok("hello from gadget\n"));
}

#[test]
fn packages_an_archive_with_one_top_folder_as_a_host_makes_it_is_added_and_run() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    let source = gadget(&scratch, "gadget");
    if !commit(&source) {
        eprintln!("skipped: git is not installed");
        return;
    }
    let zip = scratch.path("gadget-main.zip");
    assert!(git(&source, &["archive", "--format=zip", "--prefix=gadget-main/", "-o", zip.to_str().unwrap(), "HEAD"]));
    let app = app(&scratch);
    assert_eq!(run("add ../gadget-main.zip", &app, &home).code, 0);
    assert_eq!(run("run", &app, &home), Outcome::ok("hello from gadget\n"));
}

#[test]
fn packages_a_package_made_by_mote_package_is_added_and_run() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    let source = gadget(&scratch, "gadget");
    assert_eq!(run("package", &source, &home).code, 0);
    let app = app(&scratch);
    assert_eq!(run("add ../gadget/target/gadget-0.1.0.zip", &app, &home).code, 0);
    assert_eq!(run("run", &app, &home), Outcome::ok("hello from gadget\n"));
    assert_eq!(run("test", &app, &home).code, 0);
}

#[test]
fn packages_the_same_source_in_two_directories_makes_the_same_zip() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    let first = gadget(&scratch, "one/gadget");
    let second = gadget(&scratch, "two/gadget");
    run("package", &first, &home);
    run("package", &second, &home);
    assert_eq!(fs::read(first.join("target/gadget-0.1.0.zip")).unwrap(), fs::read(second.join("target/gadget-0.1.0.zip")).unwrap());
}

#[test]
fn packages_a_zip_with_a_shared_library_keeps_the_lib_beside_the_package() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    let source = gadget(&scratch, "gadget");
    scratch.write("gadget/lib/x86_64-linux/libgadget.so", "not a real library");
    run("package", &source, &home);
    let app = app(&scratch);
    run("add ../gadget/target/gadget-0.1.0.zip", &app, &home);
    let cached: Vec<PathBuf> = fs::read_dir(home.join("cache")).unwrap().map(|entry| entry.unwrap().path()).collect();
    assert_eq!(cached.len(), 1);
    assert_eq!(fs::read_to_string(cached[0].join("lib/x86_64-linux/libgadget.so")).unwrap(), "not a real library");
}
