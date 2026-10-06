//! Context managers — `with`, the `Closeable` trait, and guaranteed close on scope exit.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_with_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn check(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_with_chk_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("check").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn rejected(source: &str, tag: &str, message: &str) {
    let (ok, text) = check(source, tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

const RES: &str = "class Res {\n    label: String\n    pub fn close(self) -> Result<Null, Error> {\n        println(\"closing \" + self.label)\n        return Ok(null)\n    }\n}\n";

#[test]
fn closes_on_fall_through() {
    let src = format!("{RES}pub fn main() {{\n    with r = Res {{ label: \"a\" }} {{\n        println(\"inside\")\n    }}\n    println(\"after\")\n}}\n");
    let (ok, text) = run(&src, "fallthrough");
    assert!(ok && text == "inside\nclosing a\nafter\n", "{text}");
}

#[test]
fn closes_before_an_early_return() {
    let src = format!("{RES}fn f() -> Int {{\n    with r = Res {{ label: \"ret\" }} {{\n        return 42\n    }}\n    return 0\n}}\npub fn main() {{\n    println(f().to_string())\n}}\n");
    let (ok, text) = run(&src, "early_return");
    assert!(ok && text == "closing ret\n42\n", "{text}");
}

#[test]
fn closes_before_continue_and_break() {
    let src = format!(
        "{RES}pub fn main() {{\n    var i = 0\n    while i < 3 {{\n        with r = Res {{ label: i.to_string() }} {{\n            if i == 1 {{\n                i = i + 1\n                continue\n            }}\n            if i == 2 {{\n                break\n            }}\n            println(\"iter \" + i.to_string())\n        }}\n        i = i + 1\n    }}\n    println(\"done\")\n}}\n"
    );
    let (ok, text) = run(&src, "loop_exits");
    assert!(ok && text == "iter 0\nclosing 0\nclosing 1\nclosing 2\ndone\n", "{text}");
}

#[test]
fn nested_with_closes_innermost_first() {
    let src = format!("{RES}pub fn main() {{\n    with a = Res {{ label: \"outer\" }} {{\n        with b = Res {{ label: \"inner\" }} {{\n            println(\"nested\")\n        }}\n        println(\"between\")\n    }}\n}}\n");
    let (ok, text) = run(&src, "nested");
    assert!(ok && text == "nested\nclosing inner\nbetween\nclosing outer\n", "{text}");
}

#[test]
fn closes_before_a_try_propagation() {
    let src = format!(
        "{RES}fn f(fail: Bool) -> Result<Int, String> {{\n    with r = Res {{ label: \"try\" }} {{\n        if fail {{\n            let e: Result<Int, String> = Err(\"boom\")\n            let v = e?\n            return Ok(v)\n        }}\n    }}\n    return Ok(1)\n}}\npub fn main() {{\n    match f(true) {{\n        Err(e) => {{ println(\"got err: \" + e) }}\n        Ok(v) => {{ println(v.to_string()) }}\n    }}\n}}\n"
    );
    let (ok, text) = run(&src, "try_prop");
    assert!(ok && text == "closing try\ngot err: boom\n", "{text}");
}

#[test]
fn a_stdlib_type_satisfies_closeable_with_no_impl_block() {
    let path = std::env::temp_dir().join(format!("mote_with_stdlib_file_{}.tmp", std::process::id()));
    let path_str = path.to_string_lossy().replace('\\', "\\\\");
    let src = format!(
        "import std.sys.fs\npub fn main() {{\n    with f = fs.create(\"{path_str}\")! {{\n        f.write_text(\"hi\")!\n    }}\n    with f2 = fs.open(\"{path_str}\")! {{\n        println(f2.read(2)!.to_string())\n    }}\n    fs.remove_file(\"{path_str}\")!\n}}\n"
    );
    let (ok, text) = run(&src, "stdlib_file");
    std::fs::remove_file(&path).ok();
    assert!(ok && text.contains("Bytes[68 69]"), "{text}");
}

#[test]
fn a_type_without_close_is_a_checker_error() {
    rejected(
        "pub fn main() {\n    with x = 5 {\n        println(x)\n    }\n}\n",
        "no_close",
        "needs a `Closeable` resource",
    );
}

#[test]
fn an_any_typed_resource_is_a_checker_error() {
    rejected(
        "import { Any } from std.experimental.types\nfn take(x: Any) {\n    with r = x {\n        println(\"body\")\n    }\n}\npub fn main() {\n    take(5)\n}\n",
        "any_typed",
        "must be known, not `Any`",
    );
}

#[test]
fn with_at_the_top_level_is_a_checker_error() {
    let src = "class Res {\n    pub fn close(self) -> Result<Null, Error> { return Ok(null) }\n}\nwith r = Res {} {\n    println(\"top\")\n}\n";
    rejected(src, "top_level", "only supported inside a function body");
}
