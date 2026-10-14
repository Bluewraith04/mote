//! `std.fs` directories and metadata through `mote run` on a real directory.

use std::process::Command;

fn run(name: &str, body: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_fsdir_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let work = dir.join("work");
    let source = format!(
        "import std.fs\nimport std.io\nimport {{ Stat }} from std.fs\n\nfn join(a: String, b: String) -> String {{ return a + \"/\" + b }}\n\nlet root = \"{}\"\n{body}",
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

#[cfg(unix)]
#[test]
fn canonicalize_resolves_dots_and_symlinks() {
    let dir = std::env::temp_dir().join(format!("mote_fsdir_canon_{}", std::process::id()));
    let real = dir.join("real");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(real.join("a.txt"), "x").unwrap();
    std::os::unix::fs::symlink(real.join("a.txt"), dir.join("link.txt")).unwrap();
    let source = format!(
        "import std.fs\n\nlet root = \"{}\"\nprintln(fs.canonicalize(root + \"/link.txt\").unwrap())\nprintln(fs.canonicalize(root + \"/real/../real/a.txt\").unwrap())\nprintln(fs.canonicalize(root + \"/absent\").is_err())\n",
        dir.display()
    );
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    let expected = std::fs::canonicalize(real.join("a.txt")).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_eq!(text, format!("{0}\n{0}\ntrue", expected.display()));
}

#[cfg(unix)]
#[test]
fn lstat_does_not_follow_a_symlink_and_read_link_says_where_it_points() {
    let dir = std::env::temp_dir().join(format!("mote_fsdir_lstat_{}", std::process::id()));
    let real = dir.join("real");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(real.join("a.txt"), "hello").unwrap();
    std::os::unix::fs::symlink("a.txt", real.join("file_link")).unwrap();
    std::os::unix::fs::symlink(&real, dir.join("dir_link")).unwrap();
    std::os::unix::fs::symlink("absent", dir.join("broken")).unwrap();
    let source = format!(
        "import std.fs\n\nlet root = \"{}\"\n{}",
        dir.display(),
        r#"
let f = fs.lstat(root + "/real/a.txt").unwrap()
println("${f.is_file()} ${f.is_symlink()} ${f.size}")
let l = fs.lstat(root + "/real/file_link").unwrap()
println("${l.is_file()} ${l.is_symlink()} ${l.kind}")
println(fs.stat(root + "/real/file_link").unwrap().is_file())
let d = fs.lstat(root + "/dir_link").unwrap()
println("${d.is_dir()} ${d.is_symlink()}")
println(fs.stat(root + "/dir_link").unwrap().is_dir())
println(fs.lstat(root + "/broken").unwrap().is_symlink())
println(fs.stat(root + "/broken").is_err())
println(fs.read_link(root + "/real/file_link").unwrap())
println(fs.read_link(root + "/real/a.txt").is_err())
println(fs.lstat(root + "/absent").is_err())
"#
    );
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_eq!(text, "true false 5\nfalse true 3\ntrue\nfalse true\ntrue\ntrue\ntrue\na.txt\ntrue\ntrue");
}

#[cfg(unix)]
#[test]
fn read_dir_answers_each_entry_with_its_lstat() {
    let dir = std::env::temp_dir().join(format!("mote_fsdir_readdir_{}", std::process::id()));
    let work = dir.join("work");
    std::fs::create_dir_all(work.join("sub")).unwrap();
    std::fs::write(work.join("b.txt"), "hello").unwrap();
    std::fs::write(work.join("a.txt"), "").unwrap();
    std::os::unix::fs::symlink("b.txt", work.join("link")).unwrap();
    let source = format!(
        "import std.fs\n\nlet root = \"{}\"\n{}",
        work.display(),
        r#"
for e in fs.read_dir(root).unwrap() {
    println("${e.name} ${e.kind} ${e.size} ${e.is_dir()} ${e.is_symlink()} ${e.modified > 1600000000000}")
}
println(fs.read_dir(root + "/sub").unwrap().len())
println(fs.read_dir(root + "/absent").is_err())
println(fs.read_dir(root + "/b.txt").is_err())
"#
    );
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(text.starts_with("a.txt 0 0 false false true\nb.txt 0 5 false false true\nlink 3 5 false true true\nsub 1 "), "{text}");
    assert!(text.ends_with(" true false true\n0\ntrue\ntrue"), "{text}");
}

#[cfg(unix)]
#[test]
fn symlink_set_modified_set_mode_and_the_stat_fields() {
    let dir = std::env::temp_dir().join(format!("mote_fsdir_links_{}", std::process::id()));
    let work = dir.join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("a.txt"), "hello").unwrap();
    std::fs::hard_link(work.join("a.txt"), work.join("b.txt")).unwrap();
    std::fs::write(work.join("c.txt"), "x").unwrap();
    let source = format!(
        "import std.fs\n\nlet root = \"{}\"\n{}",
        work.display(),
        r#"
fs.symlink("a.txt", root + "/link").unwrap()
fs.symlink("absent", root + "/dangling").unwrap()
println(fs.symlink("a.txt", root + "/link").is_err())
println(fs.read_link(root + "/link").unwrap())
println(fs.lstat(root + "/dangling").unwrap().is_symlink())
fs.set_mode(root + "/c.txt", 0o600).unwrap()
fs.set_modified(root + "/c.txt", 1700000000123).unwrap()
let c = fs.stat(root + "/c.txt").unwrap()
println("${c.mode} ${c.modified} ${c.links}")
fs.set_mode(root + "/a.txt", 0o755).unwrap()
let a = fs.stat(root + "/a.txt").unwrap()
let b = fs.stat(root + "/b.txt").unwrap()
println("${a.mode} ${a.links} ${a.inode == b.inode} ${a.device == b.device} ${a.inode == c.inode}")
let viaLink = fs.stat(root + "/link").unwrap()
let linkSelf = fs.lstat(root + "/link").unwrap()
println("${viaLink.inode == a.inode} ${linkSelf.inode == a.inode}")
fs.set_modified(root + "/link", 1700000001000).unwrap()
println(fs.stat(root + "/a.txt").unwrap().modified)
fs.set_modified(root, 1700000002000).unwrap()
println(fs.stat(root).unwrap().modified)
for e in fs.read_dir(root).unwrap() {
    if e.name == "b.txt" {
        println("${e.name} ${e.mode} ${e.links} ${e.inode == a.inode}")
    }
    if e.name == "c.txt" {
        println("${e.name} ${e.mode} ${e.links} ${e.inode == c.inode}")
    }
}
println(fs.set_modified(root + "/absent", 5).is_err())
println(fs.set_mode(root + "/absent", 0o644).is_err())
println(fs.set_modified(root + "/c.txt", -1).is_err())
"#
    );
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let expected = [
        "true",
        "a.txt",
        "true",
        "384 1700000000123 1",
        "493 2 true true false",
        "true false",
        "1700000001000",
        "1700000002000",
        "b.txt 493 2 true",
        "c.txt 384 1 true",
        "true",
        "true",
        "true",
    ]
    .join("\n");
    assert_eq!(text, expected);
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
