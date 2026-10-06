use logos::{Lexer, Logos};
use crate::span::Span;

fn block_comment(lex: &mut Lexer<TokenKind>) -> bool {
    let remainder = lex.remainder();
    let mut depth = 1;
    let mut bytes = remainder.as_bytes().iter().enumerate();
    let mut end = 0;
    while let Some((i, &b)) = bytes.next() {
        if b == b'/' && remainder.as_bytes().get(i + 1) == Some(&b'*') {
            depth += 1;
            bytes.next();
        } else if b == b'*' && remainder.as_bytes().get(i + 1) == Some(&b'/') {
            depth -= 1;
            bytes.next();
            if depth == 0 {
                end = i + 2;
                break;
            }
        }
    }
    if depth == 0 {
        lex.bump(end);
        true
    } else {
        false
    }
}

fn parse_int(lex: &mut Lexer<TokenKind>) -> Result<i64, ()> {
    let slice = lex.slice();
    let num_part = if slice.starts_with("0x") || slice.starts_with("0X") {
        let after_prefix = &slice[2..];
        if let Some(pos) = after_prefix.find(|c: char| c.is_ascii_alphabetic() && !c.is_ascii_hexdigit()) {
            let sub = &after_prefix[..pos];
            let clean = sub.trim_end_matches('_');
            format!("0x{}", clean)
        } else {
            slice.to_string()
        }
    } else if slice.starts_with("0b") || slice.starts_with("0B") {
        let after_prefix = &slice[2..];
        if let Some(pos) = after_prefix.find(|c: char| c != '0' && c != '1' && c != '_') {
            let sub = &after_prefix[..pos];
            let clean = sub.trim_end_matches('_');
            format!("0b{}", clean)
        } else {
            slice.to_string()
        }
    } else if slice.starts_with("0o") || slice.starts_with("0O") {
        let after_prefix = &slice[2..];
        if let Some(pos) = after_prefix.find(|c: char| !(('0'..='7').contains(&c)) && c != '_') {
            let sub = &after_prefix[..pos];
            let clean = sub.trim_end_matches('_');
            format!("0o{}", clean)
        } else {
            slice.to_string()
        }
    } else if let Some(idx) = slice.find(|c: char| c.is_ascii_alphabetic()) {
        let prefix = &slice[..idx];
        prefix.trim_end_matches('_').to_string()
    } else {
        slice.to_string()
    };

    let s = num_part.replace('_', "");
    if s.starts_with("0x") || s.starts_with("0X") {
        i64::from_str_radix(&s[2..], 16).map_err(|_| ())
    } else if s.starts_with("0b") || s.starts_with("0B") {
        i64::from_str_radix(&s[2..], 2).map_err(|_| ())
    } else if s.starts_with("0o") || s.starts_with("0O") {
        i64::from_str_radix(&s[2..], 8).map_err(|_| ())
    } else {
        s.parse::<i64>().map_err(|_| ())
    }
}

fn parse_float(lex: &mut Lexer<TokenKind>) -> Result<f64, ()> {
    let slice = lex.slice();
    let num_part = if let Some(pos) = slice.rfind(['f', 'F']) {
        if pos > 0 && slice.as_bytes()[pos - 1] == b'_' {
            &slice[..pos - 1]
        } else if pos > 0 {
            &slice[..pos]
        } else {
            slice
        }
    } else {
        slice
    };
    let s = num_part.replace('_', "");
    s.parse::<f64>().map_err(|_| ())
}

/// One segment of a string literal: a text run with escapes resolved, or a `${ … }` hole carrying its raw source.
#[derive(Clone, Debug, PartialEq)]
pub enum StrPart {
    Lit(String),
    Hole(String, HolePos),
}

/// Where a hole's source starts inside its string literal token.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HolePos {
    /// Byte offset from the token's start.
    pub at: usize,
    /// Line breaks before the hole.
    pub lines: usize,
    /// 1-based column when `lines > 0`.
    pub col: usize,
}

