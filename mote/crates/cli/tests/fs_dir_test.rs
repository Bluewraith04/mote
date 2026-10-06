//! `std.sys.fs` directories and metadata through `mote run` on a real directory.

use std::process::Command;

fn run(name: &str, body: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_fsdir_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let work = dir.join("work");
    let source = format!(
        "import std.sys.fs\nimport std.sys.io\nimport {{ Stat }} from std.sys.fs\n\nfn join(a: String, b: String) -> String {{ return a + \"/\" + b }}\n\nlet root = \"{}\"\n{body}",
        work.display()
    );
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn create_write_list_stat_rename_and_remove() {
    let body = r#"
fs.create_dir_all(join(root, "a/b")).unwrap()
io.write_file(join(root, "one.txt"), "hello")
io.write_file(join(root, "a/two.txt"), "")
println(fs.list_dir(root).unwrap())
println(fs.list_dir(join(root, "a")).unwrap())
let s: Stat = fs.stat(join(root, "one.txt")).unwrap()
println(s.size)
println(s.is_file())
let d: Stat = fs.stat(root).unwrap()
println(d.is_dir())
fs.rename(join(root, "one.txt"), join(root, "uno.txt")).unwrap()
println(fs.exists(join(root, "one.txt")))
println(fs.exists(join(root, "uno.txt")))
fs.remove_file(join(root, "uno.txt")).unwrap()
println(fs.remove_dir(join(root, "a")).is_err())
fs.remove_dir_all(root).unwrap()
println(fs.exists(root))
"#;
    assert_eq!(run("tree", body), "[\"a\", \"one.txt\"]\n[\"b\", \"two.txt\"]\n5\ntrue\ntrue\nfalse\ntrue\ntrue\nfalse");
}

#[test]
fn errors_carry_their_kind() {
    let body = r#"
println(fs.list_dir(join(root, "absent")).is_err())
fs.create_dir_all(root).unwrap()
println(fs.create_dir(root).is_err())
println(fs.create_dir_all(root).is_ok())
println(fs.stat(join(root, "absent")).is_err())
fs.remove_dir_all(root).unwrap()
"#;
    assert_eq!(run("errors", body), "true\ntrue\ntrue\ntrue");
}
