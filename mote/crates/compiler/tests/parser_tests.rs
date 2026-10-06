use compiler::ast::*;
use compiler::lexer::Lexer;
use compiler::parser::Parser;

fn parse_str(src: &str) -> Result<Program, (String, compiler::span::Span)> {
    let mut lexer = Lexer::new(src);
    let tokens = lexer.tokenize().expect("Lexing failed");
    let mut parser = Parser::new(tokens);
    parser.parse()
}

#[test]
fn test_parse_top_level_items() {
    let src = r#"
    pub fn add<T: Number>(a: T, b: T) -> T {
        return a + b
    }

    pub struct Point<T> {
        pub let x: T
        var y: T
    }

    pub enum Shape {
        Circle(Float),
        Rectangle { width: Float, height: Float },
        Point = 0,
    }

    pub class Animal: (Greeter) {
        let name: String
        pub fn speak(self) -> String {
            return self.name
        }
    }

    pub trait Greeter {
        fn greet(self) -> String
        fn default_greet(self) -> String {
            return "hello"
        }
    }

    pub type Coordinate = Point<Float>
    "#;

    let prog = parse_str(src).expect("Failed to parse top level items");
    assert_eq!(prog.items.len(), 6);

    match &prog.items[0] {
        Item::Function(f) => {
            assert_eq!(f.name, "add");
            assert!(f.is_pub);
            assert_eq!(f.generic_params.len(), 1);
            assert_eq!(f.generic_params[0].name, "T");
            assert_eq!(f.params.len(), 2);
        }
        _ => panic!("Expected Function"),
    }

    match &prog.items[1] {
        Item::Struct(s) => {
            assert_eq!(s.name, "Point");
            assert!(s.is_pub);
            assert_eq!(s.fields.len(), 2);
            assert_eq!(s.fields[0].name, "x");
            assert_eq!(s.fields[1].name, "y");
            assert!(s.fields[1].is_mutable);
        }
        _ => panic!("Expected Struct"),
    }

    match &prog.items[2] {
        Item::Enum(e) => {
            assert_eq!(e.name, "Shape");
            assert_eq!(e.variants.len(), 3);
            assert_eq!(e.variants[0].name, "Circle");
            assert!(matches!(e.variants[0].kind, EnumVariantKind::Tuple(_)));
            assert_eq!(e.variants[1].name, "Rectangle");
            assert!(matches!(e.variants[1].kind, EnumVariantKind::Struct(_)));
            assert_eq!(e.variants[2].name, "Point");
            assert!(matches!(e.variants[2].kind, EnumVariantKind::Unit { discriminant: Some(_) }));
        }
        _ => panic!("Expected Enum"),
    }

    match &prog.items[3] {
        Item::Class(c) => {
            assert_eq!(c.name, "Animal");
            assert_eq!(c.traits.len(), 1);
            assert_eq!(c.fields.len(), 1);
            assert_eq!(c.methods.len(), 1);
        }
        _ => panic!("Expected Class"),
    }

    match &prog.items[4] {
        Item::Trait(t) => {
            assert_eq!(t.name, "Greeter");
            assert_eq!(t.members.len(), 2);
            assert!(t.members[0].default_body.is_none());
            assert!(t.members[1].default_body.is_some());
        }
        _ => panic!("Expected Trait"),
    }

    match &prog.items[5] {
        Item::TypeAlias(a) => {
            assert_eq!(a.name, "Coordinate");
            assert!(a.is_pub);
        }
        _ => panic!("Expected TypeAlias"),
    }
}

#[test]
fn test_parse_generic_bounds() {
    let src = "fn compute<T: Display + Clone, U: Debug>(x: T, y: U) {}";
    let prog = parse_str(src).expect("Failed to parse generic bounds");
    match &prog.items[0] {
        Item::Function(f) => {
            assert_eq!(f.generic_params.len(), 2);
            assert_eq!(f.generic_params[0].name, "T");
            assert_eq!(f.generic_params[0].bounds.len(), 2);
            assert_eq!(f.generic_params[1].name, "U");
            assert_eq!(f.generic_params[1].bounds.len(), 1);
        }
        _ => panic!("Expected Function"),
    }
}

