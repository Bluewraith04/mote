use compiler::*;
mod common;
use common::Compiler;

#[test]
fn test_lexer_tokenization() {
    let source = "let x = 42\nvar y = 3.14\nfn add(a: Int, b: Int) -> Int {\n    return a + b\n}\n";
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().expect("Tokenization failed");
    assert!(!tokens.is_empty());
    assert!(tokens.iter().any(|t| matches!(t.kind, TokenKind::Let)));
    assert!(tokens.iter().any(|t| matches!(t.kind, TokenKind::Fn)));
    assert!(tokens.iter().any(|t| matches!(t.kind, TokenKind::Newline)));
}

#[test]
fn test_parser_ast_construction() {
    let source = "fn compute(x: Int) -> Int {\n    let y = x * 2\n    return y + 1\n}\n";
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().unwrap();
    let mut parser = Parser::new(tokens);
    let program = parser.parse().expect("Parse failed");
    assert_eq!(program.items.len(), 1);
    if let ast::Item::Function(f) = &program.items[0] {
        assert_eq!(f.name, "compute");
        assert_eq!(f.params.len(), 1);
        assert_eq!(f.body.len(), 2);
    } else {
        panic!("Expected Function item");
    }
}

#[test]
fn test_type_inference() {
    let source = "let a = 100\nvar b = 20.5\n";
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().unwrap();
    let mut parser = Parser::new(tokens);
    let program = parser.parse().unwrap();
    let mut checker = TypeChecker::new();
    assert!(checker.check_program(&program).is_ok());
}

#[test]
fn test_checker_records_expression_types() {
    let source = "struct P {\n    x: Int\n}\nfn main() -> Int {\n    let p = P { x: 5 }\n    return p.x\n}\n";
    let mut parser = Parser::new(Lexer::new(source).tokenize().unwrap());
    let program = parser.parse().unwrap();
    let mut checker = TypeChecker::new();
    checker.check_program(&program).unwrap();

    assert!(!checker.expr_types.is_empty());
    assert!(checker
        .expr_types
        .values()
        .any(|t| matches!(t, Type::Struct { name, .. } if name == "P")));
}

#[test]
fn test_a_struct_holding_a_mutable_class_is_rejected() {
    let source = "class Node {\n    var val: Int\n}\nstruct BadPoint {\n    node: Node\n}\n";
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().unwrap();
    let mut parser = Parser::new(tokens);
    let program = parser.parse().unwrap();
    let mut checker = TypeChecker::new();
    let result = checker.check_program(&program);
    assert!(result.is_err(), "Expected a struct field that can change to be rejected");
    let err_str = format!("{:?}", result.unwrap_err());
    assert!(err_str.contains("which can change; make `BadPoint` a class"));
}

#[test]
fn test_mutability_rejection() {
    let source = "let x = 10\nx = 20\n";
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().unwrap();
    let mut parser = Parser::new(tokens);
    let program = parser.parse().unwrap();
    let mut checker = TypeChecker::new();
    let result = checker.check_program(&program);
    assert!(result.is_err(), "Expected Mutability Violation");
    let err_str = format!("{:?}", result.unwrap_err());
    assert!(err_str.contains("Mutability Violation"));
}

fn eval_main(body: &str) -> i64 {
    let source = format!("fn main() -> Int {{\n{body}\n}}\n");
    Compiler::run(&source)
        .unwrap_or_else(|e| panic!("program failed:\n{e}\n--- source ---\n{source}"))
        .as_int()
        .expect("main did not return an Int")
}

fn run_top(source: &str) -> i64 {
    Compiler::run(source)
        .unwrap_or_else(|e| panic!("program failed:\n{e}\n--- source ---\n{source}"))
        .as_int()
        .expect("program result was not an Int")
}

#[test]
fn test_e2e_fn_main_is_invoked() {
    assert_eq!(eval_main("return 42"), 42);
}

#[test]
fn test_e2e_multi_argument_call() {
    let src = "fn add3(a: Int, b: Int, c: Int) -> Int {\n    return a + b + c\n}\nfn main() -> Int {\n    return add3(2, 40, 100)\n}\n";
    assert_eq!(run_top(src), 142);
}

#[test]
fn test_e2e_match_literal_arms() {
    let body = "var r = 0\nlet n = 2\nmatch n {\n    1 => { r = 10 }\n    2 => { r = 20 }\n    _ => { r = 99 }\n}\nreturn r";
    assert_eq!(eval_main(body), 20);
    let miss = "var r = 0\nlet n = 7\nmatch n {\n    1 => { r = 10 }\n    _ => { r = 99 }\n}\nreturn r";
    assert_eq!(eval_main(miss), 99);
}

#[test]
fn test_e2e_match_or_and_range_patterns() {
    let prog = |n: i64| {
        format!("var r = 0\nlet n = {n}\nmatch n {{\n    1 | 2 | 3 => {{ r = 100 }}\n    10..20 => {{ r = 200 }}\n    20..=30 => {{ r = 300 }}\n    _ => {{ r = 0 }}\n}}\nreturn r")
    };
    assert_eq!(eval_main(&prog(2)), 100);
    assert_eq!(eval_main(&prog(15)), 200);
    assert_eq!(eval_main(&prog(20)), 300);
    assert_eq!(eval_main(&prog(99)), 0);
}

#[test]
fn test_e2e_match_binding_and_guard() {
    assert_eq!(eval_main("let n = 21\nmatch n {\n    x => { return x + x }\n}\n"), 42);
    let guarded = "var r = 0\nlet n = 8\nmatch n {\n    x if x > 5 => { r = 1 }\n    _ => { r = 0 }\n}\nreturn r";
    assert_eq!(eval_main(guarded), 1);
}

#[test]
fn test_match_exhaustiveness_is_checked() {
    let err = Compiler::compile("fn main() -> Int {\n    let n = 1\n    match n {\n        1 => { return 1 }\n        2 => { return 2 }\n    }\n    return 0\n}\n", "x").unwrap_err();
    assert!(err.contains("Non-exhaustive"), "got: {err}");
}

#[test]
fn test_match_tuple_pattern_against_non_tuple_is_rejected_clearly() {
    let err = Compiler::compile("fn main() -> Int {\n    let n = 1\n    match n {\n        (a, b) => { return a }\n        _ => { return 0 }\n    }\n    return 0\n}\n", "x").unwrap_err();
    assert!(err.contains("cannot match a tuple pattern against"), "got: {err}");
}

#[test]
fn test_match_on_bool_is_exhaustive_without_wildcard() {
    let body = "let b = true\nmatch b {\n    true => { return 1 }\n    false => { return 0 }\n}\n";
    assert_eq!(eval_main(body), 1);
}

#[test]
fn test_e2e_nested_field_access() {
    let src = "struct Inner {\n    a: Int\n    b: Int\n}\nstruct Outer {\n    p: Inner\n    q: Int\n}\nfn main() -> Int {\n    let o = Outer { p: Inner { a: 10, b: 20 }, q: 99 }\n    return o.p.b + o.q\n}\n";
    assert_eq!(run_top(src), 119);
}

#[test]
fn test_e2e_call_result_field_access() {
    let src = "struct Point {\n    x: Int\n    y: Int\n}\nfn origin_offset() -> Point {\n    return Point { x: 7, y: 3 }\n}\nfn main() -> Int {\n    return origin_offset().y\n}\n";
    assert_eq!(run_top(src), 3);
}

#[test]
fn test_call_arity_is_checked() {
    let err = Compiler::compile("fn add(a: Int, b: Int) -> Int {\n    return a + b\n}\nfn main() -> Int {\n    return add(1)\n}\n", "x").unwrap_err();
    assert!(err.contains("takes 2 argument"), "got: {err}");
}

