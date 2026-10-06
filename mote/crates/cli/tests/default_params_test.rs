//! Default parameters in the checker and at the call site, and the field/method namespace.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_default_params_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

fn rejected(source: &str, tag: &str, message: &str) {
    let (ok, text) = run(source, tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

#[test]
fn a_call_may_pass_every_argument_of_a_defaulted_function() {
    let (ok, text) = run("fn f(a: Int, b: Int = 2) -> Int { return a + b }\nfn main() { println(f(1, 5)) }\n", "full");
    assert!(ok && text.trim() == "6", "{text}");
}

#[test]
fn a_call_with_too_few_or_too_many_arguments_names_the_range() {
    let f = "fn f(a: Int, b: Int = 2) -> Int { return a + b }\n";
    rejected(&format!("{f}fn main() {{ println(f()) }}\n"), "few", "takes 1 to 2 argument(s), but 0 were given");
    rejected(&format!("{f}fn main() {{ println(f(1, 2, 3)) }}\n"), "many", "takes 1 to 2 argument(s), but 3 were given");
}

#[test]
fn defaults_must_be_trailing() {
    rejected("fn f(a: Int = 1, b: Int) -> Int { return a + b }\nfn main() { println(f(1, 2)) }\n", "trailing", "parameter `b` needs a default");
}

#[test]
fn a_default_is_checked_against_its_parameter_type() {
    rejected("fn f(a: Int = \"x\") -> Int { return a }\nfn main() { println(f(1)) }\n", "type", "default for `a` expects");
}

#[test]
fn a_default_cannot_name_another_parameter() {
    rejected("fn f(a: Int, b: Int = a) -> Int { return a + b }\nfn main() { println(f(1, 2)) }\n", "param", "Undefined identifier 'a'");
}

#[test]
fn a_method_takes_defaults_too() {
    let src = "class C {\n    n: Int\n    pub fn add(self, k: Int = 1) -> Int { return self.n + k }\n}\nfn main() {\n    let c = C { n: 4 }\n    println(c.add(3))\n}\n";
    let (ok, text) = run(src, "method");
    assert!(ok && text.trim() == "7", "{text}");
}

#[test]
fn a_class_cannot_have_a_field_and_a_method_of_one_name() {
    rejected("class C {\n    size: Int\n    pub fn size(self) -> Int { return 1 }\n}\nfn main() { }\n", "clash", "`size` is both a field and a method of `C`");
}

#[test]
fn an_omitted_argument_takes_its_default() {
    let src = "fn f(a: Int, b: Int = 2, c: Int = b0()) -> Int { return a * 100 + b * 10 + c }\nfn b0() -> Int { return 5 }\nfn main() {\n    println(f(1))\n    println(f(1, 3))\n    println(f(1, 3, 4))\n}\n";
    let (ok, text) = run(src, "omit");
    assert!(ok && text.trim() == "125\n135\n134", "{text}");
}

#[test]
fn a_default_is_evaluated_fresh_on_every_call() {
    let src = "fn f(xs: List<Int> = []) -> List<Int> {\n    return xs\n}\nfn main() {\n    var a = f()\n    a.push(1)\n    println(a.len())\n    println(f().len())\n}\n";
    let (ok, text) = run(src, "fresh");
    assert!(ok && text.trim() == "1\n0", "{text}");
}

#[test]
fn defaults_reach_instance_and_static_methods() {
    let src = "class Acc {\n    total: Int\n    pub fn add(self, k: Int = 1, scale: Int = 10) -> Int { return self.total + k * scale }\n    pub fn make(n: Int = 3) -> Int { return n }\n}\nfn main() {\n    let a = Acc { total: 4 }\n    println(a.add())\n    println(a.add(2))\n    println(Acc.make())\n}\n";
    let (ok, text) = run(src, "methods");
    assert!(ok && text.trim() == "14\n24\n3", "{text}");
}

#[test]
fn a_default_sees_the_private_names_of_its_own_module() {
    let dir = std::env::temp_dir().join(format!("mote_default_params_module_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("lib.mote"), "fn secret() -> Int { return 7 }\npub fn offset(x: Int, by: Int = secret() + 100) -> Int { return x + by }\n").unwrap();
    std::fs::write(dir.join("main.mote"), "import { offset } from .lib\nfn main() {\n    println(offset(1))\n    println(offset(1, 2))\n}\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success() && text.trim() == "108\n3", "{text}");
}

#[test]
fn a_method_in_value_position_is_an_error_that_shows_the_lambda() {
    let src = "class C {\n    n: Int\n    pub fn get(self, k: Int) -> Int { return self.n + k }\n}\nfn main() {\n    let c = C { n: 1 }\n    let f = c.get\n    println(f(2))\n}\n";
    rejected(src, "value", "`get` is a method, not a value: write `|a1| c.get(a1)`");
}

#[test]
fn a_lambda_wrapping_the_method_works() {
    let src = "class C {\n    n: Int\n    pub fn get(self, k: Int) -> Int { return self.n + k }\n}\nfn main() {\n    let c = C { n: 1 }\n    let f = |k| c.get(k)\n    println(f(2))\n}\n";
    let (ok, text) = run(src, "lambda");
    assert!(ok && text.trim() == "3", "{text}");
}
