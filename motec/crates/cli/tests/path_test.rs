//! `std.sys.path` through `mote run`.

use std::process::Command;

fn run(name: &str, body: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_path_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = format!(
        "import std.sys.path as path\n\nfn show(o: Option<String>) {{\n    match o {{\n        Some(v) => {{ println(v) }}\n        None => {{ println(\"none\") }}\n    }}\n}}\n{body}"
    );
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn join_takes_an_absolute_right_side_and_adds_one_separator() {
    let body = "println(path.join(\"a\", \"b\"))\nprintln(path.join(\"a/\", \"b\"))\nprintln(path.join(\"a\", \"/b\"))\nprintln(path.join(\"\", \"b\"))\n";
    assert_eq!(run("join", body), "a/b\na/b\n/b\nb");
}

#[test]
fn normalize_resolves_dots_lexically() {
    let body = "println(path.normalize(\"a/./b/../c//d\"))\nprintln(path.normalize(\"../../a\"))\nprintln(path.normalize(\"/../a\"))\nprintln(path.normalize(\"a/..\"))\nprintln(path.normalize(\"/\"))\n";
    assert_eq!(run("normalize", body), "a/c/d\n../../a\n/a\n.\n/");
}

#[test]
fn parent_and_file_name_split_the_last_component() {
    let body = "show(path.parent(\"a/b/c\"))\nshow(path.parent(\"/a\"))\nshow(path.parent(\"a\"))\nshow(path.parent(\"/\"))\nshow(path.file_name(\"a/b/\"))\nshow(path.file_name(\"\"))\n";
    assert_eq!(run("parent", body), "a/b\n/\n\nnone\nb\nnone");
}

#[test]
fn extension_stem_and_with_extension_follow_the_last_dot() {
    let body = "show(path.extension(\"a/b.tar.gz\"))\nshow(path.extension(\".bashrc\"))\nshow(path.extension(\"noext\"))\nshow(path.stem(\"a/b.tar.gz\"))\nprintln(path.with_extension(\"a/b.txt\", \"md\"))\nprintln(path.with_extension(\"a/b.txt\", \"\"))\n";
    assert_eq!(run("ext", body), "gz\nnone\nnone\nb.tar\na/b.md\na/b");
}

#[test]
fn components_and_is_absolute() {
    let body = "println(path.components(\"/a//b/\"))\nprintln(path.is_absolute(\"/a\"))\nprintln(path.is_absolute(\"a\"))\n";
    assert_eq!(run("components", body), "[\"/\", \"a\", \"b\"]\ntrue\nfalse");
}
