//! The checker rejects operators applied to primitives they do not support: the VM never sees them.

use std::process::Command;

fn stderr_of(body: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_checker_operand_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), format!("fn main() {{\n{body}\n}}\n")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(!out.status.success(), "`{body}` should be rejected");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn mixed_or_unsupported_operands_are_compile_errors() {
    let cases = [
        ("let r = 1 + 0.5", "cannot apply `+` to `Int` and `Float`"),
        ("let r = true + 1", "cannot apply `+` to `Bool` and `Int`"),
        ("let r = \"a\" * 2", "cannot apply `*` to `String` and `Int`"),
        ("let r = 1 << 2.0", "cannot apply `<<` to `Int` and `Float`"),
        ("let r = 1.5 < 2", "cannot compare `Float` with `Int`"),
        ("let r = true < false", "cannot compare `Bool` with `Bool`"),
        ("let l: List<Int> = [1]\nlet r = l.get(0) + \"s\"", "cannot apply `+` to `Int` and `String`"),
    ];
    for (i, (body, message)) in cases.iter().enumerate() {
        let err = stderr_of(body, &i.to_string());
        assert!(err.contains(message), "`{body}`: {err}");
        assert!(!err.contains("panicked"), "`{body}` reached the VM: {err}");
    }
}

#[test]
fn unary_operators_and_field_access_on_primitives_are_compile_errors() {
    let cases = [
        ("let s = \"x\"\nlet t = -s", "cannot apply `-` to `String`"),
        ("let n = 3\nlet t = !n", "cannot apply `!` to `Int`"),
        ("let x = 1\nx.foo = 2", "`Int` has no field 'foo'"),
        ("let x = 1\nprintln(x.foo.to_string())", "`Int` has no field 'foo'"),
    ];
    for (i, (body, message)) in cases.iter().enumerate() {
        let err = stderr_of(body, &format!("u{i}"));
        assert!(err.contains(message), "`{body}`: {err}");
        assert!(!err.contains("panicked"), "`{body}` reached the VM: {err}");
    }
}

#[test]
fn compound_assignment_is_checked_like_its_operator() {
    let cases = [
        ("var x = 1\nx += \"a\"", "cannot apply `+` to `Int` and `String`"),
        ("var f = 1.5\nf *= 2", "cannot apply `*` to `Float` and `Int`"),
        ("let y = 1\ny += 1", "cannot assign to immutable constant 'y'"),
    ];
    for (i, (body, message)) in cases.iter().enumerate() {
        let err = stderr_of(body, &format!("compound{i}"));
        assert!(err.contains(message), "`{body}`: {err}");
    }
}
