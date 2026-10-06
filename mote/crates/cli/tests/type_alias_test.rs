//! `type` aliases resolve (generic, imported, through a module alias) and an unknown type name is an error.

use std::process::Command;

const GEO: &str = "pub type Id = Int\npub type Pair<T> = (T, T)\npub struct Pt {\n    pub x: Int\n    pub y: Int\n    pub fn new(x: Int, y: Int) -> Pt { return Pt { x: x, y: y } }\n}\npub type P = Pt\n";

fn run(main: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_type_alias_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("geo.mote"), GEO).unwrap();
    std::fs::write(dir.join("main.mote"), main).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn rejected(main: &str, tag: &str, want: &str) {
    let (ok, text) = run(main, tag);
    assert!(!ok && text.contains(want), "{text}");
}

#[test]
fn aliases_resolve_locally_imported_and_through_a_module_alias() {
    let main = "import .geo as g\nimport { Id, P } from .geo\n\ntype Names = List<String>\n\nfn swap(p: g.Pair<Int>) -> g.Pair<Int> {\n    return (p.1, p.0)\n}\n\nfn main() {\n    let a: Id = 4\n    let b: g.Id = a + 1\n    println(b)\n    let n: Names = [\"x\", \"y\"]\n    println(n.len())\n    let q: P = g.Pt.new(1, 2)\n    println(q.y)\n    println(swap((1, 2)).0)\n}\n";
    let (ok, text) = run(main, "ok");
    assert!(ok && text == "5\n2\n2\n2\n", "{text}");
}

#[test]
fn an_alias_is_checked_as_its_target() {
    rejected("type Id = Int\nfn main() {\n    let a: Id = \"x\"\n}\n", "mismatch", "of type 'Int' with expression of type 'String'");
}

#[test]
fn an_unknown_type_name_is_an_error() {
    rejected("fn f(x: Strng) -> Int {\n    return 1\n}\nfn main() {\n}\n", "unknown", "unknown type `Strng`");
    rejected("fn main() {\n    let l: List<Foo> = []\n}\n", "unknown_arg", "unknown type `Foo`");
    rejected("type X = Nope\nfn main() {\n}\n", "unknown_target", "unknown type `Nope`");
}

#[test]
fn a_forward_reference_is_not_unknown() {
    let (ok, text) = run("fn main() {\n    let a: Later = Later { n: 1 }\n    println(a.n)\n}\nstruct Later {\n    n: Int\n}\n", "forward");
    assert!(ok && text == "1\n", "{text}");
}

#[test]
fn a_bad_alias_is_an_error() {
    rejected("type A = B\ntype B = A\nfn main() {\n}\n", "cycle", "type alias `A` refers to itself");
    rejected("type Pair<T> = (T, T)\nfn main() {\n    let a: Pair<Int, Int> = (1, 2)\n}\n", "arity", "`Pair` takes 1 type argument(s), but 2 were given");
    rejected("struct S {\n    n: Int\n}\ntype S = Int\nfn main() {\n}\n", "dup", "type `S` is already defined");
    rejected("import { P } from .geo\nfn main() {\n    let q = P { x: 1, y: 2 }\n}\n", "literal", "`P` is a type alias; a literal needs the type's own name `Pt`");
}
