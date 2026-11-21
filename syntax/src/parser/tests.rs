use mote_base::{DiagnosticSink, Interner, SourceMap};

use super::*;
use crate::ast::dump::Dump;
use crate::ast::{ItemKind, Member};
use crate::lexer::lex;

struct Run {
    ast: Ast,
    names: Interner,
    sink: DiagnosticSink,
    text: String,
}

fn run(text: &str) -> Run {
    let mut map = SourceMap::new();
    let id = map.add_file("test", text);
    let mut names = Interner::new();
    let mut sink = DiagnosticSink::new();
    let lexed = lex(map.file(id), &mut names, &mut sink);
    let ast = parse(map.file(id), &lexed, &mut names, &mut sink);
    Run { ast, names, sink, text: text.to_string() }
}

impl Run {
    fn dump(&self) -> String {
        Dump { ast: &self.ast, names: &self.names }.root()
    }

    fn errors(&self) -> Vec<String> {
        self.sink.iter().filter(|d| d.severity == mote_base::Severity::Error).map(|d| d.message.clone()).collect()
    }

    fn snippet(&self, span: Option<mote_base::Span>) -> &str {
        let span = span.expect("no span");
        &self.text[span.start as usize..span.end as usize]
    }
}

fn tree(text: &str) -> String {
    let r = run(text);
    assert_eq!(r.errors(), Vec::<String>::new(), "{text}");
    r.dump()
}

fn errors(text: &str) -> Vec<String> {
    run(text).errors()
}

#[test]
fn an_empty_file_has_no_items() {
    assert_eq!(tree(""), "");
    assert_eq!(tree("\n\n"), "");
}

#[test]
fn arithmetic_follows_the_precedence_table() {
    assert_eq!(tree("1 + 2 * 3"), "(+ 1 (* 2 3))");
    assert_eq!(tree("1 * 2 + 3"), "(+ (* 1 2) 3)");
    assert_eq!(tree("a - b - c"), "(- (- a b) c)");
    assert_eq!(tree("a << 1 + 2"), "(<< a (+ 1 2))");
    assert_eq!(tree("a | b ^ c & d"), "(| a (^ b (& c d)))");
    assert_eq!(tree("a && b || c"), "(|| (&& a b) c)");
    assert_eq!(tree("a == b && c < d"), "(&& (== a b) (< c d))");
}

#[test]
fn coalesce_is_the_loosest_and_right_associative() {
    assert_eq!(tree("a ?? b ?? c"), "(?? a (?? b c))");
    assert_eq!(tree("a || b ?? c"), "(?? (|| a b) c)");
}

#[test]
fn prefix_binds_tighter_than_binary_and_looser_than_postfix() {
    assert_eq!(tree("-a * b"), "(* (neg a) b)");
    assert_eq!(tree("-a.b"), "(neg (. a b))");
    assert_eq!(tree("!a && b"), "(&& (not a) b)");
    assert_eq!(tree("~a"), "(bitnot a)");
    assert_eq!(tree("- - a"), "(neg (neg a))");
}

#[test]
fn casts_sit_between_ranges_and_prefix_operators() {
    assert_eq!(tree("-a as Int"), "(as (neg a) Int)");
    assert_eq!(tree("a as Int + 1"), "(+ (as a Int) 1)");
    assert_eq!(tree("0..n as Int"), "(.. 0 (as n Int))");
    assert_eq!(tree("a as Int as Float"), "(as (as a Int) Float)");
    assert_eq!(tree("a as Int?"), "(as a Int?)");
}

#[test]
fn type_tests_and_ranges() {
    assert_eq!(tree("x is Int && y"), "(&& (is x Int) y)");
    assert_eq!(tree("1..n"), "(.. 1 n)");
    assert_eq!(tree("a + 1..=b * 2"), "(..= (+ a 1) (* b 2))");
    assert_eq!(tree("0..n + 1"), "(.. 0 (+ n 1))");
    assert_eq!(tree("a * 2..b"), "(.. (* a 2) b)");
    assert_eq!(tree("a..b << 1"), "(<< (.. a b) 1)");
}

