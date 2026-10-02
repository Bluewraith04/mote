//! Operators on `Any` operands are guarded at runtime and fault with a located type mismatch.

use std::process::Command;

const PROGRAM: &str = "import { Any } from std.experimental.types\nfn main() {\n    let m: Map<String, Any> = Map()\n    m.set(\"k\", \"v\")\n    m.set(\"n\", 4)\n    let n = m.get(\"n\")\n    println((n + 1).to_string())\n    println((n < 9).to_string())\n    let x = m.get(\"k\")\n    let total = n + x\n    println(total.to_string())\n}\n";

fn run(flags: &[&str], body: &str, tag: &str) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("mote_any_guard_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), body).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).args(flags).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn an_any_operand_of_the_wrong_type_faults_with_its_location() {
    let (out, err) = run(&[], PROGRAM, "debug");
    assert_eq!(out, "5\ntrue\n");
    assert!(err.contains("type mismatch: cannot apply `+` to `Int` and `String`"), "{err}");
    assert!(err.contains("main.mote:10:17"), "{err}");
    assert!(err.contains("10 |     let total = n + x"), "{err}");
    assert!(err.contains("   |                 ^^^^^"), "{err}");
    assert!(!err.contains("panicked"), "{err}");
}

#[test]
fn a_release_build_keeps_the_guard_and_drops_the_details() {
    let (out, err) = run(&["--release"], PROGRAM, "release");
    assert_eq!(out, "5\ntrue\n");
    assert!(err.contains("type mismatch") && !err.contains("cannot apply") && !err.contains("main.mote"), "{err}");
}

#[test]
fn unary_operators_on_an_any_operand_are_guarded() {
    let body = "import { Any } from std.experimental.types\nfn main() {\n    let m: Map<String, Any> = Map()\n    m.set(\"k\", \"v\")\n    let x = m.get(\"k\")\n    println((-x).to_string())\n}\n";
    let (_, err) = run(&[], body, "unary");
    assert!(err.contains("type mismatch: cannot apply `-` to `String`"), "{err}");
    assert!(!err.contains("panicked"), "{err}");
}

#[test]
fn compound_assignment_on_an_any_operand_is_guarded() {
    let body = "import { Any } from std.experimental.types\nfn main() {\n    let m: Map<String, Any> = Map()\n    m.set(\"k\", \"v\")\n    var t = 1\n    t += m.get(\"k\")\n}\n";
    let (_, err) = run(&[], body, "compound");
    assert!(err.contains("type mismatch: cannot apply `+` to `Int` and `String`"), "{err}");
    assert!(err.contains("main.mote:6:5"), "{err}");
}

#[test]
fn integer_division_by_zero_is_a_located_arithmetic_fault() {
    let body = "fn main() {\n    let z = 0\n    println(7.0 / 0.0)\n    println(7 % z)\n}\n";
    let (out, err) = run(&[], body, "divzero");
    assert_eq!(out, "inf\n");
    assert!(err.contains("arithmetic error: remainder by zero"), "{err}");
    assert!(err.contains("main.mote:4:13"), "{err}");
    assert!(!err.contains("internal panic"), "{err}");
}

#[test]
fn a_field_read_through_any_finds_the_field_by_name() {
    let body = "import { Any } from std.experimental.types\nstruct P {\n    x: Int\n    y: Int\n}\n\nfn main() {\n    let m: Map<String, Any> = Map()\n    m.set(\"p\", P { x: 1, y: 2 })\n    let p = m.get(\"p\")\n    println(p.y)\n    p.y += 40\n    println(p.y)\n    println(p.z)\n}\n";
    let (out, err) = run(&[], body, "named_field");
    assert_eq!(out, "2\n42\n");
    assert!(err.contains("type mismatch: `P` has no field 'z'"), "{err}");
    assert!(err.contains("main.mote:14:13"), "{err}");
}

#[test]
fn a_field_access_on_a_scalar_through_any_is_a_fault_not_a_panic() {
    let body = "import { Any } from std.experimental.types\nfn main() {\n    let m: Map<String, Any> = Map()\n    m.set(\"n\", 4)\n    println(m.get(\"n\").0)\n}\n";
    let (_, err) = run(&[], body, "scalar_field");
    assert!(err.contains("type mismatch: `Int` has no fields"), "{err}");
    assert!(!err.contains("internal panic"), "{err}");
}

#[test]
fn an_any_argument_is_checked_against_its_parameter_type() {
    let body = "import { Any } from std.experimental.types\nfn inc(n: Int) -> Int {\n    return n + 1\n}\n\nfn main() {\n    let m: Map<String, Any> = Map()\n    m.set(\"n\", 4)\n    m.set(\"s\", \"str\")\n    println(inc(m.get(\"n\")))\n    println(inc(m.get(\"s\")))\n}\n";
    let (out, err) = run(&[], body, "arg_check");
    assert_eq!(out, "5\n");
    assert!(err.contains("expected `Int`, found `String`"), "{err}");
    assert!(err.contains("main.mote:11:17"), "{err}");
}