fn parse_string(lex: &mut Lexer<TokenKind>) -> Result<Vec<StrPart>, ()> {
    let slice = lex.slice();
    if slice.len() < 2 || !slice.starts_with('"') || !slice.ends_with('"') {
        return Err(());
    }
    let inner = &slice[1..slice.len() - 1];
    let mut parts: Vec<StrPart> = Vec::new();
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('0') => out.push('\0'),
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('\\') => out.push('\\'),
                Some('\'') => out.push('\''),
                Some('"') => out.push('"'),
                Some('$') => out.push('$'),
                Some('u') => {
                    if chars.next() == Some('{') {
                        let mut hex_str = String::new();
                        for h in chars.by_ref() {
                            if h == '}' {
                                break;
                            }
                            hex_str.push(h);
                        }
                        if let Ok(code) = u32::from_str_radix(&hex_str, 16)
                            && let Some(ch) = char::from_u32(code) {
                                out.push(ch);
                            }
                    }
                }
                Some(other) => out.push(other),
                None => {}
            }
        } else if c == '$' && chars.clone().next() == Some('{') {
            chars.next();
            let off = inner.len() - chars.as_str().len();
            let before = &inner[..off];
            let lines = before.matches('\n').count();
            let col = before.rfind('\n').map_or(0, |nl| before.len() - nl);
            let pos = HolePos { at: off + 1, lines, col };
            let mut depth = 1usize;
            let mut src = String::new();
            for ec in chars.by_ref() {
                match ec {
                    '{' => {
                        depth += 1;
                        src.push(ec);
                    }
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                        src.push(ec);
                    }
                    _ => src.push(ec),
                }
            }
            if depth != 0 || src.trim().is_empty() {
                return Err(());
            }
            if !out.is_empty() {
                parts.push(StrPart::Lit(std::mem::take(&mut out)));
            }
            parts.push(StrPart::Hole(src, pos));
        } else {
            out.push(c);
        }
    }
    if !out.is_empty() || parts.is_empty() {
        parts.push(StrPart::Lit(out));
    }
    Ok(parts)
}

fn parse_char(lex: &mut Lexer<TokenKind>) -> Result<char, ()> {
    let slice = lex.slice();
    if slice.len() < 3 || !slice.starts_with('\'') || !slice.ends_with('\'') {
        return Err(());
    }
    let inner = &slice[1..slice.len() - 1];
    if let Some(escaped) = inner.strip_prefix('\\') {
        if let Some(hex) = escaped.strip_prefix("u{").and_then(|h| h.strip_suffix('}')) {
            let code = u32::from_str_radix(hex, 16).map_err(|_| ())?;
            char::from_u32(code).ok_or(())
        } else {
            match escaped {
                "0" => Ok('\0'),
                "n" => Ok('\n'),
                "r" => Ok('\r'),
                "t" => Ok('\t'),
                "\\" => Ok('\\'),
                "'" => Ok('\''),
                "\"" => Ok('"'),
                _ => inner.chars().nth(1).ok_or(()),
            }
        }
    } else {
        inner.chars().next().ok_or(())
    }
}

#[derive(Logos, Clone, Debug, PartialEq)]
#[logos(skip r"[ \t\r]+")]
#[logos(skip r"//[^\n]*")]
pub enum TokenKind {
    #[regex(r"/\*", block_comment)]
    BlockComment,

    #[token("\n")]
    Newline,