#[test]
fn comparisons_and_ranges_do_not_chain() {
    assert_eq!(errors("a < b < c"), ["comparisons do not chain"]);
    assert_eq!(errors("a == b != c"), ["comparisons do not chain"]);
    assert_eq!(errors("1..2..3"), ["ranges do not chain"]);
    assert_eq!(tree("a < b == c"), "(== (< a b) c)");
}

#[test]
fn postfix_operators_chain_left_to_right() {
    assert_eq!(tree("a.b.c(1)[2]"), "(index (call (. (. a b) c) 1) 2)");
    assert_eq!(tree("a?.b"), "(?. a b)");
    assert_eq!(tree("a?.b()"), "(call (?. a b))");
    assert_eq!(tree("a!"), "(unwrap a)");
    assert_eq!(tree("a?"), "(propagate a)");
    assert_eq!(tree("a.0.1"), "(. (. a 0) 1)");
    assert_eq!(tree("T.from(x)"), "(call (. T from) x)");
}

#[test]
fn calls_take_named_arguments() {
    assert_eq!(tree("f()"), "(call f)");
    assert_eq!(tree("f(1, w=3)"), "(call f 1 w=3)");
    assert_eq!(tree("f(a == b)"), "(call f (== a b))");
}

#[test]
fn indexing_and_slicing() {
    assert_eq!(tree("a[i]"), "(index a i)");
    assert_eq!(tree("a[1..3]"), "(slice a 1 3 _)");
    assert_eq!(tree("a[..3]"), "(slice a _ 3 _)");
    assert_eq!(tree("a[1..]"), "(slice a 1 _ _)");
    assert_eq!(tree("a[..]"), "(slice a _ _ _)");
    assert_eq!(tree("a[1..=3]"), "(slice= a 1 3 _)");
    assert_eq!(tree("a[i + 1 .. n .. 2]"), "(slice a (+ i 1) n 2)");
    assert_eq!(tree("a[f(1..2)]"), "(index a (call f (.. 1 2)))");
}

#[test]
fn literals_and_groupings() {
    assert_eq!(tree("(1)"), "(paren 1)");
    assert_eq!(tree("()"), "(tuple)");
    assert_eq!(tree("(1,)"), "(tuple 1)");
    assert_eq!(tree("(1, 2)"), "(tuple 1 2)");
    assert_eq!(tree("[]"), "(list)");
    assert_eq!(tree("[1, 2,]"), "(list 1 2)");
    assert_eq!(tree("{}"), "(map)");
    assert_eq!(tree("{\"a\": 1, \"b\": 2}"), "(map (\"a\" 1) (\"b\" 2))");
    assert_eq!(tree("[0; 16]"), "(repeat 0 16)");
    assert_eq!(tree("[[0; 2]; n]"), "(repeat (repeat 0 2) n)");
    assert_eq!(tree("true"), "true");
    assert_eq!(tree("null"), "null");
}

#[test]
fn interpolated_strings_keep_their_pieces() {
    assert_eq!(tree("\"a ${x} b ${y + 1}!\""), "(interp \"a \" x \" b \" (+ y 1) \"!\")");
    assert_eq!(tree("\"${f(\"${x}\")}\""), "(interp \"\" (call f (interp \"\" x \"\")) \"\")");
}

#[test]
fn struct_literals_use_brace_lookahead() {
    assert_eq!(tree("Point { x: 1, y: 2 }"), "(init Point (x 1) (y 2))");
    assert_eq!(tree("Point { x, y }"), "(init Point x y)");
    assert_eq!(tree("Point { ..p, x: 1 }"), "(init Point (.. p) (x 1))");
    assert_eq!(tree("Point {}"), "(init Point)");
    assert_eq!(tree("geo.Point { x: 1 }"), "(init geo.Point (x 1))");
    assert_eq!(tree("let p = Point { x }"), "(let p (init Point x))");
    assert_eq!(tree("Pair<Int, String> { a: 1, b: \"x\" }"), "(init (apply Pair Int String) (a 1) (b \"x\"))");
    assert_eq!(tree("std.Box<Int> { value }"), "(init (apply std.Box Int) value)");
    assert_eq!(tree("if a < b { c }"), "(if (< a b) (block c))");
}

