//! Imports: a plain import brings the module's namespace only; `import { * } from m` brings every `pub` name bare.

use std::process::Command;

fn run(files: &[(&str, &str)], tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_import_forms_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, source) in files {
        std::fs::write(dir.join(name), source).unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

const HELPER: (&str, &str) = ("helper.mote", "pub fn double(n: Int) -> Int {\n    return n * 2\n}\n\nfn hidden() -> Int {\n    return 1\n}\n");

#[test]
fn a_plain_import_is_used_through_its_name() {
    let (ok, text) = run(&[HELPER, ("main.mote", "import .helper\nfn main() { println(helper.double(4)) }\n")], "plain");
    assert!(ok && text.trim() == "8", "got: {text}");
}

#[test]
fn a_plain_import_does_not_bring_the_names_in_bare() {
    let (ok, text) = run(&[HELPER, ("main.mote", "import .helper\nfn main() { println(double(4)) }\n")], "plain_bare");
    assert!(!ok && text.contains("no function `double`"), "got: {text}");
}

#[test]
fn a_star_import_brings_every_pub_name_bare() {
    let (ok, text) = run(&[HELPER, ("main.mote", "import { * } from .helper\nfn main() { println(double(4)) }\n")], "star");
    assert!(ok && text.trim() == "8", "got: {text}");
}

#[test]
fn a_star_import_leaves_private_names_out() {
    let (ok, text) = run(&[HELPER, ("main.mote", "import { * } from .helper\nfn main() { println(hidden()) }\n")], "star_private");
    assert!(!ok && text.contains("hidden"), "got: {text}");
}

#[test]
fn a_star_import_from_std_works() {
    let (ok, text) = run(&[("main.mote", "import { * } from std.math\nfn main() { println(abs(-3)) }\n")], "star_std");
    assert!(ok && text.trim() == "3", "got: {text}");
}

#[test]
fn a_pub_star_import_re_exports_every_name() {
    let facade = ("facade.mote", "pub import { * } from .helper\n");
    let (ok, text) = run(&[HELPER, facade, ("main.mote", "import { * } from .facade\nfn main() { println(double(5)) }\n")], "star_facade");
    assert!(ok && text.trim() == "10", "got: {text}");
}

#[test]
fn two_star_imports_of_one_name_are_ambiguous() {
    let other = ("other.mote", "pub fn double(n: Int) -> Int {\n    return n * 3\n}\n");
    let (ok, text) = run(&[HELPER, other, ("main.mote", "import { * } from .helper\nimport { * } from .other\nfn main() { println(double(1)) }\n")], "ambiguous");
    assert!(!ok && text.contains("double"), "got: {text}");
}

#[test]
fn a_function_with_defaults_used_as_a_value_keeps_its_full_arity() {
    let src = "fn area(w: Int, h: Int = 10) -> Int { return w * h }\nfn main() {\n    let f = area\n    println(f(2, 3))\n}\n";
    let (ok, text) = run(&[("main.mote", src)], "default_value");
    assert!(ok && text.trim() == "6", "got: {text}");
    let short = "fn area(w: Int, h: Int = 10) -> Int { return w * h }\nfn main() {\n    let f = area\n    println(f(2))\n}\n";
    let (ok, text) = run(&[("main.mote", short)], "default_value_short");
    assert!(!ok && text.contains("takes 2 argument(s)"), "got: {text}");
}

#[test]
fn a_variadic_function_used_as_a_value_takes_the_list() {
    let src = "fn total(first: Int, ...rest: Int) -> Int {\n    var t = first\n    for x in rest { t += x }\n    return t\n}\nfn main() {\n    let f = total\n    println(f(1, [2, 3]))\n}\n";
    let (ok, text) = run(&[("main.mote", src)], "variadic_value");
    assert!(ok && text.trim() == "6", "got: {text}");
}