#[test]
fn test_return_type_is_checked() {
    let err = Compiler::compile("fn bad() -> Int {\n    return true\n}\n", "x").unwrap_err();
    assert!(err.contains("function returns") && err.contains("Bool"), "got: {err}");
}

#[test]
fn test_e2e_struct_param_field_access() {
    let src = "struct Point {\n    x: Int\n    y: Int\n}\nfn dist_sq(a: Point, b: Point) -> Int {\n    let dx = a.x - b.x\n    let dy = a.y - b.y\n    return dx * dx + dy * dy\n}\nfn main() -> Int {\n    let p = Point { x: 3, y: 4 }\n    let q = Point { x: 0, y: 0 }\n    return dist_sq(p, q)\n}\n";
    assert_eq!(run_top(src), 25);
}

#[test]
fn test_e2e_nested_call_arguments() {
    let src = "fn mul(x: Int, y: Int) -> Int {\n    return x * y\n}\nfn add3(a: Int, b: Int, c: Int) -> Int {\n    return a + b + c\n}\nfn main() -> Int {\n    return mul(add3(1, 2, 3), mul(2, 5))\n}\n";
    assert_eq!(run_top(src), 60);
}

#[test]
fn test_e2e_call_in_loop_accumulator() {
    let src = "fn sq(n: Int) -> Int { return n * n }\nvar total = 0\nfor i in 1..5 {\n    total += sq(i)\n}\n";
    assert_eq!(run_top(src), 30);
}

#[test]
fn test_e2e_top_level_variable_is_the_result() {
    assert_eq!(run_top("var r = 0\nr = 3 + 4 * 5\n"), 23);
    assert_eq!(run_top("var acc = 0\nvar i = 0\nwhile i < 6 {\n    acc = acc + i\n    i = i + 1\n}\n"), 15);
}

#[test]
fn test_e2e_arithmetic_precedence() {
    assert_eq!(eval_main("let a = 10\nlet b = 20\nreturn a + b * 2"), 50);
}

#[test]
fn test_e2e_while_loop_accumulate() {
    let body = "var result = 0\nvar i = 0\nwhile i < 5 {\n    result = result + i\n    i = i + 1\n}\nreturn result";
    assert_eq!(eval_main(body), 1 + 2 + 3 + 4);
}

#[test]
fn test_e2e_compound_assignment() {
    assert_eq!(eval_main("var sum = 0\nfor i in 0..5 {\n    sum += i\n}\nreturn sum"), 10);
    assert_eq!(eval_main("var n = 3\nn *= 4\nreturn n"), 12);
    assert_eq!(eval_main("var n = 20\nn -= 6\nreturn n"), 14);
}

#[test]
fn test_e2e_bitwise_and_shift() {
    assert_eq!(eval_main("return 6 & 3"), 2);
    assert_eq!(eval_main("return 5 | 2"), 7);
    assert_eq!(eval_main("return 5 ^ 1"), 4);
    assert_eq!(eval_main("return 1 << 4"), 16);
    assert_eq!(eval_main("return 64 >> 2"), 16);
}

#[test]
fn test_e2e_break_in_while() {
    let body = "var i = 0\nwhile i < 100 {\n    if i == 5 {\n        break\n    }\n    i = i + 1\n}\nreturn i";
    assert_eq!(eval_main(body), 5);
}

#[test]
fn test_e2e_break_in_for() {
    let body = "var last = 0\nfor i in 0..100 {\n    last = i\n    if i == 7 {\n        break\n    }\n}\nreturn last";
    assert_eq!(eval_main(body), 7);
}

#[test]
fn test_e2e_continue_in_while() {
    let body = "var sum = 0\nvar i = 0\nwhile i < 5 {\n    i = i + 1\n    if i == 3 {\n        continue\n    }\n    sum = sum + i\n}\nreturn sum";
    assert_eq!(eval_main(body), 1 + 2 + 4 + 5);
}

#[test]
fn test_e2e_continue_in_for() {
    let body = "var sum = 0\nfor i in 0..5 {\n    if i == 2 {\n        continue\n    }\n    sum += i\n}\nreturn sum";
    assert_eq!(eval_main(body), 1 + 3 + 4);
}

#[test]
fn test_e2e_break_targets_innermost_loop() {
    let body = "var count = 0\nfor i in 0..3 {\n    for j in 0..100 {\n        if j == 2 {\n            break\n        }\n        count += 1\n    }\n}\nreturn count";
    assert_eq!(eval_main(body), 2 * 3);
}

#[test]
fn test_break_outside_loop_is_error() {
    let err = Compiler::compile("fn oops() {\n    break\n}\n", "x").unwrap_err();
    assert!(err.contains("outside a loop"), "got: {err}");
}

#[test]
fn test_e2e_struct_distinct_fields() {
    let prog = |field: &str| {
        format!(
            "struct Point {{\n    x: Int\n    y: Int\n}}\nfn main() -> Int {{\n    let p = Point {{ x: 3, y: 7 }}\n    return p.{field}\n}}\n"
        )
    };
    assert_eq!(Compiler::run(&prog("x")).unwrap().as_int(), Some(3));
    assert_eq!(Compiler::run(&prog("y")).unwrap().as_int(), Some(7));
}

#[test]
fn test_class_instantiation_compiles_and_runs() {
    let source = "class Counter {\n    count: Int\n}\nlet c = Counter { count: 0 }\n";
    assert!(Compiler::run(source).is_ok());
}

#[test]
fn test_struct_literal_field_names_are_checked() {
    let decl = "struct P {\n    x: Int\n    y: Int\n}\n";
    for (lit, want) in [
        ("P { x: 1 }", "`P` literal is missing field(s) `y`"),
        ("P { x: 1, y: 2, z: 3 }", "`P` has no field `z`"),
        ("P { x: 1, x: 2, y: 3 }", "field `x` is given twice in a `P` literal"),
    ] {
        let err = format!("{:?}", Compiler::run(&format!("{decl}let p = {lit}\n")).unwrap_err());
        assert!(err.contains(want), "{lit}: {err}");
    }
}

#[test]
fn test_e2e_class_instance_method_mutates_self() {
    let source = "\
class Counter {
    var count: Int

    fn inc(var self) {
        self.count = self.count + 1
    }

    fn get(self) -> Int {
        return self.count
    }
}
fn main() -> Int {
    let c = Counter { count: 0 }
    c.inc()
    c.inc()
    c.inc()
    return c.get()
}
";
    assert_eq!(run_top(source), 3);
}

#[test]
fn test_e2e_class_static_method_call() {
    let source = "\
class Rng {
    var seed: Int

    fn seeded(n: Int) -> Rng {
        return Rng { seed: n }
    }

    fn value(self) -> Int {
        return self.seed
    }
}
fn main() -> Int {
    let r = Rng.seeded(42)
    return r.value()
}
";
    assert_eq!(run_top(source), 42);
}

#[test]
fn test_class_unknown_method_is_a_checker_error() {
    let source = "\
class Counter {
    var count: Int
}
fn main() -> Int {
    let c = Counter { count: 0 }
    c.bogus_method()
    return 0
}
";
    let mut parser = Parser::new(Lexer::new(source).tokenize().unwrap());
    let program = parser.parse().unwrap();
    let mut checker = TypeChecker::new();
    let result = checker.check_program(&program);
    assert!(result.is_err(), "expected an unknown-method checker error");
    let err_str = format!("{:?}", result.unwrap_err());
    assert!(err_str.contains("bogus_method"), "error should name the unknown method: {err_str}");
}

#[test]
fn test_removed_arena_block_is_a_clear_parse_error() {
    let source = "let x = 1\narena {\n    let a = 42\n}\n";
    let tokens = Lexer::new(source).tokenize().unwrap();
    let err = Parser::new(tokens).parse().unwrap_err();
    assert!(format!("{err:?}").contains("`arena { }` was removed"), "{err:?}");
}