#[test]
fn colon_colon_and_super_have_no_use_outside_import_paths() {
    assert!(!errors("let x = a::b").is_empty());
    assert!(!errors("let x = super").is_empty());
    assert!(!errors("fn f() { super.g() }").is_empty());
    assert_eq!(tree("import ..super.a.b"), "(import ..super.a.b)");
}

#[test]
fn a_paren_on_a_new_line_is_not_a_call() {
    assert_eq!(tree("match p {\n (0, y) => println(y)\n (x, y) => println(x)\n}"), "(match p ((0, y) => (call println y)) ((x, y) => (call println x)))");
    assert_eq!(tree("f(a,\n b)"), "(call f a b)");
    assert_eq!(tree("if x {\n f()\n (a, b)\n}"), "(if x (block (call f) (tuple a b)))");
}

#[test]
fn a_brace_after_a_condition_is_a_block() {
    assert_eq!(tree("if a == b { c }"), "(if (== a b) (block c))");
    assert_eq!(tree("if a == b {}"), "(if (== a b) (block))");
    assert_eq!(tree("while i < n { i += 1 }"), "(while (< i n) (block (+= i 1)))");
    assert_eq!(tree("for x in xs { f(x) }"), "(for x xs (block (call f x)))");
    assert_eq!(tree("if f(Point { x }) { y }"), "(if (call f (init Point x)) (block y))");
    assert_eq!(tree("if a == Point { x: 1 } { y }"), "(if (== a (init Point (x 1))) (block y))");
}

#[test]
fn a_match_subject_stops_before_its_arms() {
    assert_eq!(tree("match x { a: Int => 1 }"), "(match x (a: Int => 1))");
    assert_eq!(tree("match p { Point { x, y } => x }"), "(match p (Point { x, y } => x))");
}

#[test]
fn type_arguments_need_a_call_or_member_after_them() {
    assert_eq!(tree("List<Int>()"), "(call (apply List Int))");
    assert_eq!(tree("Map<String, Int>()"), "(call (apply Map String Int))");
    assert_eq!(tree("Stack<Int>.new()"), "(call (. (apply Stack Int) new))");
    assert_eq!(tree("a < b"), "(< a b)");
    assert_eq!(tree("a < b && c > d"), "(&& (< a b) (> c d))");
    assert_eq!(tree("Map<String, List<Int>>()"), "(call (apply Map String List<Int>))");
    assert_eq!(tree("std.Box<Int>(3)"), "(call (apply std.Box Int) 3)");
}

#[test]
fn lambdas_take_params_a_return_type_and_a_body() {
    assert_eq!(tree("|x| x + 1"), "(lambda (x) (+ x 1))");
    assert_eq!(tree("|| 1"), "(lambda () 1)");
    assert_eq!(tree("|x: Int, var y| -> Int x"), "(lambda (x:Int var y) -> Int x)");
    assert_eq!(tree("f(|x| x, 2)"), "(call f (lambda (x) x) 2)");
    assert_eq!(tree("|x| { return x }"), "(lambda (x) (block (return x)))");
    assert_eq!(tree("|x: (Int | String)| x"), "(lambda (x:(Int | String)) x)");
    assert_eq!(tree("|| n += 1"), "(lambda () (block (+= n 1)))");
    assert_eq!(tree("f(|x| total = x, 2)"), "(call f (lambda (x) (block (= total x))) 2)");
}

#[test]
fn if_and_match_work_as_values() {
    assert_eq!(tree("let x = if c { 1 } else { 2 }"), "(let x (if c 1 2))");
    assert_eq!(tree("let x = if c { 1 } else if d { 2 } else { 3 }"), "(let x (if c 1 (if d 2 3)))");
    assert_eq!(tree("let x = if let Some(v) = o { v } else { 0 }"), "(let x (if (let Some(v) o) v 0))");
    assert_eq!(
        tree("let x = match n { 0 => \"a\", 1 | 2 => \"b\", k if k > 5 => \"c\", _ => \"d\" }"),
        "(let x (match n (0 => \"a\") (1 | 2 => \"b\") (k if (> k 5) => \"c\") (_ => \"d\")))"
    );
}

