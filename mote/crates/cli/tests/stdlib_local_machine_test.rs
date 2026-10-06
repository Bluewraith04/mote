//! Builds a directory, writes a script, runs it as a child, streams its output back through a file handle and cleans up.
#![cfg(unix)]

use std::process::Command;

#[test]
fn a_program_drives_the_local_machine() {
    let dir = std::env::temp_dir().join(format!("mote_phase_d_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let work = dir.join("work");
    let source = format!(
        r#"import std.sys.fs
import std.sys.io
import std.sys.process as process
import {{ File, Stat }} from std.sys.fs
import {{ Output }} from std.sys.process

fn join(a: String, b: String) -> String {{ return a + "/" + b }}

fn main() {{
    let root = "{}"
    let bin = join(root, "bin")
    fs.create_dir_all(bin).unwrap()

    let script = join(bin, "count.sh")
    let w: File = fs.create(script).unwrap()
    w.write_text("for w in one two three; do echo $w; done\n").unwrap()
    w.close().unwrap()

    let o: Output = process.run("sh", [script]).unwrap()
    let report = join(bin, "count.out")
    io.write_file_bytes(report, o.stdout).unwrap()

    let st: Stat = fs.stat(report).unwrap()
    println(st.size)
    let r: File = fs.open(report).unwrap()
    var n = 0
    for item in r.lines() {{
        match item {{
            Ok(line) => {{ n = n + 1 }}
            _ => {{ println("read failed") }}
        }}
    }}
    r.close().unwrap()
    println(n)
    println(fs.list_dir(bin).unwrap())
    fs.remove_dir_all(root).unwrap()
    println(fs.exists(root))
}}
"#,
        work.display()
    );
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "14\n3\n[\"count.out\", \"count.sh\"]\nfalse");
}