#[test]
fn test_a_region_placed_struct_compiles_and_runs() {
    let source = "struct Point {\n    x: Int\n    y: Int\n}\nfn f() -> Int {\n    let p = Point { x: 10, y: 20 }\n    return p.x + p.y\n}\nf()\n";
    assert!(Compiler::run(source).is_ok());
}

#[test]
fn test_e2e_block_shadowing_restores_outer_binding() {
    let body = "let x = 1\nif x == 1 {\n    let x = 99\n}\nreturn x";
    assert_eq!(eval_main(body), 1);
}

#[test]
fn test_e2e_inner_block_locals_do_not_exhaust_registers() {
    let mut body = String::from("var acc = 0\n");
    for i in 0..400 {
        body.push_str(&format!("if acc >= 0 {{\n    let t{i} = {i}\n    acc = acc + 1\n}}\n"));
    }
    body.push_str("return acc");
    assert_eq!(eval_main(&body), 400);
}

#[test]
fn test_unsupported_expression_is_a_clean_codegen_error() {
    let err = Compiler::compile(
        "fn main() -> Int {\n    let r = 1..3\n    return 0\n}\n",
        "x",
    )
    .unwrap_err();
    assert!(err.contains("not supported by codegen"), "got: {err}");
}

#[test]
fn test_recursive_fib_compiles_and_runs() {
    let source = "fn fib(n: Int) -> Int {\n    if n < 2 {\n        return n\n    }\n    return fib(n - 1) + fib(n - 2)\n}\nlet res = fib(10)\n";
    assert_eq!(Compiler::run(source).unwrap().as_int(), Some(55));
}

#[test]
fn test_e2e_string_literal_is_a_heap_string() {
    let long = "Hello from my-mote-package";
    let v = Compiler::run(&format!("fn main() -> String {{\n    return \"{long}\"\n}}\nreturn main()\n"))
        .expect("string program should run");
    assert_eq!(v.as_heap_string().as_deref(), Some(long));

    let v = Compiler::run("return \"\"\n").unwrap();
    assert_eq!(v.as_heap_string().as_deref(), Some(""));
    let v = Compiler::run("return \"hi\"\n").unwrap();
    assert_eq!(v.as_heap_string().as_deref(), Some("hi"));
}

#[test]
fn test_e2e_string_survives_a_gc_cycle() {
    let src = r#"
class Box { n: Int }
fn main() -> String {
    let kept = "the-string-that-must-survive"
    var i = 0
    while i < 500 {
        let junk = Box { n: i }
        i = i + 1
    }
    return kept
}
return main()
"#;
    let v = Compiler::run(src).expect("gc-churn string program should run");
    assert_eq!(v.as_heap_string().as_deref(), Some("the-string-that-must-survive"));
}

#[test]
fn test_e2e_list_literal_len_and_get() {
    assert_eq!(eval_main("let xs = [10, 20, 30]\nreturn xs.len()"), 3);
    assert_eq!(eval_main("let xs = [10, 20, 30]\nreturn xs.get(1)"), 20);
    assert_eq!(eval_main("let xs = [10, 20, 30]\nreturn xs.get(0) + xs.get(2)"), 40);
}

#[test]
fn test_e2e_list_push_pop_set_clear() {
    assert_eq!(eval_main("var xs = [1]\nxs.push(2)\nxs.push(3)\nreturn xs.len()"), 3);
    assert_eq!(eval_main("var xs = [1]\nxs.push(2)\nxs.push(3)\nreturn xs.get(2)"), 3);
    assert_eq!(eval_main("var xs = [7, 8]\nlet p = xs.pop().unwrap()\nreturn p + xs.len()"), 9);
    assert_eq!(eval_main("var xs = [1, 2, 3]\nxs.set(1, 99)\nreturn xs.get(1)"), 99);
    assert_eq!(eval_main("var xs = [1, 2, 3]\nxs.clear()\nreturn xs.len()"), 0);
}

#[test]
fn test_e2e_list_is_empty() {
    assert_eq!(eval_main("let xs: List<Int> = []\nif xs.is_empty() {\n    return 1\n}\nreturn 0"), 1);
    assert_eq!(eval_main("let xs = [1]\nif xs.is_empty() {\n    return 1\n}\nreturn 0"), 0);
}

#[test]
fn test_e2e_list_grows_across_a_gc_cycle() {
    let body = "\
var xs: List<Int> = []
for i in 0..200 {
    xs.push(i * 2)
}
return xs.get(199) + xs.len()";
    assert_eq!(eval_main(body), 199 * 2 + 200);
}

#[test]
fn test_e2e_list_get_out_of_bounds_is_a_clean_error() {
    let err = Compiler::run("fn main() -> Int {\n    let xs = [1, 2]\n    return xs.get(5)\n}\nreturn main()\n")
        .unwrap_err();
    assert!(err.contains("out of bounds"), "got: {err}");
}

#[test]
fn test_e2e_list_element_type_is_checked() {
    let err = Compiler::compile("fn main() {\n    var xs = [1, 2]\n    xs.push(\"nope\")\n}\n", "x")
        .unwrap_err();
    assert!(err.to_lowercase().contains("push"), "got: {err}");
}

#[test]
fn test_e2e_for_in_list() {
    assert_eq!(
        eval_main("let xs = [3, 4, 5, 6]\nvar sum = 0\nfor x in xs {\n    sum = sum + x\n}\nreturn sum"),
        18,
    );
    assert_eq!(
        eval_main("let xs: List<Int> = []\nvar n = 0\nfor x in xs {\n    n = n + 1\n}\nreturn n"),
        0,
    );
    assert_eq!(
        eval_main("var last = 0\nfor x in [10, 20, 30, 40] {\n    if x == 30 {\n        break\n    }\n    last = x\n}\nreturn last"),
        20,
    );
}

#[test]
fn test_e2e_for_in_list_after_growth() {
    let body = "\
var xs: List<Int> = []
for i in 0..60 {
    xs.push(i)
}
var sum = 0
for x in xs {
    sum = sum + x
}
return sum";
    assert_eq!(eval_main(body), (0..60).sum::<i64>());
}

#[test]
fn test_e2e_list_pop_first_last_are_option() {
    assert_eq!(eval_main("var xs = [7, 8, 9]\nreturn xs.pop().unwrap()"), 9);
    assert_eq!(eval_main("let xs = [7, 8, 9]\nreturn xs.first().unwrap()"), 7);
    assert_eq!(eval_main("let xs = [7, 8, 9]\nreturn xs.last().unwrap()"), 9);
    assert_eq!(
        eval_main("let xs: List<Int> = []\nif xs.pop().is_none() {\n    return 1\n}\nreturn 0"),
        1,
    );
    assert_eq!(eval_main("let xs: List<Int> = []\nreturn xs.first().unwrap_or(42)"), 42);
}

#[test]
fn test_e2e_for_in_non_list_is_a_clean_compile_error() {
    let err = Compiler::compile("fn main() {\n    for x in 5 {\n        let y = x\n    }\n}\n", "x")
        .unwrap_err();
    assert!(err.contains("iter"), "got: {err}");
}

#[test]
fn test_e2e_map_literal_get_set_len() {
    assert_eq!(eval_main("let m = {\"a\": 1, \"b\": 2}\nreturn m.get(\"b\")"), 2);
    assert_eq!(eval_main("let m = {\"a\": 1, \"b\": 2, \"c\": 3}\nreturn m.len()"), 3);
    assert_eq!(eval_main("var m: Map<String, Int> = {}\nm.set(\"x\", 7)\nm.set(\"x\", 9)\nreturn m.get(\"x\") + m.len()"), 10);
    assert_eq!(eval_main("var m = {\"a\": 1}\nreturn m.get_or(\"missing\", 42)"), 42);
}

