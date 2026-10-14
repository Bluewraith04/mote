//! Stage 0: a `@modifier` function runs at compile time in a sandboxed VM, on a `Decl` that crosses as JSON.

use modules::stage0::{Limits, Stage0};
use std::path::PathBuf;

const DESCRIBE: &str = "
import std.meta as meta
import std.string as str

@modifier
pub fn describe(d: meta.Decl) -> Result<meta.Additions, meta.Diagnostic> {
    var names: List<String> = []
    for f in d.fields {
        names.push(f.name + \":\" + meta.render(f.ty))
    }
    let src = \"fn shape(self) -> String { return \\\"\" + d.name + \"(\" + str.join(names, \",\") + \")\\\" }\"
    return Ok(meta.Additions.new(src, \"fn __helper() -> Int { return 1 }\"))
}
";

fn workdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mote_stage0_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn build(tag: &str, module: &str, name: &str) -> Result<Stage0, String> {
    let dir = workdir(tag);
    let path = dir.join("mods.mote");
    std::fs::write(&path, module).unwrap();
    let built = Stage0::build(&path, name);
    std::fs::remove_dir_all(&dir).ok();
    built
}

/// The JSON a modifier reads for the declaration named `name` in `source`.
fn decl(source: &str, name: &str) -> String {
    let tokens = compiler::lexer::Lexer::new(source).tokenize().unwrap();
    let program = compiler::parser::Parser::new(tokens).parse().unwrap();
    compiler::modifiers::declaration_json(&program, name).unwrap()
}

#[test]
fn a_modifier_answers_methods_for_a_declaration() {
    let mut stage0 = build("describe", DESCRIBE, "describe").unwrap();
    let json = decl("struct P {\n    x: Int\n    ys: List<Int>\n}\n", "P");
    let out = stage0.apply(&json).unwrap();
    assert_eq!(out.public, "fn shape(self) -> String { return \"P(x:Int,ys:List<Int>)\" }");
    assert_eq!(out.private, "fn __helper() -> Int { return 1 }");
}

#[test]
fn one_compiled_modifier_serves_many_declarations() {
    let mut stage0 = build("many", DESCRIBE, "describe").unwrap();
    for n in 0..5 {
        let json = decl(&format!("class C{n} {{\n    a: Int\n}}\n"), &format!("C{n}"));
        assert!(stage0.apply(&json).unwrap().public.contains(&format!("C{n}(a:Int)")));
    }
}

#[test]
fn types_are_read_as_written_and_render_back() {
    let module = "
import std.meta as meta
import std.string as str

@modifier
pub fn types(d: meta.Decl) -> Result<meta.Additions, meta.Diagnostic> {
    var out: List<String> = []
    for f in d.fields { out.push(meta.render(f.ty)) }
    return Ok(meta.Additions.new(str.join(out, \";\"), \"\"))
}
";
    let mut stage0 = build("types", module, "types").unwrap();
    let source = "struct T {\n    a: Map<String, List<Int>>\n    b: Int?\n    c: (Int, String)\n    d: (Int) -> Bool\n    e: Int | String\n    f: [Int]\n    g: Send (Int) -> Null\n}\n";
    let out = stage0.apply(&decl(source, "T")).unwrap();
    assert_eq!(out.public, "Map<String, List<Int>>;Int?;(Int, String);(Int) -> Bool;Int | String;[Int];Send (Int) -> Null");
}

#[test]
fn an_enum_is_read_with_its_variants() {
    let module = "
import std.meta as meta
import std.string as str

@modifier
pub fn variants(d: meta.Decl) -> Result<meta.Additions, meta.Diagnostic> {
    var out: List<String> = []
    for v in d.variants {
        match v.shape {
            meta.Shape.Unit => { out.push(v.name) }
            meta.Shape.Tuple => { out.push(v.name + \"/\" + \"${v.types.len()}\") }
            meta.Shape.Struct => { out.push(v.name + \"{\" + \"${v.fields.len()}\" + \"}\") }
        }
    }
    return Ok(meta.Additions.new(str.join(out, \",\"), \"\"))
}
";
    let mut stage0 = build("enum", module, "variants").unwrap();
    let source = "enum E {\n    A\n    B(Int, String)\n    C { x: Int }\n}\n";
    assert_eq!(stage0.apply(&decl(source, "E")).unwrap().public, "A,B/2,C{1}");
}

#[test]
fn a_modifier_reads_attributes_traits_and_methods() {
    let module = "
import std.meta as meta
import std.string as str

@modifier
pub fn facts(d: meta.Decl) -> Result<meta.Additions, meta.Diagnostic> {
    var out: List<String> = []
    out.push(\"pub=${d.is_pub}\")
    for a in d.attrs { out.push(\"attr=\" + a.name) }
    for t in d.traits { out.push(\"trait=\" + meta.render(t)) }
    for m in d.methods { out.push(\"method=\" + m.name + \"/${m.params.len()}\") }
    for g in d.generics { out.push(\"generic=\" + g.name) }
    return Ok(meta.Additions.new(str.join(out, \";\"), \"\"))
}
";
    let mut stage0 = build("facts", module, "facts").unwrap();
    let source = "@stable\npub class Box<T>: (Eq) {\n    v: T\n    fn get(self) -> T { return self.v }\n    fn put(self, x: T) { }\n}\n";
    let out = stage0.apply(&decl(source, "Box")).unwrap();
    assert_eq!(out.public, "pub=true;attr=stable;trait=Eq;method=get/0;method=put/1;generic=T");
}

#[test]
fn a_diagnostic_is_the_error_the_modifier_gave() {
    let module = "
import std.meta as meta

@modifier
pub fn refuse(d: meta.Decl) -> Result<meta.Additions, meta.Diagnostic> {
    return Err(meta.Diagnostic.new(\"`\" + d.name + \"` has nothing to add\"))
}
";
    let mut stage0 = build("refuse", module, "refuse").unwrap();
    assert_eq!(stage0.apply(&decl("struct P {\n}\n", "P")), Err("`P` has nothing to add".to_string()));
}

#[test]
fn a_modifier_module_may_not_import_an_impure_module() {
    for import in ["std.fs", "std.env", "std.io", "std.time", "std.random", "std.socket", "std.process"] {
        let module = format!("import {import}\nimport std.meta as meta\n@modifier\npub fn m(d: meta.Decl) -> Result<meta.Additions, meta.Diagnostic> {{\n    return Ok(meta.Additions.new(\"\", \"\"))\n}}\n");
        let err = build("impure", &module, "m").err().unwrap();
        assert!(err.contains(&format!("a modifier cannot use `{import}`")), "{import}: {err}");
    }
}

#[test]
fn a_module_a_modifier_imports_is_held_to_the_same_rule() {
    let dir = workdir("transitive");
    std::fs::write(dir.join("helper.mote"), "import std.fs\npub fn one() -> Int { return 1 }\n").unwrap();
    let path = dir.join("mods.mote");
    std::fs::write(&path, "import .helper\nimport std.meta as meta\n@modifier\npub fn m(d: meta.Decl) -> Result<meta.Additions, meta.Diagnostic> {\n    return Ok(meta.Additions.new(\"\", \"\"))\n}\n").unwrap();
    let err = Stage0::build(&path, "m").err().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(err.contains("a modifier cannot use `std.fs`") && err.contains("helper"), "{err}");
}

#[test]
fn a_module_a_modifier_uses_may_not_ask_for_a_modifier() {
    let module = "
import std.meta as meta
import { Display } from std.traits

class Local: (Display) {
    n: Int
}

@modifier
pub fn m(d: meta.Decl) -> Result<meta.Additions, meta.Diagnostic> {
    return Ok(meta.Additions.new(\"\", \"\"))
}
";
    let err = build("separation", module, "m").err().unwrap();
    assert!(err.contains("a module a modifier uses cannot ask for `Display`"), "{err}");
}

#[test]
fn a_function_without_the_marker_is_not_a_modifier() {
    let module = "
import std.meta as meta

pub fn plain(d: meta.Decl) -> Result<meta.Additions, meta.Diagnostic> {
    return Ok(meta.Additions.new(\"\", \"\"))
}
";
    let err = build("plain", module, "plain").err().unwrap();
    assert!(err.contains("`plain` is not a `@modifier` function"), "{err}");
}

fn spin() -> &'static str {
    "
import std.meta as meta

@modifier
pub fn spin(d: meta.Decl) -> Result<meta.Additions, meta.Diagnostic> {
    var n = 0
    while true { n = n + 1 }
    return Ok(meta.Additions.new(\"\", \"\"))
}
"
}

#[test]
fn a_modifier_that_never_ends_is_stopped() {
    let mut stage0 = build("spin", spin(), "spin").unwrap().with_limits(Limits { reductions: 50_000, ..Limits::default() });
    let err = stage0.apply(&decl("struct P {\n}\n", "P")).unwrap_err();
    assert!(err.starts_with("modifier `spin` did not finish") && err.contains("reduction limit"), "{err}");
}

#[test]
fn a_modifier_that_fills_the_heap_is_stopped() {
    let module = "
import std.meta as meta

@modifier
pub fn fill(d: meta.Decl) -> Result<meta.Additions, meta.Diagnostic> {
    var keep: List<String> = []
    var i = 0
    while i < 100000000 {
        keep.push(\"padding padding padding padding padding padding\")
        i = i + 1
    }
    return Ok(meta.Additions.new(\"\", \"\"))
}
";
    let mut stage0 = build("fill", module, "fill").unwrap().with_limits(Limits { heap: 4 << 20, ..Limits::default() });
    let err = stage0.apply(&decl("struct P {\n}\n", "P")).unwrap_err();
    assert!(err.starts_with("modifier `fill` did not finish"), "{err}");
}

#[test]
fn a_modifier_that_answers_too_much_is_stopped() {
    let module = "
import std.meta as meta

@modifier
pub fn long(d: meta.Decl) -> Result<meta.Additions, meta.Diagnostic> {
    return Ok(meta.Additions.new(\"x\".repeat(5000), \"\"))
}
";
    let mut stage0 = build("long", module, "long").unwrap().with_limits(Limits { output: 1000, ..Limits::default() });
    let err = stage0.apply(&decl("struct P {\n}\n", "P")).unwrap_err();
    assert!(err.starts_with("modifier `long` did not finish") && err.contains("output limit"), "{err}");
}

#[test]
fn a_fault_in_a_modifier_is_an_error_not_a_crash() {
    let module = "
import std.meta as meta

@modifier
pub fn crash(d: meta.Decl) -> Result<meta.Additions, meta.Diagnostic> {
    let xs: List<Int> = []
    return Ok(meta.Additions.new(\"${xs.get(3)}\", \"\"))
}
";
    let mut stage0 = build("crash", module, "crash").unwrap();
    let err = stage0.apply(&decl("struct P {\n}\n", "P")).unwrap_err();
    assert!(err.starts_with("modifier `crash` failed"), "{err}");
}

#[test]
fn mote_test_calls_a_modifier_on_a_hand_built_decl() {
    let dir = workdir("mote_test");
    std::fs::write(dir.join("mods.mote"), DESCRIBE).unwrap();
    let test = "
import std.meta as meta
import .mods

test \"describe lists the fields\" {
    let x = meta.Field.new(\"x\", meta.TypeRef.new(meta.TypeKind.Named, \"Int\"))
    let ys = meta.Field.new(\"ys\", meta.TypeRef.new(meta.TypeKind.Generic, \"List\", [meta.TypeRef.new(meta.TypeKind.Named, \"Int\")]))
    let d = meta.Decl.new(meta.DeclKind.Struct, \"P\", fields = [x, ys])
    let out = mods.describe(d)!
    assert_eq(out.public, \"fn shape(self) -> String { return \\\"P(x:Int,ys:List<Int>)\\\" }\")
}

test \"a decl read from JSON is the same decl\" {
    let d = meta.parse_decl(\"{\\\"kind\\\":\\\"class\\\",\\\"name\\\":\\\"C\\\",\\\"pub\\\":true,\\\"doc\\\":\\\"\\\",\\\"generics\\\":[],\\\"traits\\\":[],\\\"fields\\\":[],\\\"variants\\\":[],\\\"methods\\\":[],\\\"attrs\\\":[]}\")!
    assert_eq(d.name, \"C\")
}
";
    std::fs::write(dir.join("t.mote"), test).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_mote")).arg("test").arg(dir.join("t.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success() && text.contains("2 passed, 0 failed"), "{text}");
}

#[test]
fn the_cost_of_fifty_declarations() {
    let started = std::time::Instant::now();
    let mut stage0 = build("cost", DESCRIBE, "describe").unwrap();
    let compiled = started.elapsed();
    let source: String = (0..50).map(|n| format!("class C{n} {{\n    a: Int\n    b: String\n    c: List<Int>\n}}\n")).collect();
    let tokens = compiler::lexer::Lexer::new(&source).tokenize().unwrap();
    let program = compiler::parser::Parser::new(tokens).parse().unwrap();
    let started = std::time::Instant::now();
    for n in 0..50 {
        let json = compiler::modifiers::declaration_json(&program, &format!("C{n}")).unwrap();
        stage0.apply(&json).unwrap();
    }
    let applied = started.elapsed();
    eprintln!("stage 0: compile {compiled:?}, 50 applications {applied:?} ({:?} each)", applied / 50);
}