#[test]
fn a_value_if_needs_an_else_and_one_expression() {
    assert_eq!(errors("let x = if c { 1 }"), ["a value `if` needs an `else`"]);
    assert_eq!(errors("let x = if c { f(); 1 } else { 2 }"), ["a branch holds one expression"]);
}

#[test]
fn a_value_arm_is_one_expression_not_a_block() {
    assert_eq!(errors("let x = match n { 1 => { return 1 } }"), ["an arm holds one expression"]);
    assert_eq!(errors("let x = match n { 1 => { 1 }, _ => 2 }"), ["an arm holds one expression"]);
    assert_eq!(errors("let x = match n { 1 => { let y: Int = 1\n y } }"), ["an arm holds one expression"]);
    assert_eq!(errors("let x = match n { 1 => {\"a\": 1}, 2 => {} }"), Vec::<String>::new());
}

#[test]
fn bindings_and_assignments() {
    assert_eq!(tree("let x = 1"), "(let x 1)");
    assert_eq!(tree("var x: Int = 1"), "(var x:Int 1)");
    assert_eq!(tree("let (a, b) = t"), "(let (a, b) t)");
    assert_eq!(tree("let Point { x, y: yy } = p"), "(let Point { x, y: yy } p)");
    assert_eq!(tree("x = 1"), "(= x 1)");
    assert_eq!(tree("x.y[0] += 2"), "(+= (index (. x y) 0) 2)");
    assert_eq!(tree("x ??= 5"), "(??= x 5)");
    assert_eq!(tree("x <<= 1"), "(<<= x 1)");
    assert_eq!(tree("(a, b) = (b, a)"), "(= (tuple a b) (tuple b a))");
}

#[test]
fn statements_in_a_block_need_no_separator() {
    assert_eq!(tree("fn f() { let a = 1\n let b = 2 a + b }"), "(fn f () (block (let a 1) (let b 2) (+ a b)))");
    assert_eq!(tree("fn f() { a; b }"), "(fn f () (block a b))");
}

#[test]
fn control_flow_statements() {
    assert_eq!(tree("if a { b } else if c { d } else { e }"), "(if a (block b) (if c (block d) (block e)))");
    assert_eq!(tree("if let Some(x) = o { x }"), "(if (let Some(x) o) (block x))");
    assert_eq!(tree("while let Some(x) = it.next() { x }"), "(while (let Some(x) (call (. it next))) (block x))");
    assert_eq!(tree("for (i, x) in xs { i }"), "(for (i, x) xs (block i))");
    assert_eq!(tree("for i in 0..10 { i }"), "(for i (.. 0 10) (block i))");
    assert_eq!(tree("scope { spawn f() }"), "(scope (block (spawn (call f))))");
    assert_eq!(tree("with f = open(p) { f }"), "(with f (call open p) (block f))");
    assert_eq!(tree("fn f() { return }"), "(fn f () (block (return)))");
    assert_eq!(tree("fn f() { return 1 }"), "(fn f () (block (return 1)))");
    assert_eq!(tree("fn f() { return\n 0 }"), "(fn f () (block (return 0)))");
    assert_eq!(tree("fn f() { while true { break } }"), "(fn f () (block (while true (block (break)))))");
    assert_eq!(tree("fn f() { continue }"), "(fn f () (block (continue)))");
    assert_eq!(tree("fn f() { yield 1 }"), "(fn f () (block (yield 1)))");
}

#[test]
fn an_else_may_start_the_next_line_at_the_top_level() {
    assert_eq!(tree("if a { b }\nelse { c }"), "(if a (block b) (block c))");
}

#[test]
fn a_match_statement_takes_blocks_and_statements() {
    assert_eq!(
        tree("match x { 1 => { a } 2 | 3 => b, k if k > 1 => return k, _ => {} }"),
        "(match x (1 => (block a)) (2 | 3 => b) (k if (> k 1) => (return k)) (_ => (block)))"
    );
}

