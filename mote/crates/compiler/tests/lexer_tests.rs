use compiler::lexer::Lexer;
use compiler::token::{HolePos, StrPart, TokenKind};

#[test]
fn test_lex_keywords() {
    let src = "let mut var fn struct class enum impl trait type pub import from as native spawn return if else match while for in break continue yield try self Self true false null is super";
    let mut lexer = Lexer::new(src);
    let tokens = lexer.tokenize().expect("Tokenization failed");

    let kinds: Vec<_> = tokens.into_iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            TokenKind::Let,
            TokenKind::Mut,
            TokenKind::Var,
            TokenKind::Fn,
            TokenKind::Struct,
            TokenKind::Class,
            TokenKind::Enum,
            TokenKind::Impl,
            TokenKind::Trait,
            TokenKind::Type,
            TokenKind::Pub,
            TokenKind::Import,
            TokenKind::From,
            TokenKind::As,
            TokenKind::Native,
            TokenKind::Spawn,
            TokenKind::Return,
            TokenKind::If,
            TokenKind::Else,
            TokenKind::Match,
            TokenKind::While,
            TokenKind::For,
            TokenKind::In,
            TokenKind::Break,
            TokenKind::Continue,
            TokenKind::Yield,
            TokenKind::Try,
            TokenKind::SelfLower,
            TokenKind::SelfUpper,
            TokenKind::True,
            TokenKind::False,
            TokenKind::Null,
            TokenKind::Is,
            TokenKind::Super,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn test_lex_numeric_literals_and_bases() {
    let src = "42 0xFF 0xDEAD_BEEF 0b1010_0101 0o755 100_u64 3.5 1e-4 2.5e10 0.5_f32";
    let mut lexer = Lexer::new(src);
    let tokens = lexer.tokenize().expect("Tokenization failed");

    let kinds: Vec<_> = tokens.into_iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            TokenKind::Int(42),
            TokenKind::Int(255),
            TokenKind::Int(0xDEAD_BEEF),
            TokenKind::Int(0b1010_0101),
            TokenKind::Int(0o755),
            TokenKind::Int(100),
            TokenKind::Float(3.5),
            TokenKind::Float(1e-4),
            TokenKind::Float(2.5e10),
            TokenKind::Float(0.5),
            TokenKind::Newline,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn test_lex_operators_and_punctuation() {
    let src = "+ - * / % ^ & | ~ ! == != < <= > >= && || << >> = += -= *= /= %= &= |= ^= <<= >>= ??= -> => :: .. ..= ? ?. ?? ( ) [ ] { } , : . ;";
    let mut lexer = Lexer::new(src);
    let tokens = lexer.tokenize().expect("Tokenization failed");

    let kinds: Vec<_> = tokens.into_iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            TokenKind::Plus,
            TokenKind::Minus,
            TokenKind::Star,
            TokenKind::Slash,
            TokenKind::Percent,
            TokenKind::Caret,
            TokenKind::Ampersand,
            TokenKind::Pipe,
            TokenKind::Tilde,
            TokenKind::Bang,
            TokenKind::EqEq,
            TokenKind::NotEq,
            TokenKind::Lt,
            TokenKind::LtEq,
            TokenKind::Gt,
            TokenKind::GtEq,
            TokenKind::AndAnd,
            TokenKind::OrOr,
            TokenKind::Shl,
            TokenKind::Shr,
            TokenKind::Eq,
            TokenKind::PlusEq,
            TokenKind::MinusEq,
            TokenKind::StarEq,
            TokenKind::SlashEq,
            TokenKind::PercentEq,
            TokenKind::AmpEq,
            TokenKind::PipeEq,
            TokenKind::CaretEq,
            TokenKind::ShlEq,
            TokenKind::ShrEq,
            TokenKind::NullCoalesceEq,
            TokenKind::Arrow,
            TokenKind::FatArrow,
            TokenKind::ColonColon,
            TokenKind::DotDot,
            TokenKind::DotDotEq,
            TokenKind::Question,
            TokenKind::QuestionDot,
            TokenKind::QuestionQuestion,
            TokenKind::OpenParen,
            TokenKind::CloseParen,
            TokenKind::OpenBracket,
            TokenKind::CloseBracket,
            TokenKind::OpenBrace,
            TokenKind::CloseBrace,
            TokenKind::Comma,
            TokenKind::Colon,
            TokenKind::Dot,
            TokenKind::Semicolon,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn test_lex_strings_chars_and_escapes() {
    let src = r#""hello\nworld" '\n' '\t' '\u{1F600}' "tab:\t quote:\"""#;
    let mut lexer = Lexer::new(src);
    let tokens = lexer.tokenize().expect("Tokenization failed");

    let kinds: Vec<_> = tokens.into_iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            TokenKind::String(vec![StrPart::Lit("hello\nworld".into())]),
            TokenKind::Char('\n'),
            TokenKind::Char('\t'),
            TokenKind::Char('😀'),
            TokenKind::String(vec![StrPart::Lit("tab:\t quote:\"".into())]),
            TokenKind::Newline,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn test_lex_string_interpolation_splits_into_parts() {
    let mut lexer = Lexer::new(r#""a ${x + 1} b ${f(y)}""#);
    let tokens = lexer.tokenize().expect("Tokenization failed");
    assert_eq!(
        tokens[0].kind,
        TokenKind::String(vec![
            StrPart::Lit("a ".into()),
            StrPart::Hole("x + 1".into(), HolePos { at: 5, lines: 0, col: 0 }),
            StrPart::Lit(" b ".into()),
            StrPart::Hole("f(y)".into(), HolePos { at: 16, lines: 0, col: 0 }),
        ])
    );

    let mut lexer = Lexer::new(r#""cost is \${5}""#);
    let tokens = lexer.tokenize().expect("Tokenization failed");
    assert_eq!(
        tokens[0].kind,
        TokenKind::String(vec![StrPart::Lit("cost is ${5}".into())])
    );

    assert!(Lexer::new(r#""x ${} y""#).tokenize().is_err());
}

#[test]
fn test_lex_nested_block_comments_and_line_comments() {
    let src = r#"
        let x = 10 // single line comment
        /* multi-line comment */
        let y = 20
        /* outer /* inner nested */ still outer */
        let z = 30
    "#;
    let mut lexer = Lexer::new(src);
    let tokens = lexer.tokenize().expect("Tokenization failed");

    let kinds: Vec<_> = tokens.into_iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            TokenKind::Let,
            TokenKind::Ident("x".into()),
            TokenKind::Eq,
            TokenKind::Int(10),
            TokenKind::Newline,
            TokenKind::Let,
            TokenKind::Ident("y".into()),
            TokenKind::Eq,
            TokenKind::Int(20),
            TokenKind::Newline,
            TokenKind::Let,
            TokenKind::Ident("z".into()),
            TokenKind::Eq,
            TokenKind::Int(30),
            TokenKind::Newline,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn test_automatic_statement_insertion() {
    let src = r#"
        let a = 1
        let b = (
            2 +
            3
        )
        let c = 4
    "#;
    let mut lexer = Lexer::new(src);
    let tokens = lexer.tokenize().expect("Tokenization failed");

    let kinds: Vec<_> = tokens.into_iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            TokenKind::Let,
            TokenKind::Ident("a".into()),
            TokenKind::Eq,
            TokenKind::Int(1),
            TokenKind::Newline,
            TokenKind::Let,
            TokenKind::Ident("b".into()),
            TokenKind::Eq,
            TokenKind::OpenParen,
            TokenKind::Int(2),
            TokenKind::Plus,
            TokenKind::Int(3),
            TokenKind::CloseParen,
            TokenKind::Newline,
            TokenKind::Let,
            TokenKind::Ident("c".into()),
            TokenKind::Eq,
            TokenKind::Int(4),
            TokenKind::Newline,
            TokenKind::Eof,
        ]
    );
}