#[test]
fn test_parse_type_nodes() {
    let src = r#"
    let a: (Int, Float) -> Bool = null
    let b: [String; 10] = null
    let c: [Int] = null
    let d: Map<String, Int>? = null
    "#;
    let prog = parse_str(src).expect("Failed to parse type nodes");
    assert_eq!(prog.items.len(), 4);

    match &prog.items[0] {
        Item::TopLevelStmt(Stmt::Let { ty: Some(TypeNode::Function(params, ret, _, _)), .. }) => {
            assert_eq!(params.len(), 2);
            assert!(matches!(ret.as_ref(), TypeNode::Bool(_)));
        }
        _ => panic!("Expected Function Type"),
    }

    match &prog.items[1] {
        Item::TopLevelStmt(Stmt::Let { ty: Some(TypeNode::Array(elem, size, _)), .. }) => {
            assert!(matches!(elem.as_ref(), TypeNode::String(_)));
            assert!(size.is_some());
        }
        _ => panic!("Expected Fixed Array Type"),
    }

    match &prog.items[2] {
        Item::TopLevelStmt(Stmt::Let { ty: Some(TypeNode::Array(elem, size, _)), .. }) => {
            assert!(matches!(elem.as_ref(), TypeNode::Int(_)));
            assert!(size.is_none());
        }
        _ => panic!("Expected Dynamic Array Type"),
    }

    match &prog.items[3] {
        Item::TopLevelStmt(Stmt::Let { ty: Some(TypeNode::Nullable(inner, _)), .. }) => {
            assert!(matches!(inner.as_ref(), TypeNode::Generic(_, _, _)));
        }
        _ => panic!("Expected Nullable Type"),
    }
}

#[test]
fn test_parse_pattern_matrix() {
    let src = r#"
    match val {
        _ => 0,
        42 => 1,
        "hello" => 2,
        true => 3,
        null => 4,
        x @ (1, 2) => 5,
        (a, b, _) if a > 0 => a + b
    }
    "#;
    let prog = parse_str(src).expect("Failed to parse pattern matrix");
    match &prog.items[0] {
        Item::TopLevelStmt(Stmt::Match { arms, .. }) => {
            assert_eq!(arms.len(), 7);
            assert!(matches!(arms[0].pattern, Pattern::Wildcard(_)));
            assert!(matches!(arms[1].pattern, Pattern::Literal(_, _)));
            assert!(matches!(arms[5].pattern, Pattern::Identifier { subpattern: Some(_), .. }));
            assert!(matches!(arms[6].pattern, Pattern::Tuple(_, _)));
            assert!(arms[6].guard.is_some());
        }
        _ => panic!("Expected Match Stmt"),
    }
}

#[test]
fn test_parse_pratt_precedence() {
    let src = "let res = a + b * c";
    let prog = parse_str(src).expect("Failed to parse precedence");
    match &prog.items[0] {
        Item::TopLevelStmt(Stmt::Let { init: Expr::Binary { op: BinaryOp::Add, right, .. }, .. }) => {
            assert!(matches!(right.as_ref(), Expr::Binary { op: BinaryOp::Mul, .. }));
        }
        _ => panic!("Precedence error on Add/Mul"),
    }

    let src2 = "let res = a > 0 ? 100 : 200";
    let prog2 = parse_str(src2).expect("Failed to parse ternary");
    match &prog2.items[0] {
        Item::TopLevelStmt(Stmt::Let { init: Expr::Ternary { .. }, .. }) => {}
        _ => panic!("Expected Ternary Expr"),
    }

    let src3 = "let res = x + (y is Point ? 1 : 0)";
    let prog3 = parse_str(src3).expect("Failed to parse type test");
    assert_eq!(prog3.items.len(), 1);

    let src4 = "let res = obj.method(1, 2)[3]!";
    let prog4 = parse_str(src4).expect("Failed to parse member chain");
    match &prog4.items[0] {
        Item::TopLevelStmt(Stmt::Let { init: Expr::Unwrap { expr, .. }, .. }) => {
            assert!(matches!(expr.as_ref(), Expr::Index { .. }));
        }
        _ => panic!("Expected Unwrap Index"),
    }
}