#[test]
fn spawn_takes_a_block_or_an_expression() {
    assert_eq!(tree("spawn { f() }"), "(spawn (block (call f)))");
    assert_eq!(tree("spawn f(1)"), "(spawn (call f 1))");
    assert_eq!(tree("let h = spawn f().g()"), "(let h (spawn (call (. (call f) g))))");
}

#[test]
fn patterns() {
    assert_eq!(tree("match x { _ => 1 }"), "(match x (_ => 1))");
    assert_eq!(tree("match x { -1 => 1 }"), "(match x (-1 => 1))");
    assert_eq!(tree("match x { 1..=5 => 1 }"), "(match x (1..=5 => 1))");
    assert_eq!(tree("match x { n @ 1..5 => n }"), "(match x (n @ 1..5 => n))");
    assert_eq!(tree("match x { \"a\" | \"b\" => 1 }"), "(match x (\"a\" | \"b\" => 1))");
    assert_eq!(tree("match x { null => 1, true => 2 }"), "(match x (null => 1) (true => 2))");
    assert_eq!(tree("match x { (a, _) => a }"), "(match x ((a, _) => a))");
    assert_eq!(tree("match x { Shape.Circle(r) => r }"), "(match x (Shape.Circle(r) => r))");
    assert_eq!(tree("match x { Some(Some(v)) => v }"), "(match x (Some(Some(v)) => v))");
    assert_eq!(tree("match x { Color.Red => 1 }"), "(match x (Color.Red => 1))");
    assert_eq!(tree("match x { Point { x: 0, .. } => 1 }"), "(match x (Point { x: 0, .. } => 1))");
    assert_eq!(tree("match x { _: Int => 1 }"), "(match x (_: Int => 1))");
    assert_eq!(tree("match x { (a | b) => 1 }"), "(match x (a | b => 1))");
}

#[test]
fn types() {
    assert_eq!(tree("let a: Int? = x"), "(let a:Int? x)");
    assert_eq!(tree("let a: Int?? = x"), "(let a:Int?? x)");
    assert_eq!(tree("let a: List<Int> = x"), "(let a:List<Int> x)");
    assert_eq!(tree("let a: List<List<Int>> = x"), "(let a:List<List<Int>> x)");
    assert_eq!(tree("let a: List<List<List<Int>>> = x"), "(let a:List<List<List<Int>>> x)");
    assert_eq!(tree("let a: Map<String, Int> = x"), "(let a:Map<String, Int> x)");
    assert_eq!(tree("let a: (Int, String) = x"), "(let a:(Int, String) x)");
    assert_eq!(tree("let a: (Int,) = x"), "(let a:(Int,) x)");
    assert_eq!(tree("let a: () = x"), "(let a:() x)");
    assert_eq!(tree("let a: (Int) = x"), "(let a:(Int) x)");
    assert_eq!(tree("let a: (Int, var String) -> Bool = x"), "(let a:(Int, var String) -> Bool x)");
    assert_eq!(tree("let a: Send (Int) -> Int = x"), "(let a:Send (Int) -> Int x)");
    assert_eq!(tree("let a: () -> Int? = x"), "(let a:() -> Int? x)");
    assert_eq!(tree("let a: [Int] = x"), "(let a:[Int] x)");
    assert_eq!(tree("let a: [UInt8; 16] = x"), "(let a:[UInt8; 16] x)");
    assert_eq!(tree("let a: Int | String = x"), "(let a:Int | String x)");
    assert_eq!(tree("let a: std.json.Value = x"), "(let a:std.json.Value x)");
    assert_eq!(tree("let a: Self = x"), "(let a:Self x)");
}

