//! `std.sys.fs` one-shot wrappers: `read_text`, `write_text`, `read_bytes` and `write_bytes`.

use std::process::Command;

fn run(source_fn: impl FnOnce(&str) -> String, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_fs_oneshot_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let data_path = dir.join("data.txt");
    let source = source_fn(&data_path.to_string_lossy());
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn write_text_then_read_text_round_trips() {
    let (ok, text) = run(
        |p| format!("import std.sys.fs as fs\npub fn main() {{\n    fs.write_text(\"{p}\", \"hello world\")!\n    println(fs.read_text(\"{p}\")!)\n}}\n"),
        "text",
    );
    assert!(ok && text == "hello world\n", "{text}");
}

#[test]
fn write_bytes_then_read_bytes_round_trips() {
    let (ok, text) = run(
        |p| format!("import std.sys.fs as fs\npub fn main() {{\n    let b = \"abc\".bytes()\n    fs.write_bytes(\"{p}\", b)!\n    let back = fs.read_bytes(\"{p}\")!\n    println(back.len().to_string())\n    println(back.decode()!)\n}}\n"),
        "bytes",
    );
    assert!(ok && text == "3\nabc\n", "{text}");
}

#[test]
fn read_text_of_a_missing_file_answers_err_not_a_panic() {
    let (ok, text) = run(
        |p| format!("import std.sys.fs as fs\npub fn main() {{\n    match fs.read_text(\"{p}\") {{\n        Ok(_) => {{ println(\"unexpected ok\") }}\n        Err(_) => {{ println(\"got err\") }}\n    }}\n}}\n"),
        "missing",
    );
    assert!(ok && text == "got err\n", "{text}");
}

#[test]
fn write_text_closes_the_handle_so_a_second_open_can_read_it_back() {
    let (ok, text) = run(
        |p| format!("import std.sys.fs as fs\npub fn main() {{\n    fs.write_text(\"{p}\", \"first\")!\n    fs.write_text(\"{p}\", \"second\")!\n    println(fs.read_text(\"{p}\")!)\n}}\n"),
        "reopen",
    );
    assert!(ok && text == "second\n", "{text}");
}