#[test]
fn test_e2e_map_contains_key_and_remove() {
    let body = "\
var m = {\"a\": 1, \"b\": 2, \"c\": 3}
var out = 0
if m.contains_key(\"b\") { out = out + 1 }
let was = m.remove(\"b\")
if was { out = out + 10 }
if m.contains_key(\"b\") == false { out = out + 100 }
return out + m.len()";
    assert_eq!(eval_main(body), 1 + 10 + 100 + 2);
}

#[test]
fn test_e2e_map_get_missing_key_is_a_clean_error() {
    let err = Compiler::run("fn main() -> Int {\n    let m = {\"a\": 1}\n    return m.get(\"nope\")\n}\nreturn main()\n")
        .unwrap_err();
    assert!(err.contains("not present"), "got: {err}");
}

#[test]
fn test_e2e_map_grows_and_survives_gc() {
    let body = "\
var m: Map<Int, Int> = {}
for i in 0..200 {
    m.set(i, i * i)
}
var sum = 0
for k in m.keys() {
    sum = sum + m.get(k)
}
return sum + m.len()";
    let expect: i64 = (0..200i64).map(|i| i * i).sum::<i64>() + 200;
    assert_eq!(eval_main(body), expect);
}

#[test]
fn test_e2e_set_add_contains_remove() {
    assert_eq!(eval_main("var s: Set<Int> = Set()\ns.add(1)\ns.add(2)\ns.add(2)\ns.add(3)\nreturn s.len()"), 3);
    let body = "\
var s: Set<Int> = Set()
s.add(10)
s.add(20)
var out = 0
if s.contains(20) { out = out + 1 }
let removed = s.remove(20)
if removed { out = out + 10 }
if s.contains(20) == false { out = out + 100 }
return out + s.len()";
    assert_eq!(eval_main(body), 1 + 10 + 100 + 1);
}

#[test]
fn test_e2e_map_element_types_checked() {
    let err = Compiler::compile(
        "fn main() {\n    var m: Map<String, Int> = {}\n    m.set(\"k\", \"not an int\")\n}\n",
        "x",
    )
    .unwrap_err();
    assert!(err.to_lowercase().contains("value"), "got: {err}");
}

#[test]
fn test_e2e_bytes_push_get_set_len() {
    assert_eq!(eval_main("var b = Bytes()\nb.push(10)\nb.push(20)\nb.push(30)\nreturn b.len()"), 3);
    assert_eq!(eval_main("var b = Bytes()\nb.push(10)\nb.push(20)\nreturn b.get(1)"), 20);
    assert_eq!(eval_main("var b = Bytes()\nb.push(10)\nb.set(0, 99)\nreturn b.get(0)"), 99);
}

#[test]
fn test_e2e_bytes_byte_range_and_bounds_are_checked() {
    let err = Compiler::run("fn main() -> Int {\n    var b = Bytes()\n    b.push(300)\n    return 0\n}\nreturn main()\n")
        .unwrap_err();
    assert!(err.contains("0..=255"), "got: {err}");
    let err = Compiler::run("fn main() -> Int {\n    let b = Bytes()\n    return b.get(0)\n}\nreturn main()\n")
        .unwrap_err();
    assert!(err.contains("out of bounds"), "got: {err}");
}

#[test]
fn test_e2e_bytes_decode_is_option() {
    assert_eq!(
        eval_main("var b = Bytes()\nb.push(72)\nb.push(105)\nif b.decode().unwrap().len() == 2 {\n    return 1\n}\nreturn 0"),
        1,
    );
    assert_eq!(
        eval_main("var b = Bytes()\nb.push(255)\nb.push(254)\nif b.decode().is_none() {\n    return 1\n}\nreturn 0"),
        1,
    );
}

#[test]
fn test_e2e_bytes_extend_slice_and_grow_across_gc() {
    let body = "\
var b = Bytes()
for i in 0..300 {
    b.push(65)
}
var c = Bytes()
c.push(66)
c.push(66)
b.extend(c)
let s = b.slice(298, 302)
return b.len() * 10 + s.len() + s.get(3)";
    assert_eq!(eval_main(body), 302 * 10 + 4 + 66);
}

#[test]
fn test_e2e_bytes_n_is_n_zero_bytes() {
    assert_eq!(eval_main("let b = Bytes(5)\nreturn b.len()"), 5);
    assert_eq!(eval_main("let b = Bytes(3)\nreturn b.get(0) + b.get(1) + b.get(2)"), 0);
    assert_eq!(eval_main("let b = Bytes(0)\nif b.is_empty() { return 1 }\nreturn 0"), 1);
    assert_eq!(eval_main("let b = Bytes(2)\nb[1] = 7\nb.push(9)\nreturn b.len() * 100 + b[1] * 10 + b[2]"), 300 + 70 + 9);
}

#[test]
fn test_e2e_bytes_n_needs_a_non_negative_int() {
    let err = Compiler::run("fn main() -> Int {\n    let b = Bytes(0 - 1)\n    return b.len()\n}\nreturn main()\n").unwrap_err();
    assert!(err.contains("must not be negative"), "got: {err}");
    let err = Compiler::compile("let b = Bytes(\"x\")\n", "x").unwrap_err();
    assert!(err.contains("Bytes(n)"), "got: {err}");
    let err = Compiler::compile("let b = Bytes(1, 2)\n", "x").unwrap_err();
    assert!(err.contains("Bytes(n)"), "got: {err}");
}

#[test]
fn test_e2e_runaway_recursion_is_a_fault_not_an_oom() {
    let err = Compiler::run("fn f(n: Int) -> Int {\n    return f(n + 1) + 1\n}\nreturn f(0)\n").unwrap_err();
    assert!(err.contains("stack overflow"), "got: {err}");
}

#[test]
fn test_e2e_println_lowers_to_callnative_not_debugprint() {
    let compiled = Compiler::compile("fn main() {\n    println(\"x\")\n}\nreturn main()\n", "x").unwrap();
    let saw_callnative = compiled.code_objects.iter().any(|co| {
        co.instructions
            .iter()
            .any(|i| (i & 0xFF) as u8 == isa::opcode::Opcode::CALLNATIVEW as u8)
    });
    assert!(saw_callnative, "println should lower to CALLNATIVE");
    assert!(Compiler::run("fn main() -> Int {\n    println(\"ok\")\n    return 1\n}\nreturn main()\n").is_ok());
}

#[test]
fn test_e2e_named_function_as_a_value_is_called_indirectly() {
    let src = "\
fn dbl(n: Int) -> Int { return n * 2 }
fn main() -> Int {
    let g = dbl
    return g(21)
}
";
    assert_eq!(run_top(src), 42);

    let compiled = Compiler::compile(src, "x").unwrap();
    let saw_callv = compiled.code_objects.iter().any(|co| {
        co.instructions
            .iter()
            .any(|i| (i & 0xFF) as u8 == isa::opcode::Opcode::CALLV as u8)
    });
    assert!(saw_callv, "an indirect call should lower to CALLV");
}

#[test]
fn test_e2e_function_passed_as_argument_and_invoked() {
    let src = "\
fn inc(n: Int) -> Int { return n + 1 }
fn apply(f: (Int) -> Int, x: Int) -> Int { return f(x) }
fn main() -> Int { return apply(inc, 41) }
";
    assert_eq!(run_top(src), 42);
}

#[test]
fn test_e2e_function_value_survives_a_gc_cycle() {
    let src = "\
class Box { n: Int }
fn target() -> Int { return 7 }
fn main() -> Int {
    let g = target
    var i = 0
    while i < 500 {
        let junk = Box { n: i }
        i = i + 1
    }
    return g()
}
";
    assert_eq!(run_top(src), 7);
}

#[test]
fn test_e2e_calling_a_non_function_value_is_a_runtime_error() {
    let err = Compiler::run("fn main() -> Int {\n    let x = 5\n    return x(1)\n}\nreturn main()\n")
        .unwrap_err();
    assert!(err.contains("not a callable value"), "got: {err}");
}

#[test]
fn test_e2e_expression_bodied_lambda() {
    assert_eq!(eval_main("let d = |x| x * 2\n    return d(21)"), 42);
}

#[test]
fn test_e2e_lambda_passed_as_callback() {
    let src = "\
fn apply(f: (Int) -> Int, x: Int) -> Int { return f(x) }
fn main() -> Int { return apply(|n| n + 1, 41) }
";
    assert_eq!(run_top(src), 42);
}

#[test]
fn test_e2e_block_bodied_lambda_with_explicit_return() {
    assert_eq!(eval_main("let add = |a, b| { return a + b }\n    return add(20, 22)"), 42);
}

#[test]
fn test_e2e_nested_non_capturing_lambda() {
    let src = "\
fn main() -> Int {
    let outer = || {
        let inner = || 21
        return inner() + inner()
    }
    return outer()
}
";
    assert_eq!(run_top(src), 42);

    let compiled = Compiler::compile(src, "x").unwrap();
    assert_eq!(compiled.code_objects.len(), 5);
}

#[test]
fn test_e2e_closure_captures_a_local_by_value() {
    assert_eq!(
        eval_main("let base = 100\n    let f = |x| x + base\n    return f(1)"),
        101,
    );

    let compiled = Compiler::compile(
        "fn main() -> Int {\n    let base = 1\n    let f = || base\n    return f()\n}\n",
        "x",
    )
    .unwrap();
    let saw_getcapture = compiled.code_objects.iter().any(|co| {
        co.instructions
            .iter()
            .any(|i| (i & 0xFF) as u8 == isa::opcode::Opcode::GETCAPTURE as u8)
    });
    assert!(saw_getcapture, "a closure body should emit GETCAPTURE");
}

#[test]
fn test_e2e_closure_capture_of_a_var_is_live() {
    let src = "\
fn main() -> Int {
    var n = 10
    let f = || n
    n = 999
    return f() + n
}
";
    assert_eq!(run_top(src), 1998);
}

#[test]
fn test_e2e_closure_captures_multiple_and_a_param() {
    let src = "\
fn main() -> Int {
    let a = 3
    let b = 7
    let combine = |x| a * x + b
    return combine(5)
}
";
    assert_eq!(run_top(src), 22);
}

