use mote_base::{Diagnostic, DiagnosticSink, Interner, SourceFile, Span};

use crate::token::{keyword, FloatLiteral, FloatSuffix, IntLiteral, IntSuffix, Radix, Token, TokenKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommentKind {
    Line,
    Doc,
    Block,
}

/// A comment, kept outside the token stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Comment {
    pub kind: CommentKind,
    pub span: Span,
}

/// The tokens of one file, ending in `Eof`, and its comments.
#[derive(Debug)]
pub struct Lexed {
    pub tokens: Vec<Token>,
    pub comments: Vec<Comment>,
}

struct Hole {
    depth: u32,
    open: Span,
}

enum Stop {
    Quote,
    Interpolation,
    End,
}

struct Lexer<'a> {
    text: &'a str,
    base: u32,
    pos: usize,
    interner: &'a mut Interner,
    sink: &'a mut DiagnosticSink,
    tokens: Vec<Token>,
    comments: Vec<Comment>,
    depth: u32,
    holes: Vec<Hole>,
    ends_statement: bool,
    line_start: bool,
}

/// Splits a file into tokens, reporting malformed text to the sink.
pub fn lex(file: &SourceFile, interner: &mut Interner, sink: &mut DiagnosticSink) -> Lexed {
    Lexer {
        text: &file.text,
        base: file.start,
        pos: 0,
        interner,
        sink,
        tokens: Vec::new(),
        comments: Vec::new(),
        depth: 0,
        holes: Vec::new(),
        ends_statement: false,
        line_start: true,
    }
    .run()
}