#[test]
fn functions() {
    assert_eq!(tree("fn f() {}"), "(fn f () (block))");
    assert_eq!(tree("pub fn f(a: Int, var b, c = 1) -> Int { a }"), "pub (fn f (a:Int var b c=1) -> Int (block a))");
    assert_eq!(tree("fn f<T, U: Show + Eq>(x: T) {}"), "(fn f<T, U: Show + Eq> (x:T) (block))");
    assert_eq!(tree("fn f(...xs: Int) {}"), "(fn f (...xs:Int) (block))");
    assert_eq!(tree("fn from(v: Int) {}"), "(fn from (v:Int) (block))");
    assert_eq!(tree("native fn clock() -> Int"), "(native clock () -> Int)");
    assert_eq!(tree("native fn alloc<T, U: Eq>(n: Int) -> [T]"), "(native alloc<T, U: Eq> (n:Int) -> [T])");
    assert_eq!(tree("fn f() { fn g() {} }"), "(fn f () (block (fn g () (block))))");
    assert_eq!(tree("fn f()\n{\n}"), "(fn f () (block))");
}

#[test]
fn records() {
    assert_eq!(
        tree("struct P { x: Int\n y: Int = 0\n pub var z: Int fn m(self) {} }"),
        "(struct P (field x:Int) (field y:Int = 0) (field pub var z:Int) (fn m (self) (block)))"
    );
    assert_eq!(tree("class C<T>: (A, B) {}"), "(class C<T>: (A, B))");
    assert_eq!(tree("pub struct P {}"), "pub (struct P)");
}

#[test]
fn enums() {
    assert_eq!(
        tree("enum E { A, B(Int, String), C { x: Int, y: Int }, D = 4 fn f(self) {} }"),
        "(enum E A B(Int, String) C{(field x:Int) (field y:Int)} D=4 (fn f (self) (block)))"
    );
    assert_eq!(tree("enum E: (Show) { A\n B }"), "(enum E: (Show) A B)");
}

#[test]
fn traits_aliases_and_imports() {
    assert_eq!(
        tree("trait T<U> { fn a(self) -> Int\n fn b(self) { 1 } }"),
        "(trait T<U> (fn a (self) -> Int) (fn b (self) (block 1)))"
    );
    assert_eq!(tree("type Id<T> = T"), "(type Id<T> = T)");
    assert_eq!(tree("import std.json"), "(import std.json)");
    assert_eq!(tree("import std.json as j"), "(import std.json as j)");
    assert_eq!(tree("pub import ..src.lib as lib"), "pub (import ..src.lib as lib)");
    assert_eq!(tree("import .a.b"), "(import .a.b)");
    assert_eq!(tree("import super.x"), "(import super.x)");
    assert_eq!(tree("import { a, b as c } from std.x"), "(import {a, b as c} from std.x)");
    assert_eq!(tree("import { * } from std.x"), "(import {*} from std.x)");
}

#[test]
fn tests_and_attributes() {
    assert_eq!(tree("test \"adds\" { assert(1) }"), "(test \"adds\" (block (call assert 1)))");
    assert_eq!(tree("@stable fn f() {}"), "@stable (fn f () (block))");
    assert_eq!(
        tree("@flag(short = s, j, \"x\", 3, 1.5, true) struct A {}"),
        "@flag(short=s, j, \"x\", 3, 1.5, true) (struct A)"
    );
}

#[test]
fn docs_attach_to_the_item_below() {
    let r = run("/// one\n/// two\n@a\npub fn f() {}\n\n/// gap\n\nfn g() {}");
    let docs: Vec<_> = r.ast.root.iter().map(|&id| r.ast[id].doc).collect();
    assert_eq!(r.snippet(docs[0]), "/// one\n/// two");
    assert_eq!(docs[1], None);
    assert_eq!(r.errors(), Vec::<String>::new());
    let warnings: Vec<_> = r.sink.iter().map(|d| (d.severity, d.message.as_str())).collect();
    assert_eq!(warnings, [(mote_base::Severity::Warning, "this doc comment documents nothing")]);
}

#[test]
fn docs_attach_to_fields_variants_and_methods() {
    let r = run("struct S {\n /// f\n x: Int\n /// m\n fn m(self) {}\n}\nenum E {\n /// v\n A\n}");
    assert!(r.errors().is_empty());
    assert!(r.sink.is_empty());
    let ItemKind::Record(record) = &r.ast[r.ast.root[0]].kind else { panic!() };
    let Member::Field(field) = &record.members[0] else { panic!() };
    assert_eq!(r.snippet(field.doc), "/// f");
    let Member::Method(method) = &record.members[1] else { panic!() };
    assert_eq!(r.snippet(method.doc), "/// m");
    let ItemKind::Enum(e) = &r.ast[r.ast.root[1]].kind else { panic!() };
    assert_eq!(r.snippet(e.variants[0].doc), "/// v");
}

