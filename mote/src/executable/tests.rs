use std::process::Command;

use mote_packages::testing::Scratch;

use super::*;
use crate::pipeline::{Options, Target};

/// The program linked into an executable in a scratch directory, and the directory.
fn linked(source: &str) -> (Scratch, PathBuf) {
    let scratch = Scratch::new();
    let dir = scratch.package("app", "app", &[], &[("main.mote", source)]);
    let mut pipeline = Pipeline::build(&dir, Target::Run, &Options::default()).unwrap_or_else(|failure| panic!("{failure}"));
    let out = scratch.dir.join(format!("app-exe{}", std::env::consts::EXE_SUFFIX));
    link(&mut pipeline, &out).unwrap_or_else(|failure| panic!("{failure}"));
    (scratch, out)
}

#[test]
fn executable_the_link_uses_cc_from_the_environment_else_cc() {
    assert_eq!(compiler_of(Some("clang".into())), "clang");
    assert_eq!(compiler_of(Some(String::new())), "cc");
    assert_eq!(compiler_of(None), "cc");
}

#[test]
fn executable_the_link_asks_for_mold_or_lld_when_one_is_on_the_path() {
    let scratch = Scratch::new();
    let empty = scratch.dir.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert_eq!(linker_of(empty.as_os_str()), None);
    scratch.write("bin/ld.lld", "");
    assert_eq!(linker_of(scratch.dir.join("bin").as_os_str()), Some("lld"));
    scratch.write("bin/mold", "");
    assert_eq!(linker_of(scratch.dir.join("bin").as_os_str()), Some("mold"));
}

#[test]
fn executable_a_program_prints_and_returns_with_code_0() {
    let (_scratch, exe) = linked("fn main() {\n    println(\"hello\")\n    println(1 + 2)\n}\n");
    let ran = Command::new(&exe).output().unwrap();
    assert_eq!((String::from_utf8_lossy(&ran.stdout).as_ref(), ran.status.code()), ("hello\n3\n", Some(0)));
}