impl Lexer<'_> {
    fn run(mut self) -> Lexed {
        while let Some(byte) = self.byte(0) {
            match byte {
                b' ' | b'\t' | b'\r' => self.pos += 1,
                b'\n' => self.newline(),
                b'/' if matches!(self.byte(1), Some(b'/' | b'*')) => self.comment(),
                b'"' => self.string_start(),
                b'0'..=b'9' => self.number(),
                b'a'..=b'z' | b'A'..=b'Z' | b'_' => self.word(),
                _ => self.punctuation(),
            }
        }
        self.finish()
    }

    fn finish(mut self) -> Lexed {
        for hole in std::mem::take(&mut self.holes) {
            self.sink.emit(Diagnostic::error("the `${` is never closed", hole.open));
        }
        let end = self.text.len();
        if self.depth == 0 && self.ends_statement {
            self.push(TokenKind::Newline, end, end);
        }
        self.push(TokenKind::Eof, end, end);
        Lexed { tokens: self.tokens, comments: self.comments }
    }

    fn span(&self, from: usize, to: usize) -> Span {
        Span::new(self.base + from as u32, self.base + to as u32)
    }

    fn byte(&self, offset: usize) -> Option<u8> {
        self.text.as_bytes().get(self.pos + offset).copied()
    }

    fn error(&mut self, message: impl Into<String>, from: usize, to: usize) {
        let span = self.span(from, to);
        self.sink.emit(Diagnostic::error(message, span));
    }

    fn push(&mut self, kind: TokenKind, from: usize, to: usize) {
        self.ends_statement = kind.ends_statement();
        self.line_start = false;
        let span = self.span(from, to);
        self.tokens.push(Token { kind, span });
    }

    fn fail(&mut self, message: impl Into<String>, from: usize, to: usize) {
        self.error(message, from, to);
        self.push(TokenKind::Error, from, to);
    }

    fn newline(&mut self) {
        let at = self.pos;
        self.pos += 1;
        if self.depth == 0 && self.ends_statement && !self.member_follows() {
            self.push(TokenKind::Newline, at, at + 1);
        }
        self.line_start = true;
    }

    fn member_follows(&self) -> bool {
        let bytes = self.text.as_bytes();
        let mut i = self.pos;
        loop {
            match (bytes.get(i), bytes.get(i + 1)) {
                (Some(b' ' | b'\t' | b'\r' | b'\n'), _) => i += 1,
                (Some(b'/'), Some(b'/')) => i += bytes[i..].iter().position(|&b| b == b'\n').unwrap_or(bytes.len() - i),
                (Some(b'.'), next) => return next != Some(&b'.'),
                (Some(b'?'), next) => return next == Some(&b'.'),
                _ => return false,
            }
        }
    }

    fn add_comment(&mut self, kind: CommentKind, from: usize, to: usize) {
        let span = self.span(from, to);
        self.comments.push(Comment { kind, span });
        self.line_start = false;
    }

    fn comment(&mut self) {
        let text = self.text;
        let start = self.pos;
        if self.byte(1) == Some(b'/') {
            let end = text[start..].find('\n').map_or(text.len(), |i| start + i);
            let line = &text[start..end];
            let doc = self.line_start && line.starts_with("///") && !line.starts_with("////");
            self.pos = end;
            self.add_comment(if doc { CommentKind::Doc } else { CommentKind::Line }, start, end);
            return;
        }
        let bytes = text.as_bytes();
        let mut depth = 1;
        let mut i = start + 2;
        while i < bytes.len() && depth > 0 {
            match (bytes[i], bytes.get(i + 1)) {
                (b'/', Some(b'*')) => {
                    depth += 1;
                    i += 2;
                }
                (b'*', Some(b'/')) => {
                    depth -= 1;
                    i += 2;
                }
                _ => i += 1,
            }
        }
        self.pos = i;
        if depth > 0 {
            self.error("the comment is never closed", start, i);
        }
        self.add_comment(CommentKind::Block, start, i);
    }

    fn word(&mut self) {
        let text = self.text;
        let start = self.pos;
        let len = text.as_bytes()[start..].iter().take_while(|b| b.is_ascii_alphanumeric() || **b == b'_').count();
        self.pos += len;
        let word = &text[start..self.pos];
        let kind = keyword(word).unwrap_or_else(|| TokenKind::Ident(self.interner.intern(word)));
        self.push(kind, start, self.pos);
    }

    fn number(&mut self) {
        let text = self.text;
        let start = self.pos;
        let after_dot = matches!(self.tokens.last(), Some(Token { kind: TokenKind::Dot, .. }));
        let radix = if after_dot { Radix::Decimal } else { self.radix_prefix() };
        let digits_start = self.pos;
        let (value, digits, overflow) = self.digits(radix, !after_dot);

        let mut float = false;
        if radix == Radix::Decimal && !after_dot {
            if self.byte(0) == Some(b'.') && self.byte(1).is_some_and(|b| b.is_ascii_digit()) {
                float = true;
                self.pos += 1;
                self.skip_decimal();
            }
            if matches!(self.byte(0), Some(b'e' | b'E')) && self.exponent_follows() {
                float = true;
                self.pos += 1;
                if matches!(self.byte(0), Some(b'+' | b'-')) {
                    self.pos += 1;
                }
                self.skip_decimal();
            }
        }
        let numeric_end = self.pos;

        if digits == 0 {
            self.skip_word();
            return self.fail("the number has no digits", start, self.pos);
        }

        let (int_suffix, float_suffix) = match (float, after_dot) {
            (_, true) => (None, None),
            (true, false) => (None, self.suffix(&[("f32", FloatSuffix::F32), ("f64", FloatSuffix::F64)])),
            (false, false) => (self.suffix(&INT_SUFFIXES), None),
        };

        if self.byte(0).is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_') {
            self.skip_word();
            return self.fail("the number is malformed", start, self.pos);
        }

        if float {
            let cleaned: String = text[digits_start..numeric_end].chars().filter(|c| *c != '_').collect();
            match cleaned.parse::<f64>() {
                Ok(value) if value.is_finite() => {
                    self.push(TokenKind::Float(FloatLiteral { value, suffix: float_suffix }), start, self.pos)
                }
                _ => self.fail("the number is too large", start, self.pos),
            }
        } else if overflow {
            self.fail("the number is too large", start, self.pos);
        } else {
            self.push(TokenKind::Int(IntLiteral { value, radix, suffix: int_suffix }), start, self.pos);
        }
    }

    fn radix_prefix(&mut self) -> Radix {
        let radix = match (self.byte(0), self.byte(1)) {
            (Some(b'0'), Some(b'x')) => Radix::Hexadecimal,
            (Some(b'0'), Some(b'b')) => Radix::Binary,
            (Some(b'0'), Some(b'o')) => Radix::Octal,
            _ => return Radix::Decimal,
        };
        self.pos += 2;
        radix
    }

    fn digits(&mut self, radix: Radix, underscores: bool) -> (u128, usize, bool) {
        let base = radix.base();
        let (mut value, mut count, mut overflow) = (0u128, 0usize, false);
        while let Some(byte) = self.byte(0) {
            if byte == b'_' && underscores {
                self.pos += 1;
                continue;
            }
            let Some(digit) = (byte as char).to_digit(base) else { break };
            value = match value.checked_mul(base as u128).and_then(|v| v.checked_add(digit as u128)) {
                Some(v) => v,
                None => {
                    overflow = true;
                    0
                }
            };
            count += 1;
            self.pos += 1;
        }
        (value, count, overflow)
    }

    fn skip_decimal(&mut self) {
        while self.byte(0).is_some_and(|b| b.is_ascii_digit() || b == b'_') {
            self.pos += 1;
        }
    }

    fn skip_word(&mut self) {
        while self.byte(0).is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_') {
            self.pos += 1;
        }
    }

    fn exponent_follows(&self) -> bool {
        match self.byte(1) {
            Some(b'0'..=b'9') => true,
            Some(b'+' | b'-') => self.byte(2).is_some_and(|b| b.is_ascii_digit()),
            _ => false,
        }
    }

    fn suffix<T: Copy>(&mut self, table: &[(&str, T)]) -> Option<T> {
        let text = self.text;
        let rest = &text[self.pos..];
        for &(name, value) in table {
            if let Some(after) = rest.strip_prefix(name)
                && !after.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
            {
                self.pos += name.len();
                return Some(value);
            }
        }
        None
    }

    fn string_start(&mut self) {
        let start = self.pos;
        self.pos += 1;
        let (text, stop) = self.string_text();
        match stop {
            Stop::Quote => {
                let symbol = self.interner.intern(&text);
                self.push(TokenKind::String(symbol), start, self.pos);
            }
            Stop::Interpolation => {
                let symbol = self.interner.intern(&text);
                self.push(TokenKind::InterpStart(symbol), start, self.pos);
                let open = self.span(self.pos - 2, self.pos);
                self.holes.push(Hole { depth: 0, open });
                self.depth += 1;
            }
            Stop::End => self.fail("the string is never closed", start, self.pos),
        }
    }

    fn hole_close(&mut self, start: usize) {
        let open = self.holes.last().map_or(self.span(start, start), |hole| hole.open);
        if matches!(self.tokens.last(), Some(Token { kind: TokenKind::InterpStart(_) | TokenKind::InterpMid(_), .. })) {
            let span = Span::new(open.start, self.base + self.pos as u32);
            self.sink.emit(Diagnostic::error("the `${}` is empty", span));
        }
        let (text, stop) = self.string_text();
        match stop {
            Stop::Quote => {
                let symbol = self.interner.intern(&text);
                self.holes.pop();
                self.depth = self.depth.saturating_sub(1);
                self.push(TokenKind::InterpEnd(symbol), start, self.pos);
            }
            Stop::Interpolation => {
                let symbol = self.interner.intern(&text);
                self.push(TokenKind::InterpMid(symbol), start, self.pos);
                let open = self.span(self.pos - 2, self.pos);
                if let Some(hole) = self.holes.last_mut() {
                    *hole = Hole { depth: 0, open };
                }
            }
            Stop::End => {
                self.holes.pop();
                self.depth = self.depth.saturating_sub(1);
                self.fail("the string is never closed", start, self.pos);
            }
        }
    }

    fn string_text(&mut self) -> (String, Stop) {
        let text = self.text;
        let mut out = String::new();
        loop {
            let Some(c) = text[self.pos..].chars().next() else { return (out, Stop::End) };
            match c {
                '"' => {
                    self.pos += 1;
                    return (out, Stop::Quote);
                }
                '$' if self.byte(1) == Some(b'{') => {
                    self.pos += 2;
                    return (out, Stop::Interpolation);
                }
                '\\' => out.extend(self.escape()),
                _ => {
                    out.push(c);
                    self.pos += c.len_utf8();
                }
            }
        }
    }

    fn escape(&mut self) -> Option<char> {
        let text = self.text;
        let start = self.pos;
        self.pos += 1;
        let c = text[self.pos..].chars().next()?;
        self.pos += c.len_utf8();
        Some(match c {
            '0' => '\0',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            '\\' => '\\',
            '\'' => '\'',
            '"' => '"',
            '$' => '$',
            'u' => return self.unicode_escape(start),
            _ => {
                self.error(format!("unknown escape `\\{c}`"), start, self.pos);
                return None;
            }
        })
    }

    fn unicode_escape(&mut self, start: usize) -> Option<char> {
        let text = self.text;
        if self.byte(0) != Some(b'{') {
            self.error("a unicode escape is written `\\u{…}`", start, self.pos);
            return None;
        }
        self.pos += 1;
        let digits_start = self.pos;
        while self.byte(0).is_some_and(|b| b.is_ascii_hexdigit()) {
            self.pos += 1;
        }
        let digits = &text[digits_start..self.pos];
        if self.byte(0) != Some(b'}') || digits.is_empty() {
            self.error("a unicode escape is written `\\u{…}`", start, self.pos);
            return None;
        }
        self.pos += 1;
        let c = u32::from_str_radix(digits, 16).ok().and_then(char::from_u32);
        if c.is_none() {
            self.error("not a unicode character", start, self.pos);
        }
        c
    }

    fn punctuation(&mut self) {
        use TokenKind::*;
        let text = self.text;
        let start = self.pos;
        let (kind, len) = match &text.as_bytes()[start..] {
            [b'.', b'.', b'.', ..] => (DotDotDot, 3),
            [b'.', b'.', b'=', ..] => (DotDotEq, 3),
            [b'<', b'<', b'=', ..] => (ShlEq, 3),
            [b'>', b'>', b'=', ..] => (ShrEq, 3),
            [b'?', b'?', b'=', ..] => (NullCoalesceEq, 3),
            [b'=', b'=', ..] => (EqEq, 2),
            [b'!', b'=', ..] => (NotEq, 2),
            [b'<', b'=', ..] => (LtEq, 2),
            [b'>', b'=', ..] => (GtEq, 2),
            [b'&', b'&', ..] => (AndAnd, 2),
            [b'|', b'|', ..] => (OrOr, 2),
            [b'<', b'<', ..] => (Shl, 2),
            [b'>', b'>', ..] => (Shr, 2),
            [b'+', b'=', ..] => (PlusEq, 2),
            [b'-', b'=', ..] => (MinusEq, 2),
            [b'*', b'=', ..] => (StarEq, 2),
            [b'/', b'=', ..] => (SlashEq, 2),
            [b'%', b'=', ..] => (PercentEq, 2),
            [b'&', b'=', ..] => (AmpEq, 2),
            [b'|', b'=', ..] => (PipeEq, 2),
            [b'^', b'=', ..] => (CaretEq, 2),
            [b'-', b'>', ..] => (Arrow, 2),
            [b'=', b'>', ..] => (FatArrow, 2),
            [b':', b':', ..] => (ColonColon, 2),
            [b'.', b'.', ..] => (DotDot, 2),
            [b'?', b'.', ..] => (QuestionDot, 2),
            [b'?', b'?', ..] => (QuestionQuestion, 2),
            [b'+', ..] => (Plus, 1),
            [b'-', ..] => (Minus, 1),
            [b'*', ..] => (Star, 1),
            [b'/', ..] => (Slash, 1),
            [b'%', ..] => (Percent, 1),
            [b'^', ..] => (Caret, 1),
            [b'&', ..] => (Ampersand, 1),
            [b'|', ..] => (Pipe, 1),
            [b'~', ..] => (Tilde, 1),
            [b'!', ..] => (Bang, 1),
            [b'<', ..] => (Lt, 1),
            [b'>', ..] => (Gt, 1),
            [b'=', ..] => (Eq, 1),
            [b'?', ..] => (Question, 1),
            [b'(', ..] => (OpenParen, 1),
            [b')', ..] => (CloseParen, 1),
            [b'{', ..] => (OpenBrace, 1),
            [b'}', ..] => (CloseBrace, 1),
            [b'[', ..] => (OpenBracket, 1),
            [b']', ..] => (CloseBracket, 1),
            [b',', ..] => (Comma, 1),
            [b':', ..] => (Colon, 1),
            [b'.', ..] => (Dot, 1),
            [b';', ..] => (Semicolon, 1),
            [b'@', ..] => (At, 1),
            _ => {
                let c = text[start..].chars().next().unwrap_or('\0');
                self.pos += c.len_utf8().max(1);
                return self.fail(format!("unexpected character `{c}`"), start, self.pos);
            }
        };
        self.pos += len;
        match kind {
            OpenParen | OpenBracket => self.depth += 1,
            OpenBrace => {
                self.depth += 1;
                if let Some(hole) = self.holes.last_mut() {
                    hole.depth += 1;
                }
            }
            CloseParen | CloseBracket => self.depth = self.depth.saturating_sub(1),
            CloseBrace => {
                if self.holes.last().is_some_and(|hole| hole.depth == 0) {
                    return self.hole_close(start);
                }
                self.depth = self.depth.saturating_sub(1);
                if let Some(hole) = self.holes.last_mut() {
                    hole.depth -= 1;
                }
            }
            _ => {}
        }
        self.push(kind, start, self.pos);
    }
}

