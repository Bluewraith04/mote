//! `Null` is the type of no result: a function without a return type returns it, and `Void` is gone.

use std::process::Command;

fn mote(verb: &str, source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_null_unit_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.mote");
    std::fs::write(&file, source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg(verb).arg(&file).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn test_a_function_without_a_return_type_returns_null() {
    let src = "fn a() { }\nfn b() -> Null { }\nfn main() {\n    println(a())\n    println(b())\n}\n";
    let (ok, text) = mote("run", src, "returns");
    assert!(ok, "{text}");
    assert_eq!(text, "null\nnull\n");
}

#[test]
fn test_a_callback_typed_to_return_null_accepts_any_result() {
    let src = "fn each(xs: List<Int>, f: (Int) -> Null) {\n    for x in xs { f(x) }\n}\nfn main() {\n    let seen: List<Int> = []\n    each([1, 2], |x| seen.push(x))\n    each([1, 2], |x| x * 2)\n    println(seen)\n}\n";
    let (ok, text) = mote("run", src, "lenient");
    assert!(ok, "{text}");
    assert_eq!(text, "[1, 2]\n");
}

#[test]
fn test_a_callback_typed_to_return_a_value_rejects_null() {
    let (ok, text) = mote("check", "fn main() {\n    let h: () -> Int = || null\n}\n", "strict");
    assert!(!ok);
    assert!(text.contains("cannot initialize variable 'h'"), "{text}");
}

#[test]
fn test_a_null_result_is_not_an_int() {
    let (ok, text) = mote("check", "fn a() { }\nfn main() {\n    let n: Int = a()\n}\n", "not_int");
    assert!(!ok);
    assert!(text.contains("of type 'Int' with expression of type 'Null'"), "{text}");
}

#[test]
fn test_void_is_no_longer_a_type() {
    let (ok, text) = mote("check", "fn a() -> Void { }\n", "void_gone");
    assert!(!ok);
    assert!(text.contains("unknown type `Void`"), "{text}");
}