#[test]
fn test_parse_statements() {
    let src = r#"
    let a = 1
    var b = 2
    b += 10
    if a > 0 {
        b = b * 2
    } else {
        b = 0
    }
    while b > 0 {
        b -= 1
        if b == 5 {
            break
        } else {
            continue
        }
    }
    for i in 0..10 {
        yield i
    }
    spawn {
        let t = 100
    }
    "#;
    let prog = parse_str(src).expect("Failed to parse statement suite");
    assert_eq!(prog.items.len(), 7);
}

#[test]
fn test_parse_import_paths() {
    let cases: &[(&str, bool, usize, &[&str])] = &[
        ("import foo.bar", false, 0, &["foo", "bar"]),
        ("import .helper", true, 1, &["helper"]),
        ("import ..sibling.mod", true, 2, &["sibling", "mod"]),
        ("import super.config", true, 2, &["config"]),
        ("import super.super.root", true, 3, &["root"]),
    ];
    for (src, is_rel, depth, segs) in cases {
        let prog = parse_str(src).unwrap_or_else(|e| panic!("`{src}` failed: {e:?}"));
        let Item::Import(imp) = &prog.items[0] else {
            panic!("`{src}` did not parse as an import");
        };
        assert_eq!(imp.path.is_relative, *is_rel, "is_relative for `{src}`");
        assert_eq!(imp.path.relative_depth, *depth, "relative_depth for `{src}`");
        assert_eq!(imp.path.segments, *segs, "segments for `{src}`");
    }
}

#[test]
fn test_parse_native_fn_decl() {
    let prog = parse_str("native fn add(a: Int, b: Int) -> Int\npub native fn no_return(x: Int)")
        .unwrap();
    match &prog.items[0] {
        Item::NativeFunction(f) => {
            assert_eq!(f.name, "add");
            assert_eq!(f.params.len(), 2);
            assert_eq!(f.params[0].name, "a");
            assert!(matches!(f.return_type, Some(TypeNode::Int(_))));
            assert!(!f.is_pub);
        }
        other => panic!("expected `native fn`, got {other:?}"),
    }
    match &prog.items[1] {
        Item::NativeFunction(f) => {
            assert_eq!(f.name, "no_return");
            assert!(f.return_type.is_none());
            assert!(f.is_pub);
        }
        other => panic!("expected `pub native fn`, got {other:?}"),
    }
}

#[test]
fn test_parse_native_fn_decl_rejects_generics() {
    let err = parse_str("native fn foo<T>(x: T) -> T").expect_err("generic native fn must not parse");
    assert!(err.0.contains("generic"), "got: {}", err.0);
}

#[test]
fn test_parse_native_fn_decl_rejects_body() {
    let err = parse_str("native fn foo(x: Int) -> Int {\n    return x\n}")
        .expect_err("native fn with a body must not parse");
    assert!(err.0.contains("no body"), "got: {}", err.0);
}

#[test]
fn test_parse_native_fn_decl_in_program_with_other_items() {
    let prog = parse_str("native fn helper(x: Int) -> Int\nfn main() -> Int {\n    return 0\n}")
        .unwrap();
    assert_eq!(prog.items.len(), 2);
    assert!(matches!(prog.items[0], Item::NativeFunction(_)));
    assert!(matches!(prog.items[1], Item::Function(_)));
}

#[test]
fn test_parse_pub_on_top_level_let_var() {
    let prog = parse_str("pub let MAX = 42\npub var count = 0\nlet local = 1").unwrap();
    match &prog.items[0] {
        Item::TopLevelStmt(Stmt::Let { name, is_pub, .. }) => {
            assert_eq!(name, "MAX");
            assert!(is_pub);
        }
        other => panic!("expected `pub let`, got {other:?}"),
    }
    match &prog.items[1] {
        Item::TopLevelStmt(Stmt::Var { name, is_pub, .. }) => {
            assert_eq!(name, "count");
            assert!(is_pub);
        }
        other => panic!("expected `pub var`, got {other:?}"),
    }
    match &prog.items[2] {
        Item::TopLevelStmt(Stmt::Let { is_pub, .. }) => assert!(!is_pub, "bare `let` is not pub"),
        other => panic!("expected `let`, got {other:?}"),
    }
}

#[test]
fn test_pub_before_a_non_declaration_is_an_error() {
    let err = parse_str("pub if true { }").expect_err("`pub if` must not parse");
    assert!(err.0.contains("pub"), "got: {}", err.0);
}