const INT_SUFFIXES: [(&str, IntSuffix); 8] = [
    ("i8", IntSuffix::I8),
    ("i16", IntSuffix::I16),
    ("i32", IntSuffix::I32),
    ("i64", IntSuffix::I64),
    ("u8", IntSuffix::U8),
    ("u16", IntSuffix::U16),
    ("u32", IntSuffix::U32),
    ("u64", IntSuffix::U64),
];

#[cfg(test)]
mod tests {
    use mote_base::{Interner, SourceMap, Symbol};

    use super::*;
    use TokenKind as K;

    struct Run {
        lexed: Lexed,
        sink: DiagnosticSink,
        interner: Interner,
    }

    fn run(text: &str) -> Run {
        let mut map = SourceMap::new();
        let id = map.add_file("test", text);
        let mut interner = Interner::new();
        let mut sink = DiagnosticSink::new();
        let lexed = lex(map.file(id), &mut interner, &mut sink);
        Run { lexed, sink, interner }
    }

    impl Run {
        fn all(&self) -> Vec<K> {
            self.lexed.tokens.iter().map(|t| t.kind).collect()
        }

        fn kinds(&self) -> Vec<K> {
            let mut kinds = self.all();
            kinds.pop();
            if kinds.last() == Some(&K::Newline) {
                kinds.pop();
            }
            kinds
        }

