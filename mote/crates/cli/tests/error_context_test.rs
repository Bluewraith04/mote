//! `Error.context` and `std.error.context` put what a caller was doing in front of a failure's message.

use std::process::Command;

fn run(source: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_error_context_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    text
}

#[test]
fn a_method_prefixes_the_message_and_keeps_the_kind() {
    let src = "fn main() {
    let e = Error.new(ErrorKind.InvalidData, \"line 3\").context(\"parsing config\")
    println(e.message)
    println(e.kind == ErrorKind.InvalidData)
}
";
    assert_eq!(run(src, "method"), "parsing config: line 3\ntrue\n");
}

#[test]
fn contexts_chain_outward_in() {
    let src = "fn main() {
    let e = Error.new(ErrorKind.NotFound, \"no such file\").context(\"reading config\").context(\"starting up\")
    println(e.message)
}
";
    assert_eq!(run(src, "chain"), "starting up: reading config: no such file\n");
}

#[test]
fn a_function_wraps_the_err_of_a_result_and_passes_an_ok() {
    let src = "import { context } from std.error
import std.fs as fs

fn load(path: String) -> Result<String, Error> {
    let text = context(fs.read_text(path), \"reading ${path}\")?
    return Ok(text)
}

fn main() {
    match load(\"/no/such/file\") {
        Ok(_) => println(\"read\")
        Err(e) => {
            println(e.message.starts_with(\"reading /no/such/file: \"))
            println(e.kind == ErrorKind.NotFound)
        }
    }
    let ok: Result<Int, Error> = context(Ok(4), \"unused\")
    println(ok.unwrap())
}
";
    assert_eq!(run(src, "function"), "true\ntrue\n4\n");
}