    #[token("let")]
    Let,
    #[token("mut")]
    Mut,
    #[token("var")]
    Var,
    #[token("fn")]
    Fn,
    #[token("struct")]
    Struct,
    #[token("class")]
    Class,
    #[token("enum")]
    Enum,
    #[token("impl")]
    Impl,
    #[token("trait")]
    Trait,
    #[token("type")]
    Type,
    #[token("pub")]
    Pub,
    #[token("import")]
    Import,
    #[token("from")]
    From,
    #[token("as")]
    As,
    #[token("native")]
    Native,
    #[token("scope")]
    Scope,
    #[token("spawn")]
    Spawn,
    #[token("with")]
    With,
    #[token("return")]
    Return,
    #[token("if")]
    If,
    #[token("else")]
    Else,
    #[token("match")]
    Match,
    #[token("while")]
    While,
    #[token("for")]
    For,
    #[token("in")]
    In,
    #[token("break")]
    Break,
    #[token("continue")]
    Continue,
    #[token("yield")]
    Yield,
    #[token("try")]
    Try,
    #[token("self")]
    SelfLower,
    #[token("Self")]
    SelfUpper,
    #[token("true")]
    True,
    #[token("false")]
    False,
    #[token("null")]
    Null,
    #[token("super")]
    Super,
    #[token("is")]
    Is,

    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*", |lex| lex.slice().to_string())]
    Ident(String),

    #[regex(r"0x[0-9a-fA-F_]+(_?[ui](8|16|32|64|size))?", parse_int)]
    #[regex(r"0b[01_]+(_?[ui](8|16|32|64|size))?", parse_int)]
    #[regex(r"0o[0-7_]+(_?[ui](8|16|32|64|size))?", parse_int)]
    #[regex(r"[0-9][0-9_]*(_?[ui](8|16|32|64|size))?", parse_int)]
    Int(i64),

    #[regex(r"[0-9][0-9_]*\.[0-9][0-9_]*([eE][+-]?[0-9_]+)?(_?f(32|64))?", parse_float)]
    #[regex(r"[0-9][0-9_]*[eE][+-]?[0-9_]+(_?f(32|64))?", parse_float)]
    Float(f64),

    #[regex(r#""([^"\\]|\\.)*""#, parse_string)]
    String(Vec<StrPart>),

    #[regex(r"'([^'\\]|\\([^\n]|u\{[0-9a-fA-F]+\}))*'", parse_char)]
    Char(char),

    InterpStart(String),
    InterpMid(String),
    InterpEnd(String),

    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("%")]
    Percent,
    #[token("^")]
    Caret,
    #[token("&")]
    Ampersand,
    #[token("|")]
    Pipe,
    #[token("~")]
    Tilde,
    #[token("!")]
    Bang,

    #[token("==")]
    EqEq,
    #[token("!=")]
    NotEq,
    #[token("<")]
    Lt,
    #[token("<=")]
    LtEq,
    #[token(">")]
    Gt,
    #[token(">=")]
    GtEq,

    #[token("&&")]
    AndAnd,
    #[token("||")]
    OrOr,

    #[token("<<")]
    Shl,
    #[token(">>")]
    Shr,

    #[token("=")]
    Eq,
    #[token("+=")]
    PlusEq,
    #[token("-=")]
    MinusEq,
    #[token("*=")]
    StarEq,
    #[token("/=")]
    SlashEq,
    #[token("%=")]
    PercentEq,
    #[token("&=")]
    AmpEq,
    #[token("|=")]
    PipeEq,
    #[token("^=")]
    CaretEq,
    #[token("<<=")]
    ShlEq,
    #[token(">>=")]
    ShrEq,
    #[token("??=")]
    NullCoalesceEq,

    #[token("->")]
    Arrow,
    #[token("=>")]
    FatArrow,
    #[token("::")]
    ColonColon,
    #[token("..=")]
    DotDotEq,
    #[token("...")]
    DotDotDot,
    #[token("..")]
    DotDot,

    #[token("?")]
    Question,
    #[token("?.")]
    QuestionDot,
    #[token("??")]
    QuestionQuestion,

    #[token("(")]
    OpenParen,
    #[token(")")]
    CloseParen,
    #[token("{")]
    OpenBrace,
    #[token("}")]
    CloseBrace,
    #[token("[")]
    OpenBracket,
    #[token("]")]
    CloseBracket,
    #[token(",")]
    Comma,
    #[token(":")]
    Colon,
    #[token(".")]
    Dot,
    #[token(";")]
    Semicolon,
    #[token("@")]
    At,

    Eof,
}