        fn sym(&mut self, name: &str) -> Symbol {
            self.interner.intern(name)
        }

        fn errors(&self) -> Vec<&str> {
            self.sink.iter().map(|d| d.message.as_str()).collect()
        }
    }

    fn int(value: u128) -> K {
        int_as(value, Radix::Decimal, None)
    }

    fn int_as(value: u128, radix: Radix, suffix: Option<IntSuffix>) -> K {
        K::Int(IntLiteral { value, radix, suffix })
    }

    fn float(value: f64, suffix: Option<FloatSuffix>) -> K {
        K::Float(FloatLiteral { value, suffix })
    }

    #[test]
    fn keywords_are_not_identifiers() {
        let r = run("let var fn struct class enum trait type pub import from as native scope spawn with return if else match while for in break continue yield self Self true false null super is");
        let expected = [
            K::Let, K::Var, K::Fn, K::Struct, K::Class, K::Enum, K::Trait, K::Type, K::Pub, K::Import, K::From, K::As, K::Native,
            K::Scope, K::Spawn, K::With, K::Return, K::If, K::Else, K::Match, K::While, K::For, K::In, K::Break, K::Continue,
            K::Yield, K::SelfLower, K::SelfUpper, K::True, K::False, K::Null, K::Super, K::Is,
        ];
        assert_eq!(r.kinds(), expected);
    }