#[test]
fn test_e2e_closure_survives_a_gc_cycle() {
    let src = "\
class Box { n: Int }
fn main() -> Int {
    let captured = 37
    let f = |x| x + captured
    var i = 0
    while i < 500 {
        let junk = Box { n: i }
        i = i + 1
    }
    return f(5)
}
";
    assert_eq!(run_top(src), 42);
}

#[test]
fn test_e2e_lambda_survives_a_gc_cycle() {
    let src = "\
class Box { n: Int }
fn main() -> Int {
    let f = |x| x + 5
    var i = 0
    while i < 500 {
        let junk = Box { n: i }
        i = i + 1
    }
    return f(37)
}
";
    assert_eq!(run_top(src), 42);
}

#[test]
fn test_e2e_direct_calls_still_bypass_function_values() {
    let src = "fn sq(n: Int) -> Int { return n * n }\nfn main() -> Int { return sq(8) }\n";
    let compiled = Compiler::compile(src, "x").unwrap();
    let saw_callv = compiled.code_objects.iter().any(|co| {
        co.instructions
            .iter()
            .any(|i| (i & 0xFF) as u8 == isa::opcode::Opcode::CALLV as u8)
    });
    assert!(!saw_callv, "a direct call must not lower to CALLV");
    assert_eq!(run_top(src), 64);
}

#[test]
fn test_e2e_function_reads_a_module_global_regardless_of_source_order() {
    let src = "\
fn helper(x: Int) -> Int { return x + BASE }
let BASE = 100
fn main() -> Int { return helper(5) }
";
    assert_eq!(run_top(src), 105);

    let compiled = Compiler::compile(src, "x").unwrap();
    assert_eq!(compiled.global_count, 1);
    let ops: Vec<u8> = compiled
        .code_objects
        .iter()
        .flat_map(|co| co.instructions.iter().map(|i| (i & 0xFF) as u8))
        .collect();
    use isa::opcode::Opcode;
    assert!(ops.contains(&(Opcode::GETGLOBAL as u8)), "helper should GETGLOBAL BASE");
    assert!(ops.contains(&(Opcode::SETGLOBAL as u8)), "the top-level let should SETGLOBAL");
}

#[test]
fn test_e2e_mutable_global_var_is_shared_across_functions() {
    let src = "\
var counter = 3
fn peek() -> Int { return counter }
fn main() -> Int {
    return peek() + peek()
}
";
    assert_eq!(run_top(src), 6);
}

#[test]
fn test_e2e_writing_a_global_from_a_function_is_a_globals_frozen_error() {
    let source = "\
var counter = 0
fn bump() -> Int { counter = counter + 1  return counter }
";
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().unwrap();
    let mut parser = Parser::new(tokens);
    let program = parser.parse().unwrap();
    let mut checker = TypeChecker::new();
    let result = checker.check_program(&program);
    assert!(result.is_err(), "Expected a globals-frozen error");
    let err_str = format!("{:?}", result.unwrap_err());
    assert!(err_str.contains("global"), "got: {err_str}");
}

#[test]
fn test_e2e_compound_assign_on_a_global() {
    let src = "\
var total = 0
total += 10
total += 5
return total
";
    assert_eq!(run_top(src), 15);
}

#[test]
fn test_e2e_compound_assign_on_a_global_from_a_function_is_a_globals_frozen_error() {
    let source = "\
var total = 0
fn add(n: Int) -> Int { total += n  return total }
";
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().unwrap();
    let mut parser = Parser::new(tokens);
    let program = parser.parse().unwrap();
    let mut checker = TypeChecker::new();
    let result = checker.check_program(&program);
    assert!(result.is_err(), "Expected a globals-frozen error");
    let err_str = format!("{:?}", result.unwrap_err());
    assert!(err_str.contains("global"), "got: {err_str}");
}

#[test]
fn test_e2e_assigning_an_immutable_global_from_a_function_is_an_error() {
    let err = Compiler::compile(
        "let CONFIG = 1\nfn bad() -> Int {\n    CONFIG = 2\n    return CONFIG\n}\n",
        "x",
    )
    .unwrap_err();
    assert!(err.contains("immutable") || err.contains("Mutability"), "got: {err}");
}

#[test]
fn test_e2e_global_holding_a_heap_object_survives_a_gc_cycle() {
    let src = "\
class Box { n: Int }
let KEPT = Box { n: 42 }
fn churn() -> Int {
    var i = 0
    while i < 500 {
        let junk = Box { n: i }
        i = i + 1
    }
    return 0
}
fn main() -> Int {
    churn()
    return KEPT.n
}
";
    assert_eq!(run_top(src), 42);
}

#[test]
fn test_e2e_top_level_var_is_still_the_implicit_result() {
    let src = "fn sq(n: Int) -> Int { return n * n }\nvar total = 0\nfor i in 1..5 {\n    total += sq(i)\n}\n";
    assert_eq!(run_top(src), 30);
}

#[test]
fn test_e2e_pub_on_a_top_level_let_is_inert_in_a_single_file() {
    let src = "pub let BASE = 40\nfn main() -> Int { return BASE + 2 }\n";
    assert_eq!(run_top(src), 42);
}

#[test]
fn test_e2e_single_file_top_level_code_plus_global_still_one_frame() {
    let src = "\
var total = 0
total = total + 10
total = total + 32
return total
";
    assert_eq!(run_top(src), 42);
}

