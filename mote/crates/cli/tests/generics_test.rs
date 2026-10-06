//! Type parameters, opaque inside their declaration, inferred and substituted at a call.

use std::process::Command;

fn run(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_generics_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn rejected(source: &str, tag: &str, message: &str) {
    let (ok, text) = run(source, tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

#[test]
fn a_generic_function_is_inferred_from_its_arguments() {
    let src = "fn id<T>(x: T) -> T { return x }\nfn first<T>(xs: List<T>) -> T { return xs.get(0) }\nfn pick<A, B>(a: A, b: B) -> A { return a }\nfn main() {\n    let a: Int = id(5)\n    let b = first([10, 20])\n    let s: String = id(\"hi\")\n    println(a + b + pick(3, \"x\"))\n    println(s)\n}\n";
    let (ok, text) = run(src, "infer");
    assert!(ok && text.trim() == "18\nhi", "{text}");
}

#[test]
fn the_result_carries_the_bound_type() {
    rejected("fn id<T>(x: T) -> T { return x }\nfn main() {\n    let a: String = id(5)\n}\n", "result", "`id` argument 1 expects 'String', found 'Int'");
}

#[test]
fn a_type_parameter_accepts_only_itself_inside_its_declaration() {
    rejected("fn bad<T>(x: T) -> T { return 5 }\nfn main() { }\n", "opaque", "function returns");
}

#[test]
fn a_type_parameter_has_no_fields_or_methods_without_a_bound() {
    rejected("fn f<T>(x: T) -> Int { return x.len() }\nfn main() { }\n", "method", "type parameter with no bound, so it has no method `.len()`");
    rejected("fn f<T>(x: T) -> Int { return x.n }\nfn main() { }\n", "field", "type parameter with no bound, so it has no field 'n'");
}

#[test]
fn to_string_works_on_a_type_parameter() {
    let (ok, text) = run("fn size<T>(x: T) -> Int { return x.to_string().len() }\nfn main() { println(size(12345)) }\n", "tostring");
    assert!(ok && text.trim() == "5", "{text}");
}

#[test]
fn an_operator_on_a_type_parameter_is_guarded_at_runtime() {
    let src = "fn add<T>(a: T, b: T) -> T { return a + b }\nfn main() {\n    println(add(1, 2))\n    println(add(\"a\", \"b\"))\n    println(add(true, false))\n}\n";
    let (ok, text) = run(src, "guard");
    assert!(!ok && text.starts_with("3\nab\n") && text.contains("cannot apply `+`"), "{text}");
}

#[test]
fn arguments_that_disagree_on_one_parameter_are_a_mismatch() {
    rejected("fn add<T>(a: T, b: T) -> T { return a }\nfn main() { println(add(1, \"b\")) }\n", "disagree", "`add` argument 2 expects 'Int', found 'String'");
}
#[test]
fn a_bound_naming_no_trait_is_refused() {
    rejected("fn show<T: Nope>(x: T) -> T { return x }\nfn main() { }\n", "bound", "`Nope` is not a trait");
}

#[test]
fn a_method_may_have_its_own_type_parameters() {
    let src = "class Tool {\n    n: Int\n    pub fn echo<T>(self, x: T) -> T { return x }\n}\nfn main() {\n    let t = Tool { n: 1 }\n    let v: Int = t.echo(4)\n    println(v)\n}\n";
    let (ok, text) = run(src, "own_params");
    assert!(ok && text.trim() == "4", "{text}");
}

const BOX: &str = "class Box<T> {\n    v: T\n    pub fn get(self) -> T { return self.v }\n    pub fn make(x: T) -> Box<T> { return Box { v: x } }\n}\n";

fn with_box(main: &str) -> String {
    format!("{BOX}fn main() {{\n{main}\n}}\n")
}

#[test]
fn a_generic_class_literal_infers_its_arguments() {
    let (ok, text) = run(&with_box("    let b = Box { v: 3 }\n    let n: Int = b.v\n    let m: Int = b.get()\n    println(n + m)"), "lit");
    assert!(ok && text.trim() == "6", "{text}");
}

#[test]
fn a_generic_class_is_invariant_in_its_arguments() {
    rejected(&with_box("    let b: Box<String> = Box { v: 3 }"), "invariant", "Box<");
}

#[test]
fn a_bare_generic_name_is_an_error() {
    rejected(&with_box("    let b: Box<Int> = Box { v: 3 }\n    let c: Box = b\n    println(c.v)"), "bare", "`Box` needs its type arguments, as in `Box<T>`");
}

#[test]
fn a_wrong_type_argument_count_is_an_error() {
    rejected(&with_box("    let b: Box<Int, Int> = Box { v: 3 }"), "arity", "Box");
}

#[test]
fn a_static_method_infers_the_class_parameter() {
    let (ok, text) = run(&with_box("    let b = Box.make(3)\n    let n: Int = b.get()\n    println(n)"), "static");
    assert!(ok && text.trim() == "3", "{text}");
}

#[test]
fn a_generic_struct_works() {
    let src = "struct Pair<A, B> {\n    a: A\n    b: B\n}\nfn main() {\n    let p = Pair { a: 1, b: \"x\" }\n    let n: Int = p.a\n    let s: String = p.b\n    println(n)\n    println(s)\n}\n";
    let (ok, text) = run(src, "struct");
    assert!(ok && text.trim() == "1\nx", "{text}");
}

#[test]
fn a_generic_function_infers_from_a_generic_class_argument() {
    let src = format!("{BOX}fn unwrap<T>(b: Box<T>) -> T {{ return b.v }}\nfn main() {{\n    let n: Int = unwrap(Box {{ v: 5 }})\n    println(n)\n}}\n");
    let (ok, text) = run(&src, "fn_over_box");
    assert!(ok && text.trim() == "5", "{text}");
}

#[test]
fn a_field_read_has_the_substituted_type() {
    rejected(&with_box("    let b = Box { v: 3 }\n    let s: String = b.v"), "field_type", "String");
}

#[test]
fn a_generic_immutable_class_is_sendable_only_for_sendable_arguments() {
    let cls = "class Cell<T> {\n    v: T\n}\n";
    let ok_src = format!("{cls}fn main() {{\n    let c = Cell {{ v: 1 }}\n    let t = spawn {{ c.v }}\n    println(t.join().unwrap())\n}}\n");
    let (ok, text) = run(&ok_src, "send_ok");
    assert!(ok && text.trim() == "1", "{text}");
    let bad_src = format!("{cls}fn main() {{\n    let c = Cell {{ v: [1] }}\n    let t = spawn {{ c.v }}\n}}\n");
    rejected(&bad_src, "send_bad", "Sendable");
}

#[test]
fn some_carries_its_payload_type() {
    let src = "fn main() {\n    let o = Some(5)\n    let n: Int = o.unwrap()\n    println(n)\n}\n";
    let (ok, text) = run(src, "some_int");
    assert!(ok && text.trim() == "5", "{text}");
    rejected("fn main() {\n    let o = Some(5)\n    let s: String = o.unwrap()\n}\n", "some_str", "String");
}

#[test]
fn an_option_annotation_checks_the_payload() {
    rejected("fn main() {\n    let o: Option<String> = Some(5)\n}\n", "opt_mismatch", "Option");
}

#[test]
fn try_yields_the_result_payload_type() {
    let src = "fn parse(s: String) -> Result<Int, String> {\n    if s == \"\" { return Err(\"empty\") }\n    return Ok(s.len())\n}\nfn twice(s: String) -> Result<Int, String> {\n    let n: Int = parse(s)?\n    return Ok(n * 2)\n}\nfn main() {\n    println(twice(\"abc\").unwrap())\n}\n";
    let (ok, text) = run(src, "try_payload");
    assert!(ok && text.trim() == "6", "{text}");
    rejected("fn parse(s: String) -> Result<Int, String> { return Ok(1) }\nfn f(s: String) -> Result<Int, String> {\n    let n: String = parse(s)?\n    return Ok(1)\n}\nfn main() { }\n", "try_bad", "String");
}

#[test]
fn match_binds_the_payload_type() {
    let src = "fn main() {\n    let o = Some(7)\n    match o {\n        Some(n) => { let m: Int = n\n println(m) }\n        None => println(0)\n    }\n}\n";
    let (ok, text) = run(src, "match_payload");
    assert!(ok && text.trim() == "7", "{text}");
}

#[test]
fn list_and_channel_reads_give_typed_options() {
    let src = "fn main() {\n    let xs = [1, 2]\n    let n: Int = xs.first().unwrap()\n    println(n)\n}\n";
    let (ok, text) = run(src, "list_first");
    assert!(ok && text.trim() == "1", "{text}");
}

#[test]
fn a_bare_option_is_an_error() {
    let src = "fn f(o: Option) -> Int { return o.unwrap() }\nfn main() { println(f(Some(3))) }\n";
    rejected(src, "bare_option", "`Option` needs its type argument, as in `Option<T>` or `T?`");
}

#[test]
fn a_user_generic_enum_works() {
    let src = "enum Either<L, R> { Left(L), Right(R) }\nfn main() {\n    let e: Either<Int, String> = Either.Left(1)\n    match e {\n        Left(n) => { let m: Int = n\n println(m) }\n        Right(s) => println(s)\n    }\n}\n";
    let (ok, text) = run(src, "either");
    assert!(ok && text.trim() == "1", "{text}");
}

#[test]
fn a_result_keeps_its_payload_class() {
    let src = "class P {\n    x: Int\n}\nfn make() -> Result<P, String> { return Ok(P { x: 4 }) }\nfn main() {\n    let p = make().unwrap()\n    println(p.x)\n}\n";
    let (ok, text) = run(src, "result_class");
    assert!(ok && text.trim() == "4", "{text}");
}

const STACK: &str = include_str!("programs_mote/generic_stack.mote");

#[test]
fn done_bar_a_generic_stack_and_helpers_run() {
    let (ok, text) = run(STACK, "stack_program");
    assert!(ok && text == "2\n1\n7\nx\n1\n", "{text}");
}

fn stack_with(main_body: &str) -> String {
    STACK.replace("fn main() {", &format!("fn main() {{\n{main_body}"))
}

#[test]
fn done_bar_pushing_the_wrong_type_is_rejected() {
    rejected(&stack_with("    let t: Stack<Int> = Stack.new()\n    t.push(\"a\")"), "stack_push", "expects 'Int', found 'String'");
}

#[test]
fn done_bar_reading_the_wrong_type_is_rejected() {
    rejected(&stack_with("    let t: Stack<Int> = Stack.new()\n    let s: String = t.pop().unwrap()"), "stack_pop", "String");
}

#[test]
fn done_bar_mixed_stacks_are_rejected() {
    rejected(&stack_with("    let t: Stack<Int> = Stack.new()\n    let u: Stack<String> = t"), "stack_mix", "Stack");
}

#[test]
fn done_bar_a_helper_binds_one_parameter_once() {
    rejected(&stack_with("    let t: Stack<Int> = Stack.new()\n    println(top_or(t, \"none\"))"), "stack_fallback", "`top_or` argument 2 expects 'Int', found 'String'");
}

#[test]
fn std_payloads_are_typed_without_annotations() {
    let src = "import std.sys.fs as fs\nimport { Date } from std.date\nfn main() {\n    let names: Map<String, String> = {\"k\": \"txt\"}\n    match fs.create(\"g5_out.txt\") {\n        Ok(f) => {\n            f.write_text(\"hello\\n\")\n            f.close()\n        }\n        Err(e) => println(e.message)\n    }\n    println(fs.stat(\"g5_out.txt\").unwrap().size)\n    println(Date.new(2024, 3, 9).unwrap().add_months(1).to_iso())\n    println(names.get(\"k\").len())\n}\n";
    let file = std::env::temp_dir().join(format!("mote_generics_out_{}.txt", std::process::id()));
    let (ok, text) = run(&src.replace("g5_out.txt", file.to_str().unwrap()), "std_typed");
    std::fs::remove_file(&file).ok();
    assert!(ok && text == "6\n2024-04-09\n3\n", "{text}");
}

#[test]
fn a_std_payload_of_the_wrong_type_is_rejected() {
    rejected("import std.sys.fs as fs\nfn main() {\n    let s: String = fs.stat(\"x\").unwrap().size\n}\n", "std_wrong", "String");
}