    #[test]
    fn the_keyword_table_is_sorted_and_holds_the_words_above() {
        let words: Vec<&str> = crate::KEYWORDS.iter().map(|(word, _)| *word).collect();
        assert!(words.windows(2).all(|pair| pair[0] < pair[1]), "{words:?}");
        let mut listed: Vec<&str> = "let var fn struct class enum trait type pub import from as native scope spawn with return if else match while for in break continue yield self Self true false null super is".split(' ').collect();
        listed.sort_unstable();
        assert_eq!(words, listed);
    }

    #[test]
    fn retired_words_are_identifiers() {
        let mut r = run("impl mut try");
        let expected = [K::Ident(r.sym("impl")), K::Ident(r.sym("mut")), K::Ident(r.sym("try"))];
        assert_eq!(r.kinds(), expected);
    }

    #[test]
    fn identifiers_keep_their_text() {
        let mut r = run("foo _x x1 Foo_Bar");
        let expected = [K::Ident(r.sym("foo")), K::Ident(r.sym("_x")), K::Ident(r.sym("x1")), K::Ident(r.sym("Foo_Bar"))];
        assert_eq!(r.kinds(), expected);
    }

    #[test]
    fn integers_carry_value_radix_and_suffix() {
        let r = run("255 0xff 0b101 0o17 1_000 7u8 0xffu8 1_u16 3i64 1i8 1i16 1i32 1u32 1u64");
        let expected = [
            int(255),
            int_as(255, Radix::Hexadecimal, None),
            int_as(5, Radix::Binary, None),
            int_as(15, Radix::Octal, None),
            int(1000),
            int_as(7, Radix::Decimal, Some(IntSuffix::U8)),
            int_as(255, Radix::Hexadecimal, Some(IntSuffix::U8)),
            int_as(1, Radix::Decimal, Some(IntSuffix::U16)),
            int_as(3, Radix::Decimal, Some(IntSuffix::I64)),
            int_as(1, Radix::Decimal, Some(IntSuffix::I8)),
            int_as(1, Radix::Decimal, Some(IntSuffix::I16)),
            int_as(1, Radix::Decimal, Some(IntSuffix::I32)),
            int_as(1, Radix::Decimal, Some(IntSuffix::U32)),
            int_as(1, Radix::Decimal, Some(IntSuffix::U64)),
        ];
        assert_eq!(r.kinds(), expected);
        assert!(r.errors().is_empty());
    }