#[test]
fn test_e2e_enum_unit_variants_construct_and_match() {
    let src = "\
enum Dir { North, South, East, West }
fn step(d: Dir) -> Int {
    match d {
        North => { return 1 }
        South => { return 2 }
        East => { return 3 }
        West => { return 4 }
    }
}
fn main() -> Int { return step(East) + step(North) }
";
    assert_eq!(run_top(src), 4);
}

#[test]
fn test_e2e_enum_tuple_payload_binding() {
    let src = "\
enum Shape { Circle(Int), Rect(Int, Int), Dot }
fn area(s: Shape) -> Int {
    match s {
        Circle(r) => { return r * r * 3 }
        Rect(w, h) => { return w * h }
        Dot => { return 0 }
    }
}
fn main() -> Int { return area(Circle(4)) + area(Rect(3, 5)) + area(Dot) }
";
    assert_eq!(run_top(src), 48 + 15);
}

#[test]
fn test_e2e_enum_as_return_value_and_qualified_variant() {
    let src = "\
enum Slot { Full(Int), Empty }
fn lookup(k: Int) -> Slot {
    if k > 0 { return Slot.Full(k * 10) }
    return Slot.Empty
}
fn main() -> Int {
    match lookup(5) {
        Full(v) => { return v }
        Empty => { return -1 }
    }
}
";
    assert_eq!(run_top(src), 50);
}

#[test]
fn test_e2e_prelude_option_is_available_without_import() {
    let src = "\
fn first_positive(a: Int, b: Int) -> Option<Int> {
    if a > 0 { return Some(a) }
    if b > 0 { return Some(b) }
    return None
}
fn main() -> Int {
    match first_positive(0, 7) {
        Some(v) => { return v }
        None => { return -1 }
    }
}
";
    assert_eq!(run_top(src), 7);
}

#[test]
fn test_e2e_enum_wildcard_arm_covers_the_rest() {
    let src = "\
enum Color { Red, Green, Blue, Other }
fn code(c: Color) -> Int {
    match c {
        Red => { return 1 }
        _ => { return 0 }
    }
}
fn main() -> Int { return code(Blue) + code(Red) }
";
    assert_eq!(run_top(src), 1);
}

#[test]
fn test_enum_non_exhaustive_match_is_an_error() {
    let err = Compiler::compile(
        "enum E { A, B, C }\nfn f(e: E) -> Int {\n    match e {\n        A => { return 1 }\n        B => { return 2 }\n    }\n}\n",
        "x",
    )
    .unwrap_err();
    assert!(err.contains("Non-exhaustive"), "got: {err}");
}

#[test]
fn test_enum_variant_arity_mismatch_is_an_error() {
    let err = Compiler::compile(
        "enum E { Pair(Int, Int) }\nfn f() -> E {\n    return Pair(1)\n}\n",
        "x",
    )
    .unwrap_err();
    assert!(err.to_lowercase().contains("value"), "got: {err}");
}

#[test]
fn test_e2e_try_operator_propagates_and_unwraps() {
    let src = "\
fn half(n: Int) -> Option<Int> {
    if n % 2 == 0 { return Some(n / 2) }
    return None
}
fn quarter(n: Int) -> Option<Int> {
    let h = half(n)?
    return half(h)
}
fn main() -> Int {
    match quarter(20) {
        Some(v) => { return v }
        None => { return -1 }
    }
}
";
    assert_eq!(run_top(src), 5);
}

#[test]
fn test_e2e_try_operator_short_circuits_on_none() {
    let src = "\
fn half(n: Int) -> Option<Int> {
    if n % 2 == 0 { return Some(n / 2) }
    return None
}
fn quarter(n: Int) -> Option<Int> {
    let h = half(n)?
    return half(h)
}
fn main() -> Int {
    match quarter(10) {
        Some(v) => { return v }
        None => { return -1 }
    }
}
";
    assert_eq!(run_top(src), -1);
}

#[test]
fn test_e2e_try_operator_on_result() {
    let src = "\
fn checked(n: Int) -> Result {
    if n > 0 { return Ok(n) }
    return Err(0)
}
fn add_two(n: Int) -> Result {
    let v = checked(n)?
    return Ok(v + 2)
}
fn main() -> Int {
    match add_two(40) {
        Ok(v) => { return v }
        Err(e) => { return -1 }
    }
}
";
    assert_eq!(run_top(src), 42);
}

#[test]
fn test_e2e_unwrap_operator_returns_payload() {
    let src = "\
fn half(n: Int) -> Option<Int> {
    if n % 2 == 0 { return Some(n / 2) }
    return None
}
fn main() -> Int {
    return half(84)!
}
";
    assert_eq!(run_top(src), 42);
}

#[test]
fn test_e2e_unwrap_operator_panics_on_none() {
    let src = "\
fn half(n: Int) -> Option<Int> {
    if n % 2 == 0 { return Some(n / 2) }
    return None
}
fn main() -> Int {
    return half(7)!
}
";
    let err = Compiler::run(src).unwrap_err();
    assert!(err.to_lowercase().contains("empty option") || err.contains("Err"), "got: {err}");
}

#[test]
fn test_try_operator_outside_option_function_is_an_error() {
    let err = Compiler::compile(
        "fn f() -> Option<Int> { return Some(1) }\nfn main() -> Int {\n    let v = f()?\n    return v\n}\n",
        "x",
    )
    .unwrap_err();
    assert!(err.contains("needs a function that returns an optional"), "got: {err}");
}

#[test]
fn test_e2e_option_probe_methods() {
    let src = "\
fn find(n: Int) -> Option<Int> {
    if n > 0 { return Some(n) }
    return None
}
fn main() -> Int {
    var hits = 0
    if find(3).is_some() { hits += 1 }
    if find(0).is_none() { hits += 10 }
    if find(0).is_some() { hits += 100 }
    return hits
}
";
    assert_eq!(run_top(src), 11);
}

#[test]
fn test_e2e_result_unwrap_or() {
    let src = "\
fn parse(n: Int) -> Result {
    if n > 0 { return Ok(n * 2) }
    return Err(0)
}
fn main() -> Int {
    return parse(20).unwrap_or(-1) + parse(0).unwrap_or(7)
}
";
    assert_eq!(run_top(src), 40 + 7);
}

#[test]
fn test_e2e_option_unwrap_panics() {
    let src = "\
fn first(n: Int) -> Option<Int> {
    if n > 0 { return Some(n) }
    return None
}
fn main() -> Int { return first(0).unwrap() }
";
    let err = Compiler::run(src).unwrap_err();
    assert!(err.to_lowercase().contains("unwrap"), "got: {err}");
}

#[test]
fn test_e2e_string_len_method() {
    let src = "\
fn main() -> Int {
    let s = \"héllo\"
    return s.len()
}
";
    assert_eq!(run_top(src), 6);
}

#[test]
fn test_string_unknown_method_is_an_error() {
    let err = Compiler::compile(
        "fn main() -> Int {\n    let s = \"x\"\n    return s.frobnicate()\n}\n",
        "x",
    )
    .unwrap_err();
    assert!(err.contains("no method") && err.contains("String"), "got: {err}");
}

#[test]
fn test_e2e_string_concat_and_content_equality() {
    assert_eq!(eval_main("let s = \"foo\" + \"bar\"\nreturn s.len()"), 6);
    assert_eq!(eval_main("if (\"ab\" + \"cd\") == \"abcd\" { return 1 }\nreturn 0"), 1);
    assert_eq!(eval_main("if \"ab\" == \"abcd\" { return 1 }\nreturn 0"), 0);
    assert_eq!(eval_main("if \"a\".concat(\"b\").concat(\"c\") == \"abc\" { return 1 }\nreturn 0"), 1);
}

