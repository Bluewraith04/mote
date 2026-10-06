//! The std traits (`Display`, `Debug`, `Eq`, `Ord`) and their structural satisfaction by the primitive types.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_stdtraits_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn check(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_stdtraits_chk_{}_{}", tag, std::process::id()));
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

#[test]
fn int_float_char_string_satisfy_ord() {
    let src = "fn cmp<T: Ord>(a: T, b: T) -> Int { return a.compare(b) }\nfn main() {\n    println(cmp(1, 2).to_string())\n    println(cmp(2.0, 1.0).to_string())\n    println(cmp('a', 'a').to_string())\n    println(cmp(\"x\", \"y\").to_string())\n}\n";
    let (ok, text) = run(src, "ord");
    assert!(ok && text == "-1\n1\n0\n-1\n", "{text}");
}

#[test]
fn bool_does_not_satisfy_ord() {
    let src = "fn cmp<T: Ord>(a: T, b: T) -> Int { return a.compare(b) }\nfn main() {\n    cmp(true, false)\n}\n";
    rejected(src, "bool_ord", "`Bool` cannot be used for `T`: missing method `compare`");
}

#[test]
fn bool_satisfies_eq_display_debug() {
    let src = "fn same<T: Eq>(a: T, b: T) -> Bool { return a.eq(b) }\nfn show<T: Display>(a: T) -> String { return a.to_string() }\npub fn main() {\n    println(same(true, true).to_string())\n    println(show(false))\n    println(true.debug())\n}\n";
    let (ok, text) = run(src, "bool_eq");
    assert!(ok && text == "true\nfalse\ntrue\n", "{text}");
}

#[test]
fn eq_rejects_a_mismatched_argument_type() {
    rejected(
        "fn main() {\n    println(3.eq(\"x\").to_string())\n}\n",
        "eq_mismatch",
        "`.eq()` expects 'Int'",
    );
}

#[test]
fn compare_takes_exactly_one_argument() {
    rejected("fn main() {\n    println(3.compare().to_string())\n}\n", "compare_arity", "`.compare()` takes 1 argument");
}

#[test]
fn an_unknown_method_on_a_primitive_is_rejected() {
    rejected("fn main() {\n    3.frobnicate()\n}\n", "unknown_prim_method", "no method `.frobnicate()` on `Int`");
}

#[test]
fn a_class_with_hand_written_methods_satisfies_ord_and_display() {
    let src = "class Meters {\n    n: Int\n    pub fn compare(self, other: Meters) -> Int {\n        if self.n < other.n { return -1 }\n        if self.n > other.n { return 1 }\n        return 0\n    }\n    pub fn to_string(self) -> String { return \"${self.n}m\" }\n}\nfn cmp<T: Ord>(a: T, b: T) -> Int { return a.compare(b) }\nfn show<T: Display>(a: T) -> String { return a.to_string() }\npub fn main() {\n    println(cmp(Meters { n: 1 }, Meters { n: 2 }).to_string())\n    println(show(Meters { n: 5 }))\n}\n";
    let (ok, text) = run(src, "class_ord");
    assert!(ok && text == "-1\n5m\n", "{text}");
}

#[test]
fn derived_display_and_debug_satisfy_the_bounds() {
    let src = "@derive(Display, Debug)\nclass Point {\n    x: Int\n    y: Int\n}\nfn show<T: Display>(a: T) -> String { return a.to_string() }\npub fn main() {\n    let p = Point { x: 1, y: 2 }\n    println(show(p))\n    println(p.debug())\n}\n";
    let (ok, text) = run(src, "derived_bound");
    assert!(ok && text == "Point(x: 1, y: 2)\nPoint { x: 1, y: 2 }\n", "{text}");
}

#[test]
fn sort_works_over_int_and_string_by_ord() {
    let src = "import { * } from std.iter\npub fn main() {\n    let xs = sort([5, 3, 1, 4, 2])\n    for x in xs { print(x.to_string()); print(\" \") }\n    println(\"\")\n    let ss = sort([\"banana\", \"apple\", \"cherry\"])\n    for s in ss { print(s); print(\" \") }\n    println(\"\")\n}\n";
    let (ok, text) = run(src, "sort_ord");
    assert!(ok && text == "1 2 3 4 5 \napple banana cherry \n", "{text}");
}

#[test]
fn sort_is_refused_on_a_type_without_ord() {
    let src = "import { * } from std.iter\nclass Blob {\n    n: Int\n}\npub fn main() {\n    sort([Blob { n: 1 }])\n}\n";
    rejected(src, "sort_no_ord", "cannot be used for");
}

const TRAIT_BOUNDS: &str = include_str!("programs_mote/trait_bounds.mote");

#[test]
fn done_bar_max_describe_and_a_derived_struct_run() {
    let (ok, text) = run(TRAIT_BOUNDS, "done_bar");
    assert!(ok && text == "7\npear\nvalue: Point(x: 1, y: 2)\nPoint { x: 1, y: 2 }\n", "{text}");
}

fn trait_bounds_with(extra_items: &str, main_body: &str) -> String {
    let head = TRAIT_BOUNDS.split("fn main() {").next().unwrap();
    format!("{head}{extra_items}\nfn main() {{\n{main_body}\n}}\n")
}

#[test]
fn max_is_refused_on_a_type_without_ord() {
    let src = trait_bounds_with("class Blob {\n    n: Int\n}\n", "    max(Blob { n: 1 }, Blob { n: 2 })");
    rejected(&src, "max_no_ord", "cannot be used for");
}

#[test]
fn describe_is_refused_on_a_type_without_display() {
    let src = trait_bounds_with("class Blob {\n    n: Int\n}\n", "    describe(Blob { n: 1 })");
    rejected(&src, "describe_no_display", "cannot be used for");
}

#[test]
fn deriving_over_an_existing_method_is_refused() {
    let src = "@derive(Display)\nclass Point {\n    x: Int\n    pub fn to_string(self) -> String { return \"x\" }\n}\nfn main() { }\n";
    rejected(src, "derive_clash", "already has a method named `to_string`");
}