    #[test]
    fn the_largest_integer_fits_and_one_more_does_not() {
        let r = run("340282366920938463463374607431768211455");
        assert_eq!(r.kinds(), [int(u128::MAX)]);
        let r = run("340282366920938463463374607431768211456");
        assert_eq!(r.kinds(), [K::Error]);
        assert_eq!(r.errors(), ["the number is too large"]);
    }

    #[test]
    fn malformed_numbers_are_one_error_token() {
        for (text, message) in [
            ("0x", "the number has no digits"),
            ("0b12", "the number is malformed"),
            ("1abc", "the number is malformed"),
            ("0XFF", "the number is malformed"),
            ("1e999", "the number is too large"),
        ] {
            let r = run(text);
            assert_eq!(r.kinds(), [K::Error], "{text}");
            assert_eq!(r.errors(), [message], "{text}");
        }
    }

    #[test]
    fn floats_carry_value_and_suffix() {
        let r = run("1.5 2.5e-3 1e10 1_0.5_0 1.5f32 2e3f64");
        let expected = [
            float(1.5, None),
            float(2.5e-3, None),
            float(1e10, None),
            float(10.5, None),
            float(1.5, Some(FloatSuffix::F32)),
            float(2000.0, Some(FloatSuffix::F64)),
        ];
        assert_eq!(r.kinds(), expected);
    }

    #[test]
    fn a_dot_after_digits_is_not_always_a_float() {
        let mut r = run("1..2 1.max(2)");
        let expected = [
            int(1),
            K::DotDot,
            int(2),
            int(1),
            K::Dot,
            K::Ident(r.sym("max")),
            K::OpenParen,
            int(2),
            K::CloseParen,
        ];
        assert_eq!(r.kinds(), expected);
    }

    #[test]
    fn tuple_indexes_after_a_dot_are_plain_integers() {
        let mut r = run("t.0.1");
        let expected = [K::Ident(r.sym("t")), K::Dot, int(0), K::Dot, int(1)];
        assert_eq!(r.kinds(), expected);
    }