#[test]
fn test_e2e_string_slice_find_replace() {
    assert_eq!(eval_main("let s = \"hello world\"\nreturn s.slice(0, 5).len()"), 5);
    assert_eq!(eval_main("if \"hello world\".slice(6, 11) == \"world\" { return 1 }\nreturn 0"), 1);
    assert_eq!(eval_main("if \"a,b,c\".replace(\",\", \"-\") == \"a-b-c\" { return 1 }\nreturn 0"), 1);
    assert_eq!(eval_main("return \"hello\".find(\"ll\").unwrap()"), 2);
    assert_eq!(eval_main("if \"hello\".find(\"zz\").is_none() { return 1 }\nreturn 0"), 1);
}

#[test]
fn test_e2e_string_case_trim_repeat_split() {
    assert_eq!(eval_main("if \"AbC\".to_lower() == \"abc\" { return 1 }\nreturn 0"), 1);
    assert_eq!(eval_main("if \"AbC\".to_upper() == \"ABC\" { return 1 }\nreturn 0"), 1);
    assert_eq!(eval_main("if \"  hi  \".trim() == \"hi\" { return 1 }\nreturn 0"), 1);
    assert_eq!(eval_main("return \"ab\".repeat(3).len()"), 6);
    assert_eq!(eval_main("return \"a,b,c,d\".split(\",\").len()"), 4);
    assert_eq!(eval_main("if \"a,b,c\".split(\",\").get(1) == \"b\" { return 1 }\nreturn 0"), 1);
}

#[test]
fn test_e2e_string_contains_starts_ends() {
    assert_eq!(eval_main("if \"hello\".contains(\"ell\") { return 1 }\nreturn 0"), 1);
    assert_eq!(eval_main("if \"hello\".starts_with(\"he\") { return 1 }\nreturn 0"), 1);
    assert_eq!(eval_main("if \"hello\".ends_with(\"lo\") { return 1 }\nreturn 0"), 1);
    assert_eq!(eval_main("if \"hello\".starts_with(\"xx\") { return 0 }\nreturn 1"), 1);
}

#[test]
fn test_e2e_string_bytes_roundtrips_through_decode() {
    let body = "\
let b = \"Hi\".bytes()
if b.len() == 2 {
    if b.get(0) == 72 {
        if b.decode().unwrap() == \"Hi\" {
            return 1
        }
    }
}
return 0";
    assert_eq!(eval_main(body), 1);
}

#[test]
fn test_string_slice_off_a_char_boundary_is_a_runtime_error() {
    let err = Compiler::run("fn main() -> Int {\n    let s = \"é\"\n    return s.slice(0, 1).len()\n}\nreturn main()\n")
        .unwrap_err();
    assert!(err.to_lowercase().contains("boundary"), "got: {err}");
}

#[test]
fn test_string_method_arg_type_is_checked() {
    let err = Compiler::compile(
        "fn main() -> Int {\n    let s = \"x\"\n    return s.split(3).len()\n}\n",
        "x",
    )
    .unwrap_err();
    assert!(err.contains("String") && err.contains("split"), "got: {err}");
}

#[test]
fn test_e2e_for_inclusive_range() {
    assert_eq!(eval_main("var s = 0\nfor i in 0..=5 {\n    s += i\n}\nreturn s"), 1 + 2 + 3 + 4 + 5);
    assert_eq!(eval_main("var s = 0\nfor i in 0..5 {\n    s += i\n}\nreturn s"), 1 + 2 + 3 + 4);
}

#[test]
fn test_e2e_for_inclusive_range_break_continue() {
    let body = "var s = 0\nfor i in 1..=10 {\n    if i == 4 { continue }\n    if i == 7 { break }\n    s += i\n}\nreturn s";
    assert_eq!(eval_main(body), 1 + 2 + 3 + 5 + 6);
}

#[test]
fn test_for_over_a_non_iterable_is_a_clean_error() {
    let err = Compiler::compile(
        "fn main() -> Int {\n    let n = 5\n    for x in n {\n        return x\n    }\n    return 0\n}\n",
        "x",
    )
    .unwrap_err();
    assert!(err.contains("iter"), "got: {err}");
}

#[test]
fn test_e2e_string_to_string_is_identity() {
    assert_eq!(run_top("fn main() -> Int {\n    let s = \"abc\".to_string()\n    return s.len()\n}\nreturn main()"), 3);
}

#[test]
fn test_e2e_to_string_is_universal_via_the_display_convention() {
    assert_eq!(eval_main("return (42).to_string().len()"), 2);
    assert_eq!(eval_main("return true.to_string().len()"), 4);
    assert_eq!(eval_main("let n = Some(5)\nreturn n.to_string().len()"), 1);
    assert_eq!(eval_main("if [1, 2, 3].to_string() == \"[1, 2, 3]\" { return 1 }\nreturn 0"), 1);
}

#[test]
fn test_e2e_interpolation_basic() {
    let body = "\
let name = \"Ada\"
let n = 3
let s = \"hi ${name}, ${n} items\"
if s == \"hi Ada, 3 items\" { return 1 }
return 0";
    assert_eq!(eval_main(body), 1);
}

#[test]
fn test_e2e_interpolation_edges_and_expressions() {
    assert_eq!(eval_main("let x = 2\nif \"${x}!\" == \"2!\" { return 1 }\nreturn 0"), 1);
    assert_eq!(eval_main("let x = 2\nif \"v${x + 3}\" == \"v5\" { return 1 }\nreturn 0"), 1);
    assert_eq!(eval_main("let a = 1\nlet b = 2\nif \"${a}${b}\" == \"12\" { return 1 }\nreturn 0"), 1);
    assert_eq!(eval_main("let x = [1, 2]\nif \"${x}\" == \"[1, 2]\" { return 1 }\nreturn 0"), 1);
}

#[test]
fn test_e2e_interpolation_escape_opts_out() {
    assert_eq!(eval_main("let x = 9\nreturn \"\\${x}\".len()"), 4);
}

#[test]
fn test_e2e_interpolation_nested_call_and_method() {
    let src = "\
fn double(n: Int) -> Int { return n * 2 }
fn main() -> Int {
    let xs = [10, 20, 30]
    let s = \"len=${xs.len()} first*2=${double(xs.get(0))}\"
    if s == \"len=3 first*2=20\" { return 1 }
    return 0
}
return main()";
    assert_eq!(run_top(src), 1);
}

#[test]
fn test_interpolation_empty_hole_is_a_lex_error() {
    let err = Compiler::compile("fn main() -> Int {\n    let s = \"a ${} b\"\n    return 0\n}\n", "x")
        .unwrap_err();
    assert!(!err.is_empty(), "expected a lex/parse error for `${{}}`");
}

#[test]
fn test_interpolation_in_a_pattern_is_rejected() {
    let err = Compiler::compile(
        "fn main() -> Int {\n    let x = 1\n    match \"a\" {\n        \"${x}\" => { return 1 }\n        _ => { return 0 }\n    }\n}\n",
        "x",
    )
    .unwrap_err();
    assert!(err.contains("interpolation"), "got: {err}");
}

#[test]
fn test_e2e_math_dispatch_natives() {
    assert_eq!(eval_main("return __num_to_int(__math_f1(\"sqrt\", 81.0))"), 9);
    assert_eq!(eval_main("return __num_to_int(__math_f2(\"pow\", 2.0, 10.0))"), 1024);
    assert_eq!(eval_main("return __num_to_int(__math_f1(\"floor\", 3.9))"), 3);
    assert_eq!(eval_main("if __math_pred(\"is_finite\", 1.0) { return 1 }\nreturn 0"), 1);
    assert_eq!(eval_main("return __math_iwrap(\"wrapping_add\", 9223372036854775807, 1)"), i64::MIN);
    assert_eq!(eval_main("return __math_iwrap(\"saturating_mul\", 9223372036854775807, 9)"), i64::MAX);
    assert_eq!(eval_main("return __num_to_int(__num_to_float(7))"), 7);
}

