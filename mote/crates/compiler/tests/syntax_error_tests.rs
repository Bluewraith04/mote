use compiler::lexer::Lexer;
use compiler::parser::Parser;

fn error_of(src: &str) -> String {
    let tokens = match Lexer::new(src).tokenize() {
        Ok(tokens) => tokens,
        Err((message, _)) => return message,
    };
    match Parser::new(tokens).parse() {
        Ok(_) => panic!("parsed without an error: {src}"),
        Err((message, _)) => message,
    }
}

#[test]
fn missing_brace_names_what_it_follows() {
    assert_eq!(error_of("fn main() {\n  if 1 > 0\n    f()\n  }\n}"), "expected `{` after the condition, found `f`");
    assert_eq!(error_of("fn main() {\n  while true\n    f()\n}"), "expected `{` after the condition, found `f`");
    assert_eq!(error_of("fn main() {\n  for i in xs\n    f()\n}"), "expected `{` after the loop header, found `f`");
    assert_eq!(error_of("fn main() {\n  match x\n    f()\n}"), "expected `{` after the value being matched, found `f`");
    assert_eq!(error_of("fn main()\n  f()\n}"), "expected `{` after the parameters, found `f`");
    assert_eq!(error_of("fn main() -> Int\n  f()\n}"), "expected `{` after the return type, found `f`");
    assert_eq!(error_of("struct P\n  x: Int\n}"), "expected `{` after the struct's name, found `x`");
}

#[test]
fn expected_and_found_are_in_words() {
    assert_eq!(error_of("fn add(a: Int, b: Int -> Int {\n}"), "expected `)`, found `->`");
    assert_eq!(error_of("fn main() {\n  let xs = [1, 2\n  f()\n}"), "expected `]`, found `f`");
    assert_eq!(error_of("fn main() {\n  let a = 1 +\n  let b = 2\n}"), "expected an expression, found the keyword `let`");
    assert_eq!(error_of("fn main() {\n  let y: = 3\n}"), "expected a type, found `=`");
    assert_eq!(error_of("fn main() {\n  let type = 4\n}"), "expected a name, found the keyword `type`");
    assert_eq!(error_of("fn main() {\n  f(1,\n}"), "expected an expression, found `}`");
    assert_eq!(error_of("fn main() {\n  let n = 5\n  f(n 2)\n}"), "expected `)`, found the number 2");
    assert_eq!(error_of("fn main() {\n  let s = f(\"a\" \"b\")\n}"), "expected `)`, found a string");
}

#[test]
fn the_end_of_the_file_is_named() {
    assert_eq!(error_of("fn main() {"), "expected `}`, found the end of the file");
}

#[test]
fn lexer_errors_say_what_is_wrong() {
    assert_eq!(error_of("fn main() {\n  let s = \"open\n}"), "the string is never closed");
    assert_eq!(error_of("fn main() {\n  let x = # 5\n}"), "unexpected character `#`");
    assert_eq!(error_of("fn main() {\n  let t = \"a ${} b\"\n}"), "a `${…}` in the string is empty or never closed");
    assert_eq!(error_of("fn main() {\n  /* open\n}"), "the comment is never closed");
    assert_eq!(error_of("fn main() {\n  let n = 99999999999999999999\n}"), "the number `99999999999999999999` is malformed or too large");
}
