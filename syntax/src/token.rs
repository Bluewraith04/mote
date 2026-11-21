use mote_base::{Span, Symbol};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Radix {
    Binary,
    Octal,
    Decimal,
    Hexadecimal,
}

impl Radix {
    pub(crate) fn base(self) -> u32 {
        match self {
            Radix::Binary => 2,
            Radix::Octal => 8,
            Radix::Decimal => 10,
            Radix::Hexadecimal => 16,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntSuffix {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatSuffix {
    F32,
    F64,
}

/// An integer literal's magnitude, base and written type suffix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntLiteral {
    pub value: u128,
    pub radix: Radix,
    pub suffix: Option<IntSuffix>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloatLiteral {
    pub value: f64,
    pub suffix: Option<FloatSuffix>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TokenKind {
    Ident(Symbol),
    Int(IntLiteral),
    Float(FloatLiteral),
    String(Symbol),
    InterpStart(Symbol),
    InterpMid(Symbol),
    InterpEnd(Symbol),

    Let,
    Var,
    Fn,
    Struct,
    Class,
    Enum,
    Trait,
    Type,
    Pub,
    Import,
    From,
    As,
    Native,
    Scope,
    Spawn,
    With,
    Return,
    If,
    Else,
    Match,
    While,
    For,
    In,
    Break,
    Continue,
    Yield,
    SelfLower,
    SelfUpper,
    True,
    False,
    Null,
    Super,
    Is,

    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Ampersand,
    Pipe,
    Tilde,
    Bang,
    EqEq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    AndAnd,
    OrOr,
    Shl,
    Shr,
    Eq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    AmpEq,
    PipeEq,
    CaretEq,
    ShlEq,
    ShrEq,
    NullCoalesceEq,
    Arrow,
    FatArrow,
    ColonColon,
    DotDotEq,
    DotDotDot,
    DotDot,
    Question,
    QuestionDot,
    QuestionQuestion,
    OpenParen,
    CloseParen,
    OpenBrace,
    CloseBrace,
    OpenBracket,
    CloseBracket,
    Comma,
    Colon,
    Dot,
    Semicolon,
    At,

    Newline,
    Eof,
    Error,
}

impl TokenKind {
    /// Whether a newline after this token ends a statement.
    pub(crate) fn ends_statement(self) -> bool {
        matches!(
            self,
            TokenKind::Ident(_)
                | TokenKind::Int(_)
                | TokenKind::Float(_)
                | TokenKind::String(_)
                | TokenKind::InterpEnd(_)
                | TokenKind::True
                | TokenKind::False
                | TokenKind::Null
                | TokenKind::Return
                | TokenKind::Break
                | TokenKind::Continue
                | TokenKind::Yield
                | TokenKind::CloseParen
                | TokenKind::CloseBracket
                | TokenKind::CloseBrace
                | TokenKind::Question
                | TokenKind::Bang
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// Every keyword with its token, sorted by word.
pub const KEYWORDS: [(&str, TokenKind); 33] = [
    ("Self", TokenKind::SelfUpper),
    ("as", TokenKind::As),
    ("break", TokenKind::Break),
    ("class", TokenKind::Class),
    ("continue", TokenKind::Continue),
    ("else", TokenKind::Else),
    ("enum", TokenKind::Enum),
    ("false", TokenKind::False),
    ("fn", TokenKind::Fn),
    ("for", TokenKind::For),
    ("from", TokenKind::From),
    ("if", TokenKind::If),
    ("import", TokenKind::Import),
    ("in", TokenKind::In),
    ("is", TokenKind::Is),
    ("let", TokenKind::Let),
    ("match", TokenKind::Match),
    ("native", TokenKind::Native),
    ("null", TokenKind::Null),
    ("pub", TokenKind::Pub),
    ("return", TokenKind::Return),
    ("scope", TokenKind::Scope),
    ("self", TokenKind::SelfLower),
    ("spawn", TokenKind::Spawn),
    ("struct", TokenKind::Struct),
    ("super", TokenKind::Super),
    ("trait", TokenKind::Trait),
    ("true", TokenKind::True),
    ("type", TokenKind::Type),
    ("var", TokenKind::Var),
    ("while", TokenKind::While),
    ("with", TokenKind::With),
    ("yield", TokenKind::Yield),
];

pub(crate) fn keyword(word: &str) -> Option<TokenKind> {
    KEYWORDS.binary_search_by_key(&word, |(text, _)| text).ok().map(|at| KEYWORDS[at].1)
}