    #[test]
    fn strings_decode_escapes() {
        let mut r = run(r#""a\tb\n\\ \" \$ \0 \r \' \u{1F600} é""#);
        let expected = [K::String(r.sym("a\tb\n\\ \" $ \0 \r ' \u{1F600} é"))];
        assert_eq!(r.kinds(), expected);
        assert!(r.errors().is_empty());
    }

    #[test]
    fn a_lone_dollar_is_text() {
        let mut r = run(r#""$5 \${x}""#);
        let expected = [K::String(r.sym("$5 ${x}"))];
        assert_eq!(r.kinds(), expected);
    }

    #[test]
    fn strings_may_span_lines() {
        let mut r = run("\"a\nb\"");
        let expected = [K::String(r.sym("a\nb"))];
        assert_eq!(r.kinds(), expected);
    }

    #[test]
    fn bad_escapes_are_reported_and_dropped() {
        let mut r = run(r#""a\qb""#);
        let expected = [K::String(r.sym("ab"))];
        assert_eq!(r.kinds(), expected);
        assert_eq!(r.errors(), ["unknown escape `\\q`"]);
        let r = run(r#""\x41""#);
        assert_eq!(r.errors(), ["unknown escape `\\x`"]);
        let r = run(r#""\u{110000}""#);
        assert_eq!(r.errors(), ["not a unicode character"]);
        let r = run(r#""\u41""#);
        assert_eq!(r.errors(), ["a unicode escape is written `\\u{…}`"]);
    }

    #[test]
    fn an_unterminated_string_is_one_error() {
        let r = run("\"abc");
        assert_eq!(r.kinds(), [K::Error]);
        assert_eq!(r.errors(), ["the string is never closed"]);
    }

    #[test]
    fn interpolation_splits_a_string_around_its_holes() {
        let mut r = run(r#""a ${x} b ${y + 1} c""#);
        let expected = [
            K::InterpStart(r.sym("a ")),
            K::Ident(r.sym("x")),
            K::InterpMid(r.sym(" b ")),
            K::Ident(r.sym("y")),
            K::Plus,
            int(1),
            K::InterpEnd(r.sym(" c")),
        ];
        assert_eq!(r.kinds(), expected);
        assert!(r.errors().is_empty());
    }

    #[test]
    fn a_hole_may_hold_a_string() {
        let mut r = run(r#""${f("b")}""#);
        let expected = [
            K::InterpStart(r.sym("")),
            K::Ident(r.sym("f")),
            K::OpenParen,
            K::String(r.sym("b")),
            K::CloseParen,
            K::InterpEnd(r.sym("")),
        ];
        assert_eq!(r.kinds(), expected);
        assert!(r.errors().is_empty());
    }

    #[test]
    fn braces_inside_a_hole_do_not_end_it() {
        let mut r = run(r#""${ {1: 2}.len() }""#);
        let expected = [
            K::InterpStart(r.sym("")),
            K::OpenBrace,
            int(1),
            K::Colon,
            int(2),
            K::CloseBrace,
            K::Dot,
            K::Ident(r.sym("len")),
            K::OpenParen,
            K::CloseParen,
            K::InterpEnd(r.sym("")),
        ];
        assert_eq!(r.kinds(), expected);
    }

    #[test]
    fn an_empty_hole_is_an_error() {
        let r = run(r#""a${}b""#);
        assert_eq!(r.errors(), ["the `${}` is empty"]);
    }

    #[test]
    fn an_unclosed_hole_is_an_error() {
        let r = run(r#""a ${x"#);
        assert_eq!(r.errors(), ["the `${` is never closed"]);
    }

    #[test]
    fn a_prefix_does_not_make_a_raw_string() {
        let mut r = run(r#"r"a\nb""#);
        let expected = [K::Ident(r.sym("r")), K::String(r.sym("a\nb"))];
        assert_eq!(r.kinds(), expected);
    }

    #[test]
    fn a_single_quote_is_not_a_token() {
        let mut r = run("'a'");
        let expected = [K::Error, K::Ident(r.sym("a")), K::Error];
        assert_eq!(r.kinds(), expected);
        assert_eq!(r.errors(), ["unexpected character `'`", "unexpected character `'`"]);
    }

    #[test]
    fn comments_are_kept_outside_the_tokens() {
        let mut r = run("// a\n/// doc\nx /// trailing\n//// four\n/* a /* b */ c */ y");
        let expected = [K::Ident(r.sym("x")), K::Newline, K::Ident(r.sym("y"))];
        assert_eq!(r.kinds(), expected);
        let comments: Vec<_> = r.lexed.comments.iter().map(|c| c.kind).collect();
        use CommentKind::*;
        assert_eq!(comments, [Line, Doc, Line, Line, Block]);
        assert_eq!(r.lexed.comments[0].span, Span::new(0, 4));
        assert_eq!(r.lexed.comments[1].span, Span::new(5, 12));
    }

    #[test]
    fn an_unterminated_block_comment_is_an_error() {
        let r = run("x /* open");
        assert_eq!(r.errors(), ["the comment is never closed"]);
        assert_eq!(r.lexed.comments.len(), 1);
    }

    #[test]
    fn a_newline_ends_a_statement_after_a_closing_token() {
        let mut r = run("a\nb");
        let expected = [K::Ident(r.sym("a")), K::Newline, K::Ident(r.sym("b")), K::Newline, K::Eof];
        assert_eq!(r.all(), expected);
    }

    #[test]
    fn a_newline_ends_a_statement_after_each_listed_token_and_after_no_other() {
        for ends in ["a", "1", "1.5", "\"s\"", "\"${a}\"", "true", "false", "null", "return", "break", "continue", "yield", ")", "]", "}", "?", "!"] {
            let r = run(&format!("{ends}\n+"));
            assert!(r.all().contains(&K::Newline), "{ends}");
        }
        for continues in ["+", "(", ".", "=", "->", ",", "let", "if", "fn"] {
            let r = run(&format!("{continues}\n+"));
            assert!(!r.all().contains(&K::Newline), "{continues}");
        }
    }

    #[test]
    fn newlines_inside_brackets_are_ignored() {
        let mut r = run("(a\nb)");
        let expected = [K::OpenParen, K::Ident(r.sym("a")), K::Ident(r.sym("b")), K::CloseParen, K::Newline, K::Eof];
        assert_eq!(r.all(), expected);
    }

    #[test]
    fn a_trailing_operator_continues_the_line() {
        let mut r = run("x +\ny");
        let expected = [K::Ident(r.sym("x")), K::Plus, K::Ident(r.sym("y")), K::Newline, K::Eof];
        assert_eq!(r.all(), expected);
    }

    #[test]
    fn blank_lines_make_one_newline() {
        let mut r = run("a\n\n\nb");
        let expected = [K::Ident(r.sym("a")), K::Newline, K::Ident(r.sym("b")), K::Newline, K::Eof];
        assert_eq!(r.all(), expected);
    }

    #[test]
    fn a_block_has_no_newlines_inside_and_one_after() {
        let mut r = run("f {\n a\n}\n");
        let expected = [K::Ident(r.sym("f")), K::OpenBrace, K::Ident(r.sym("a")), K::CloseBrace, K::Newline, K::Eof];
        assert_eq!(r.all(), expected);
    }

    #[test]
    fn carriage_returns_are_whitespace() {
        let mut r = run("a\r\nb");
        let expected = [K::Ident(r.sym("a")), K::Newline, K::Ident(r.sym("b")), K::Newline, K::Eof];
        assert_eq!(r.all(), expected);
    }

    #[test]
    fn a_line_starting_with_a_member_access_continues_the_previous_one() {
        let mut r = run("a\n  .b\n  // note\n  ?.c\nd\n..e");
        let expected = [
            K::Ident(r.sym("a")),
            K::Dot,
            K::Ident(r.sym("b")),
            K::QuestionDot,
            K::Ident(r.sym("c")),
            K::Newline,
            K::Ident(r.sym("d")),
            K::Newline,
            K::DotDot,
            K::Ident(r.sym("e")),
            K::Newline,
            K::Eof,
        ];
        assert_eq!(r.all(), expected);
    }

    #[test]
    fn a_comment_does_not_hide_the_newline() {
        let mut r = run("a // c\nb");
        let expected = [K::Ident(r.sym("a")), K::Newline, K::Ident(r.sym("b")), K::Newline, K::Eof];
        assert_eq!(r.all(), expected);
    }

    #[test]
    fn a_propagating_question_ends_a_statement() {
        let mut r = run("a?\nb");
        let expected = [K::Ident(r.sym("a")), K::Question, K::Newline, K::Ident(r.sym("b")), K::Newline, K::Eof];
        assert_eq!(r.all(), expected);
    }

    #[test]
    fn empty_and_blank_input_is_just_eof() {
        assert_eq!(run("").all(), [K::Eof]);
        assert_eq!(run("\n\n").all(), [K::Eof]);
    }

    #[test]
    fn operators_take_the_longest_match() {
        let r = run(">>= ?? ??= ..= ... .. ?. -> => :: <<= != <= >= == && || << >> += -= *= /= %= &= |= ^=");
        let expected = [
            K::ShrEq, K::QuestionQuestion, K::NullCoalesceEq, K::DotDotEq, K::DotDotDot, K::DotDot, K::QuestionDot, K::Arrow,
            K::FatArrow, K::ColonColon, K::ShlEq, K::NotEq, K::LtEq, K::GtEq, K::EqEq, K::AndAnd, K::OrOr, K::Shl, K::Shr,
            K::PlusEq, K::MinusEq, K::StarEq, K::SlashEq, K::PercentEq, K::AmpEq, K::PipeEq, K::CaretEq,
        ];
        assert_eq!(r.kinds(), expected);
    }

    #[test]
    fn single_character_punctuation() {
        let r = run("( ) { } [ ] , : . ; @ + - * / % ^ & | ~ ! < > = ?");
        let expected = [
            K::OpenParen, K::CloseParen, K::OpenBrace, K::CloseBrace, K::OpenBracket, K::CloseBracket, K::Comma, K::Colon,
            K::Dot, K::Semicolon, K::At, K::Plus, K::Minus, K::Star, K::Slash, K::Percent, K::Caret, K::Ampersand, K::Pipe,
            K::Tilde, K::Bang, K::Lt, K::Gt, K::Eq, K::Question,
        ];
        assert_eq!(r.kinds(), expected);
    }

    #[test]
    fn an_unexpected_character_is_an_error_token_and_lexing_goes_on() {
        let mut r = run("a # b é");
        let expected = [K::Ident(r.sym("a")), K::Error, K::Ident(r.sym("b")), K::Error];
        assert_eq!(r.kinds(), expected);
        assert_eq!(r.errors(), ["unexpected character `#`", "unexpected character `é`"]);
    }

    #[test]
    fn spans_use_the_files_offset_range() {
        let mut map = SourceMap::new();
        map.add_file("a", "xx");
        let id = map.add_file("b", "ab cd");
        let mut interner = Interner::new();
        let mut sink = DiagnosticSink::new();
        let lexed = lex(map.file(id), &mut interner, &mut sink);
        let spans: Vec<_> = lexed.tokens.iter().map(|t| t.span).collect();
        assert_eq!(spans, [Span::new(3, 5), Span::new(6, 8), Span::new(8, 8), Span::new(8, 8)]);
    }
}
