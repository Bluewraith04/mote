//! The tree-sitter grammar parses the repository, and the Zed extension is complete.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn tree_sitter_installed() -> bool {
    Command::new("tree-sitter").arg("--version").output().is_ok()
}

#[test]
fn grammar_corpus() {
    if !tree_sitter_installed() {
        eprintln!("skipped: tree-sitter is not installed");
        return;
    }
    let out = Command::new("tree-sitter").args(["test", "-p", "editors/tree-sitter-mote"]).current_dir(root()).output().unwrap();
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}

#[test]
fn grammar_parses_the_repository() {
    if !tree_sitter_installed() {
        eprintln!("skipped: tree-sitter is not installed");
        return;
    }
    let out = Command::new(root().join("tools/check-grammar.sh")).current_dir(root()).output().unwrap();
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}

#[test]
fn zed_extension_queries_compile() {
    if !tree_sitter_installed() {
        eprintln!("skipped: tree-sitter is not installed");
        return;
    }
    let out = Command::new(root().join("tools/check-zed.sh")).current_dir(root()).output().unwrap();
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}
