//! Named arguments: `f(a, y = 2)` reorders to the declared order and calls the default of each parameter it skips.

use std::process::Command;

fn run_files(files: &[(&str, &str)], tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_named_args_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, source) in files {
        std::fs::write(dir.join(name), source).unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

fn run(source: &str, tag: &str) -> (bool, String) {
    run_files(&[("main.mote", source)], tag)
}

fn prints(source: &str, tag: &str, expected: &str) {
    let (ok, text) = run(source, tag);
    assert!(ok && text.trim() == expected, "expected `{expected}`, got: {text}");
}

fn rejected(source: &str, tag: &str, message: &str) {
    let (ok, text) = run(source, tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

const AREA: &str = "fn area(width: Int, height: Int = 10, depth: Int = 1) -> Int { return width * height * depth }\n";

#[test]
fn named_arguments_may_come_in_any_order() {
    prints(&format!("{AREA}fn main() {{ println(area(depth = 2, width = 4, height = 5)) }}\n"), "order", "40");
}

#[test]
fn a_positional_argument_may_come_before_named_ones() {
    prints(&format!("{AREA}fn main() {{ println(area(3, depth = 5)) }}\n"), "mixed", "150");
}

#[test]
fn a_named_argument_skips_the_defaults_before_it() {
    prints(&format!("{AREA}fn main() {{ println(area(width = 3)) }}\n"), "skip", "30");
}

#[test]
fn a_method_takes_named_arguments() {
    let src = "class C {\n    n: Int\n    pub fn add(self, k: Int = 1, m: Int = 10) -> Int { return self.n + k * m }\n}\nfn main() {\n    let c = C { n: 4 }\n    println(c.add(m = 3))\n}\n";
    prints(src, "method", "7");
}

#[test]
fn a_constructor_takes_named_arguments() {
    let src = "struct Box {\n    w: Int\n    h: Int\n    fn new(w: Int, h: Int = 2, tag: String = \"b\") -> Box { return Box { w: w, h: h } }\n}\nfn main() {\n    let b = Box(w = 3, tag = \"x\")\n    println(\"${b.w} ${b.h}\")\n}\n";
    prints(src, "ctor", "3 2");
}

#[test]
fn named_arguments_nest_and_may_be_lambdas() {
    let src = "fn apply(f: (Int) -> Int, x: Int, times: Int = 1) -> Int {\n    var r = x\n    var i = 0\n    while i < times {\n        r = f(r)\n        i += 1\n    }\n    return r\n}\n";
    prints(&format!("{AREA}{src}fn main() {{ println(apply(x = 1, f = |v| v * 2, times = area(width = 1, depth = 3))) }}\n"), "nest", "1073741824");
}

#[test]
fn a_var_parameter_is_passed_by_name() {
    let src = "fn bump(var n: Int, by: Int = 1, times: Int = 1) { n += by * times }\nfn main() {\n    var m = 0\n    bump(times = 3, n = m)\n    println(m)\n}\n";
    prints(src, "var", "3");
}

#[test]
fn a_skipped_default_runs_in_its_own_module() {
    let util = "let LIMIT = 9\npub fn size(w: Int, h: Int = LIMIT, d: Int = 1) -> Int { return w * h * d }\n";
    let (ok, text) = run_files(&[("util.mote", util), ("main.mote", "import .util\nfn main() { println(util.size(2, d = 3)) }\n")], "module");
    assert!(ok && text.trim() == "54", "{text}");
}

#[test]
fn a_comparison_in_an_argument_is_still_a_comparison() {
    prints("fn f(a: Bool) -> Bool { return a }\nfn main() { println(f(1 == 1)) }\n", "eq", "true");
}

#[test]
fn an_unknown_name_is_named() {
    rejected(&format!("{AREA}fn main() {{ println(area(1, size = 2)) }}\n"), "unknown", "`area` has no parameter `size`");
}

#[test]
fn a_parameter_given_twice_is_rejected() {
    rejected(&format!("{AREA}fn main() {{ println(area(1, width = 2)) }}\n"), "twice", "`width` is given twice");
    rejected(&format!("{AREA}fn main() {{ println(area(width = 1, width = 2)) }}\n"), "twice2", "`width` is given twice");
}

#[test]
fn a_positional_argument_cannot_follow_a_named_one() {
    rejected(&format!("{AREA}fn main() {{ println(area(width = 1, 2)) }}\n"), "after", "a positional argument cannot follow a named one");
}

#[test]
fn a_required_parameter_must_be_given() {
    rejected(&format!("{AREA}fn main() {{ println(area(depth = 2)) }}\n"), "missing", "`area` needs an argument for `width`");
}

#[test]
fn a_function_value_takes_no_names() {
    rejected("fn main() {\n    let f = |x: Int| x + 1\n    println(f(x = 1))\n}\n", "value", "named arguments need a function or method that names its parameters");
}

#[test]
fn a_variadic_function_takes_no_names() {
    rejected("fn v(a: Int, ...rest: Int) -> Int { return a }\nfn main() { println(v(a = 1)) }\n", "variadic", "so none can be named");
}

#[test]
fn a_generic_function_cannot_skip_a_default() {
    let src = "fn id<T>(x: T, y: Int = 1, z: Int = 2) -> T { return x }\nfn main() { println(id(1, z = 5)) }\n";
    rejected(src, "generic", "is generic, so a default cannot be skipped");
}