#[test]
fn a_doc_before_a_statement_is_a_warning() {
    let r = run("fn f() {\n /// stray\n let a = 1\n}");
    assert!(r.errors().is_empty());
    assert_eq!(r.sink.len(), 1);
}

#[test]
fn a_member_access_line_continues_a_top_level_chain() {
    assert_eq!(tree("xs\n  .map(f)\n  .sum()"), "(call (. (call (. xs map) f) sum))");
}

#[test]
fn spans_cover_the_whole_node() {
    let r = run("let x = 1 + 22");
    let item = &r.ast[r.ast.root[0]];
    assert_eq!((item.span.start, item.span.end), (0, 14));
}

#[test]
fn a_missing_closer_is_one_error() {
    assert_eq!(errors("f(1, 2"), ["expected `)`, found the end of the file"]);
    assert_eq!(errors("let x = (1 + 2"), ["expected `)`, found the end of the file"]);
}

#[test]
fn an_expected_token_is_named_with_what_was_found() {
    assert_eq!(errors("let = 1"), ["expected a pattern, found `=`"]);
    assert_eq!(errors("fn f( {}")[0], "expected a name, found `{`");
    assert_eq!(errors("struct { }"), ["expected a name, found `{`"]);
    assert_eq!(errors("let x = )"), ["expected an expression, found `)`"]);
}

#[test]
fn one_bad_element_does_not_swallow_the_call() {
    let r = run("f(1, +, 3)\nlet y = 2");
    assert_eq!(r.errors(), ["expected an expression, found `+`"]);
    assert_eq!(r.dump(), "(call f 1 (error) 3)\n(let y 2)");
}

#[test]
fn a_bad_statement_recovers_at_the_next_statement() {
    let r = run("fn f() {\n let a = \n let b = 2\n return b\n}");
    assert_eq!(r.errors(), ["expected an expression, found `let`"]);
    assert_eq!(r.dump(), "(fn f () (block (let a (error)) (let b 2) (return b)))");
}

#[test]
fn a_bad_item_recovers_at_the_next_item() {
    let r = run("struct\nfn g() {}\n)))\nfn h() {}");
    assert_eq!(r.dump(), "(struct _)\n(fn g () (block))\n(error)\n(fn h () (block))");
}

#[test]
fn errors_at_one_position_are_reported_once() {
    assert_eq!(errors("let x = * *").len(), 1);
}

#[test]
fn lexer_errors_are_not_reported_again() {
    assert_eq!(errors("let x = #"), ["unexpected character `#`"]);
}

#[test]
fn every_comma_list_takes_a_trailing_comma() {
    assert_eq!(tree("f(1,)"), "(call f 1)");
    assert_eq!(tree("[1,]"), "(list 1)");
    assert_eq!(tree("fn f(a, b,) {}"), "(fn f (a b) (block))");
    assert_eq!(tree("fn f<A, B,>() {}"), "(fn f<A, B> () (block))");
    assert_eq!(tree("let x: Map<A, B,> = y"), "(let x:Map<A, B> y)");
    assert_eq!(tree("let x: Map<A, List<B>,> = y"), "(let x:Map<A, List<B>> y)");
    assert_eq!(tree("import { a, b, } from m"), "(import {a, b} from m)");
    assert_eq!(tree("enum E { A(Int,) }"), "(enum E A(Int))");
    assert_eq!(tree("|a, b,| a"), "(lambda (a b) a)");
}

#[test]
fn deep_nesting_is_an_error_not_a_crash() {
    let text = format!("{}1{}", "(".repeat(1000), ")".repeat(1000));
    assert_eq!(errors(&text), ["this is nested too deeply"]);
    let text = format!("let x = {}1", "-".repeat(1000));
    assert_eq!(errors(&text), ["this is nested too deeply"]);
}
