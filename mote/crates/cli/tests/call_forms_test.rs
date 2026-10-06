//! Call forms: a function stored in a field is called as `obj.f(x)`; a lambda argument's untyped parameters are resolved from the collection it goes into.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_call_forms_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

fn prints(source: &str, tag: &str, expected: &str) {
    let (ok, text) = run(source, tag);
    assert!(ok && text.trim() == expected, "expected `{expected}`, got: {text}");
}

fn rejected(source: &str, tag: &str, message: &str) {
    let (ok, text) = run(source, tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

const BOXES: &str = "class Box {\n    f: (Int) -> Int\n}\nstruct Pair {\n    g: (Int) -> Int\n    n: Int\n}\n";

#[test]
fn a_class_field_of_function_type_is_called_directly() {
    prints(&format!("{BOXES}fn main() {{\n    let b = Box {{ f: |x: Int| x + 1 }}\n    println(b.f(1))\n}}\n"), "class_field", "2");
}

#[test]
fn a_struct_field_of_function_type_is_called_with_parentheses_too() {
    let main = "fn main() {\n    let p = Pair { g: |x: Int| x * 3, n: 2 }\n    println(p.g(p.n))\n    println((p.g)(p.n))\n}\n";
    prints(&format!("{BOXES}{main}"), "struct_field", "6\n6");
}

#[test]
fn a_function_field_call_checks_its_arguments() {
    let main = "fn main() {\n    let b = Box { f: |x: Int| x + 1 }\n    println(b.f(\"no\"))\n}\n";
    rejected(&format!("{BOXES}{main}"), "field_arg", "argument 1");
}

#[test]
fn a_name_that_is_neither_a_method_nor_a_function_field_is_an_error() {
    let main = "fn main() {\n    let b = Box { f: |x: Int| x + 1 }\n    println(b.h(1))\n}\n";
    rejected(&format!("{BOXES}{main}"), "no_field", "no method `.h()` on `Box`");
}

const EL: &str = "class P {\n    var n: Int\n\n    fn edit(var self, t: String) {\n        self.n += 1\n    }\n}\n\nclass El<S> {\n    h: (var S, String) -> Null\n}\n\nfn el<S>(h: (var S, String) -> Null) -> El<S> {\n    return El { h: h }\n}\n";

#[test]
fn a_lambda_inside_a_generic_call_takes_its_parameter_types_from_the_list() {
    let main = "fn main() {\n    var xs: List<El<P>> = []\n    xs.push(el(|var v, t| { v.edit(t) }))\n    println(xs.len())\n}\n";
    prints(&format!("{EL}{main}"), "nested_lambda", "1");
}

#[test]
fn a_method_on_a_parameter_nothing_fixes_is_a_checker_error() {
    let main = "fn make<S>(h: (S) -> Null) -> Int { return 1 }\nfn main() {\n    println(make(|v| { v.edit() }))\n}\n";
    rejected(main, "unknown_param", "is not known; write it");
}