#[test]
fn test_math_to_int_on_non_finite_is_a_runtime_error() {
    let err = Compiler::run("fn main() -> Int {\n    return __num_to_int(1.0 / 0.0)\n}\nreturn main()\n")
        .unwrap_err();
    assert!(err.to_lowercase().contains("finite"), "got: {err}");
}

#[test]
fn test_math_dispatch_arg_types_are_checked() {
    let err = Compiler::compile(
        "fn main() -> Int {\n    return __num_to_int(__math_f1(\"sqrt\", 4))\n}\n",
        "x",
    )
    .unwrap_err();
    assert!(err.contains("Float") || err.contains("__math_f1"), "got: {err}");
}

#[test]
fn test_e2e_for_in_over_a_typed_list_resolves_struct_fields() {
    let src = "\
struct Point { x: Int  y: Int }
fn main() -> Int {
    let pts = [Point { x: 1, y: 10 }, Point { x: 2, y: 20 }, Point { x: 3, y: 30 }]
    var s = 0
    for p in pts {
        s = s + p.y
    }
    return s
}
return main()";
    assert_eq!(run_top(src), 60);
}

#[test]
fn test_e2e_typed_lambda_params_resolve_struct_fields() {
    let src = "\
struct Job { name: Int  weight: Int }
fn pick(cmp: (Job, Job) -> Bool, a: Job, b: Job) -> Job {
    if cmp(a, b) { return a }
    return b
}
fn main() -> Int {
    let heavier = pick(|a: Job, b: Job| a.weight > b.weight,
        Job { name: 1, weight: 3 }, Job { name: 2, weight: 8 })
    return heavier.weight
}
return main()";
    assert_eq!(run_top(src), 8);
}

#[test]
fn test_e2e_for_in_string_list_still_typed_as_string() {
    let src = "\
fn main() -> Int {
    let words = [\"a\", \"bb\", \"ccc\"]
    var n = 0
    for w in words {
        n = n + w.to_upper().len()
    }
    return n
}
return main()";
    assert_eq!(run_top(src), 6);
}

#[test]
fn test_e2e_print_struct_and_enum_compile_and_run() {
    let src = "\
struct P {
  x: Int
  y: Int
}
enum E { A(Int), B }
fn main() -> Int {
  println(P { x: 1, y: 2 })
  println(A(9))
  println(B)
  println(Some(3))
  println(None)
  return 0
}
return main()";
    assert_eq!(run_top(src), 0);
}

#[test]
fn any_boundary_check_faults_on_a_real_type_mismatch() {
    let src = r#"
fn identity(val: __Any) -> String {
    return val
}
fn main() -> Int {
    let v: Float = 10.75
    let w: String = identity(v)
    return 0
}
return main()
"#;
    let err = Compiler::run(src).unwrap_err();
    assert!(err.contains("expected"), "got: {err}");
    assert!(err.contains("String"), "should name the expected type, got: {err}");
}

#[test]
fn any_boundary_check_passes_a_correctly_typed_value_through() {
    let src = r#"
fn identity(val: __Any) -> Int {
    return val
}
fn main() -> Int {
    let v: Int = 41
    let w: Int = identity(v)
    return w + 1
}
return main()
"#;
    assert_eq!(run_top(src), 42);
}

#[test]
fn any_boundary_check_applies_to_let_init_not_just_return() {
    let src = r#"
fn any_string() -> __Any {
    return "not an int"
}
fn main() -> Int {
    let w: Int = any_string()
    return w
}
return main()
"#;
    let err = Compiler::run(src).unwrap_err();
    assert!(err.contains("expected"), "got: {err}");
}

#[test]
fn any_boundary_check_accepts_null_for_a_nullable_target() {
    let src = r#"
fn maybe_null() -> __Any {
    return null
}
fn main() -> Int {
    let w: Int? = maybe_null()
    if w == null { return 7 }
    return -1
}
return main()
"#;
    assert_eq!(run_top(src), 7);
}

const STRUCT_P: &str = "\
struct P {
    var x: Int
    var y: Int

    fn origin() -> P { return P { x: 0, y: 0 } }
    fn from_x(x: Int) -> P { return P { x: x, y: 0 } }
    fn sum(self) -> Int { return self.x + self.y }
    fn bump(var self) { self.x = self.x + 1 }
    fn moved(self, dx: Int) -> P {
        var c = self
        c.x = c.x + dx
        return c
    }
}
fn mutate(var p: P) { p.x = 99 }
";

fn run_struct(body: &str) -> i64 {
    run_top(&format!("{STRUCT_P}fn main() -> Int {{\n{body}\n}}\nreturn main()"))
}

#[test]
fn test_e2e_struct_static_constructor_and_instance_method() {
    assert_eq!(run_struct("var p = P.from_x(4)\n p.bump()\n p.bump()\n return p.sum()"), 6);
    assert_eq!(run_struct("return P.origin().sum()"), 0);
}

#[test]
fn test_e2e_struct_let_binding_copies() {
    assert_eq!(run_struct("let a = P.from_x(1)\n var b = a\n b.x = 5\n return a.x * 10 + b.x"), 15);
}

#[test]
fn test_e2e_struct_var_argument_is_written_in_place() {
    assert_eq!(run_struct("var a = P.from_x(1)\n mutate(a)\n return a.x"), 99);
}

#[test]
fn test_e2e_struct_self_by_value_copies_receiver() {
    assert_eq!(run_struct("let a = P.from_x(1)\n let c = a.moved(10)\n return a.x * 100 + c.x"), 111);
}

#[test]
fn test_e2e_struct_mut_self_mutates_in_place() {
    assert_eq!(run_struct("var a = P.from_x(1)\n a.bump()\n return a.x"), 2);
}

#[test]
fn test_e2e_struct_nested_value_is_deep_copied() {
    let source = "\
struct Inner { var v: Int }
struct Outer { var i: Inner }
fn main() -> Int {
    let a = Outer { i: Inner { v: 1 } }
    var b = a
    b.i.v = 7
    return a.i.v * 10 + b.i.v
}
return main()";
    assert_eq!(run_top(source), 17);
}

#[test]
fn test_e2e_struct_pushed_into_list_is_copied() {
    let source = format!("{STRUCT_P}fn main() -> Int {{
    var p = P.from_x(1)
    let xs = [p]
    p.x = 50
    let q = xs.get(0)
    return q.x
}}
return main()");
    assert_eq!(run_top(&source), 1);
}

#[test]
fn test_e2e_class_still_aliases() {
    let source = "\
class C { var x: Int }
fn main() -> Int {
    let a = C { x: 1 }
    var b = a
    b.x = 5
    return a.x
}
return main()";
    assert_eq!(run_top(source), 5);
}

#[test]
fn test_struct_unknown_method_is_a_checker_error() {
    let source = format!("{STRUCT_P}fn main() -> Int {{ let p = P.origin()\n return p.nope() }}\nreturn main()");
    let mut parser = Parser::new(Lexer::new(&source).tokenize().unwrap());
    let program = parser.parse().unwrap();
    let err = TypeChecker::new().check_program(&program).unwrap_err();
    assert!(format!("{err:?}").contains("no method `.nope()` on `P`"), "{err:?}");
}

#[test]
fn test_the_checker_stage_output_matches_its_golden_dump() {
    let source = "struct P {\n    a: Int\n    b: Int\n}\nfn f(p: P) -> Int {\n    return p.b + 1\n}\n";
    let checked = compiler::Compiler::check(source, "golden").unwrap();
    assert_eq!(checked.modules.len(), 1);
    let expected = "\
6:12+1  P
6:12+3  Int
6:12+7  Int
6:18+1  Int
";
    assert_eq!(checked.dump_types(), expected);
}