impl TokenKind {
    /// The token as an error message names it: its text in backticks, or a plain-words description.
    pub fn describe(&self) -> String {
        use TokenKind::*;
        let text = match self {
            Newline => return "the end of the line".into(),
            Eof => return "the end of the file".into(),
            BlockComment => return "a comment".into(),
            Ident(name) => return format!("`{name}`"),
            Int(n) => return format!("the number {n}"),
            Float(x) => return format!("the number {x}"),
            String(_) | InterpStart(_) | InterpMid(_) | InterpEnd(_) => return "a string".into(),
            Char(_) => return "a character".into(),
            Let => "let",
            Mut => "mut",
            Var => "var",
            Fn => "fn",
            Struct => "struct",
            Class => "class",
            Enum => "enum",
            Impl => "impl",
            Trait => "trait",
            Type => "type",
            Pub => "pub",
            Import => "import",
            From => "from",
            As => "as",
            Native => "native",
            Scope => "scope",
            Spawn => "spawn",
            With => "with",
            Return => "return",
            If => "if",
            Else => "else",
            Match => "match",
            While => "while",
            For => "for",
            In => "in",
            Break => "break",
            Continue => "continue",
            Yield => "yield",
            Try => "try",
            SelfLower => "self",
            SelfUpper => "Self",
            True => "true",
            False => "false",
            Null => "null",
            Super => "super",
            Is => "is",
            Plus => "+",
            Minus => "-",
            Star => "*",
            Slash => "/",
            Percent => "%",
            Caret => "^",
            Ampersand => "&",
            Pipe => "|",
            Tilde => "~",
            Bang => "!",
            EqEq => "==",
            NotEq => "!=",
            Lt => "<",
            LtEq => "<=",
            Gt => ">",
            GtEq => ">=",
            AndAnd => "&&",
            OrOr => "||",
            Shl => "<<",
            Shr => ">>",
            Eq => "=",
            PlusEq => "+=",
            MinusEq => "-=",
            StarEq => "*=",
            SlashEq => "/=",
            PercentEq => "%=",
            AmpEq => "&=",
            PipeEq => "|=",
            CaretEq => "^=",
            ShlEq => "<<=",
            ShrEq => ">>=",
            NullCoalesceEq => "??=",
            Arrow => "->",
            FatArrow => "=>",
            ColonColon => "::",
            DotDotEq => "..=",
            DotDotDot => "...",
            DotDot => "..",
            Question => "?",
            QuestionDot => "?.",
            QuestionQuestion => "??",
            OpenParen => "(",
            CloseParen => ")",
            OpenBrace => "{",
            CloseBrace => "}",
            OpenBracket => "[",
            CloseBracket => "]",
            Comma => ",",
            Colon => ":",
            Dot => ".",
            Semicolon => ";",
            At => "@",
        };
        if self.is_keyword() { format!("the keyword `{text}`") } else { format!("`{text}`") }
    }

    fn is_keyword(&self) -> bool {
        use TokenKind::*;
        matches!(
            self,
            Let | Mut | Var | Fn | Struct | Class | Enum | Impl | Trait | Type | Pub | Import | From | As | Native | Scope
                | Spawn | With | Return | If | Else | Match | While | For | In | Break | Continue | Yield | Try
                | SelfLower | SelfUpper | True | False | Null | Super | Is
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }

    /// Returns true if this token kind can potentially end a statement before a newline.
    pub(crate) fn can_end_statement(&self) -> bool {
        matches!(
            self.kind,
            TokenKind::Ident(_)
                | TokenKind::Int(_)
                | TokenKind::Float(_)
                | TokenKind::String(_)
                | TokenKind::InterpEnd(_)
                | TokenKind::Char(_)
                | TokenKind::True
                | TokenKind::False
                | TokenKind::Null
                | TokenKind::Return
                | TokenKind::Break
                | TokenKind::Continue
                | TokenKind::Yield
                | TokenKind::CloseParen
                | TokenKind::CloseBrace
                | TokenKind::CloseBracket
                | TokenKind::Question
                | TokenKind::Bang
        )
    }
}
