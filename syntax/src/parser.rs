mod expr;
mod item;
mod pattern;
mod stmt;
mod ty;

#[cfg(test)]
mod tests;

use mote_base::{Diagnostic, DiagnosticSink, Id, Interner, SourceFile, Span, Symbol};

use crate::ast::{Ast, Expr, Ident, Type};
use crate::lexer::{Comment, CommentKind, Lexed};
use crate::token::{Token, TokenKind};
use TokenKind as T;

const MAX_DEPTH: u32 = 128;

/// Parses a file into a tree, reporting errors to the sink.
pub fn parse(file: &SourceFile, lexed: &Lexed, interner: &mut Interner, sink: &mut DiagnosticSink) -> Ast {
    let mut ast = Ast::default();
    let mut parser = Parser::new(&mut ast, file, lexed, interner, sink);
    parser.program();
    parser.finish();
    ast
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Head {
    Free,
    Condition,
    Subject,
}

#[derive(Clone, Copy)]
struct Ctx {
    head: Head,
    no_range: bool,
    no_union: bool,
}

const FREE: Ctx = Ctx { head: Head::Free, no_range: false, no_union: false };

struct Words {
    test: Symbol,
    send: Symbol,
    wildcard: Symbol,
    from: Symbol,
    self_upper: Symbol,
}

struct Parser<'a> {
    ast: &'a mut Ast,
    file: &'a SourceFile,
    tokens: &'a [Token],
    comments: &'a [Comment],
    sink: &'a mut DiagnosticSink,
    words: Words,
    pos: usize,
    half: bool,
    ctx: Ctx,
    depth: u32,
    failures: u32,
    last_error: Option<u32>,
    doc_used: Vec<bool>,
}

impl<'a> Parser<'a> {
    fn new(
        ast: &'a mut Ast,
        file: &'a SourceFile,
        lexed: &'a Lexed,
        interner: &mut Interner,
        sink: &'a mut DiagnosticSink,
    ) -> Self {
        let words = Words {
            test: interner.intern("test"),
            send: interner.intern("Send"),
            wildcard: interner.intern("_"),
            from: interner.intern("from"),
            self_upper: interner.intern("Self"),
        };
        Parser {
            ast,
            file,
            tokens: &lexed.tokens,
            comments: &lexed.comments,
            sink,
            words,
            pos: 0,
            half: false,
            ctx: FREE,
            depth: 0,
            failures: 0,
            last_error: None,
            doc_used: vec![false; lexed.comments.len()],
        }
    }

    fn finish(self) {
        for (i, comment) in self.comments.iter().enumerate() {
            if comment.kind == CommentKind::Doc && !self.doc_used[i] {
                self.sink.emit(Diagnostic::warning("this doc comment documents nothing", comment.span));
            }
        }
    }

    fn peek(&self) -> TokenKind {
        if self.half { T::Gt } else { self.tokens[self.pos].kind }
    }

    fn peek_at(&self, n: usize) -> TokenKind {
        self.tokens.get(self.pos + n).map_or(T::Eof, |t| t.kind)
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.peek() == kind
    }

    fn span(&self) -> Span {
        let span = self.tokens[self.pos].span;
        if self.half { Span::new(span.start + 1, span.end) } else { span }
    }

    fn start(&self) -> u32 {
        self.span().start
    }

    fn prev_end(&self) -> u32 {
        if self.half {
            self.tokens[self.pos].span.start + 1
        } else if self.pos == 0 {
            0
        } else {
            self.tokens[self.pos - 1].span.end
        }
    }

    fn span_from(&self, start: u32) -> Span {
        Span::new(start, self.prev_end().max(start))
    }

    fn bump(&mut self) -> Token {
        let token = self.tokens[self.pos];
        if self.half {
            self.half = false;
            self.pos += 1;
            return Token { kind: T::Gt, span: Span::new(token.span.start + 1, token.span.end) };
        }
        if token.kind != T::Eof {
            self.pos += 1;
        }
        token
    }

    fn eat(&mut self, kind: TokenKind) -> bool {
        let found = self.at(kind);
        if found {
            self.bump();
        }
        found
    }

    fn expect(&mut self, kind: TokenKind) -> bool {
        if self.eat(kind) {
            return true;
        }
        self.unexpected(&describe(kind));
        false
    }

    fn close_angle(&mut self) -> bool {
        match self.peek() {
            T::Gt => {
                self.bump();
                true
            }
            T::Shr => {
                self.half = true;
                true
            }
            _ => {
                self.unexpected("`>`");
                false
            }
        }
    }

    fn error(&mut self, message: impl Into<String>, span: Span) {
        self.failures += 1;
        if self.last_error == Some(span.start) {
            return;
        }
        self.last_error = Some(span.start);
        self.sink.emit(Diagnostic::error(message, span));
    }

    fn unexpected(&mut self, expected: &str) {
        if self.peek() == T::Error {
            self.failures += 1;
            return;
        }
        let message = format!("expected {expected}, found {}", describe(self.peek()));
        let span = self.span();
        self.error(message, span);
    }

    fn name(&mut self) -> Ident {
        if let T::Ident(name) = self.peek() {
            let span = self.bump().span;
            return Ident { name, span };
        }
        self.unexpected("a name");
        Ident { name: self.words.wildcard, span: Span::new(self.start(), self.start()) }
    }

    fn member_name(&mut self) -> Ident {
        if self.at(T::From) {
            let span = self.bump().span;
            return Ident { name: self.words.from, span };
        }
        self.name()
    }

    fn with_ctx<R>(&mut self, ctx: Ctx, f: impl FnOnce(&mut Self) -> R) -> R {
        let saved = std::mem::replace(&mut self.ctx, ctx);
        let out = f(self);
        self.ctx = saved;
        out
    }

    fn free<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.with_ctx(FREE, f)
    }

    fn head<R>(&mut self, head: Head, f: impl FnOnce(&mut Self) -> R) -> R {
        self.with_ctx(Ctx { head, ..FREE }, f)
    }

    fn enter(&mut self) -> bool {
        self.depth += 1;
        if self.depth <= MAX_DEPTH {
            return true;
        }
        let span = self.span();
        self.error("this is nested too deeply", span);
        self.half = false;
        self.pos = self.tokens.len() - 1;
        self.last_error = Some(self.span().start);
        false
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn skip_separators(&mut self) {
        while matches!(self.peek(), T::Newline | T::Semicolon) {
            self.bump();
        }
    }

    fn skip_newlines(&mut self) {
        while self.at(T::Newline) {
            self.bump();
        }
    }

    fn error_expr(&mut self) -> Id<Expr> {
        let at = self.start();
        self.ast.exprs.alloc(Expr { kind: crate::ast::ExprKind::Error, span: Span::new(at, at) })
    }

    fn error_type(&mut self) -> Id<Type> {
        let at = self.start();
        self.ast.types.alloc(Type { kind: crate::ast::TypeKind::Error, span: Span::new(at, at) })
    }

    fn take_doc(&mut self, start: u32) -> Option<Span> {
        let mut i = self.comments.partition_point(|c| c.span.end <= start);
        let mut doc: Option<Span> = None;
        let mut boundary = start;
        while i > 0 {
            let comment = self.comments[i - 1];
            if comment.kind != CommentKind::Doc || !self.one_line_between(comment.span.end, boundary) {
                break;
            }
            self.doc_used[i - 1] = true;
            doc = Some(doc.map_or(comment.span, |d| comment.span.merge(d)));
            boundary = comment.span.start;
            i -= 1;
        }
        doc
    }

    /// Whether the token here starts a line.
    fn starts_line(&self) -> bool {
        let base = self.file.start;
        let gap = self.file.text.get((self.prev_end() - base) as usize..(self.span().start - base) as usize);
        gap.is_some_and(|gap| gap.contains('\n'))
    }

    fn one_line_between(&self, from: u32, to: u32) -> bool {
        let base = self.file.start;
        let Some(gap) = self.file.text.get((from - base) as usize..(to - base) as usize) else { return false };
        gap.bytes().all(|b| b.is_ascii_whitespace()) && gap.bytes().filter(|&b| b == b'\n').count() == 1
    }

    fn list<R>(&mut self, close: TokenKind, item: impl FnMut(&mut Self) -> R) -> Vec<R> {
        self.list_flagged(close, item).0
    }

    fn list_flagged<R>(&mut self, close: TokenKind, mut item: impl FnMut(&mut Self) -> R) -> (Vec<R>, bool) {
        self.free(|p| {
            let mut items = Vec::new();
            let mut trailing = false;
            while !p.at(close) && !p.at(T::Eof) {
                let (failures, pos) = (p.failures, p.pos);
                items.push(item(p));
                if p.failures != failures || p.pos == pos {
                    p.skip_in_list(close);
                    if p.pos == pos {
                        p.bump();
                    }
                }
                trailing = false;
                if !p.eat(T::Comma) {
                    break;
                }
                trailing = true;
            }
            p.expect(close);
            (items, trailing)
        })
    }

    fn items_until<R>(&mut self, close: TokenKind, mut item: impl FnMut(&mut Self) -> R) -> Vec<R> {
        let mut items = Vec::new();
        while !self.at(close) && !self.at(T::Eof) {
            let (failures, pos) = (self.failures, self.pos);
            items.push(item(self));
            if self.failures != failures || self.pos == pos {
                self.skip_in_list(close);
                if self.pos == pos {
                    self.bump();
                }
            }
            self.eat(T::Comma);
        }
        self.expect(close);
        items
    }

    fn skip_in_list(&mut self, close: TokenKind) {
        let mut depth = 0u32;
        loop {
            let kind = self.peek();
            match kind {
                T::Eof => return,
                T::Comma if depth == 0 => return,
                _ if depth == 0 && kind == close => return,
                T::OpenParen | T::OpenBracket | T::OpenBrace => depth += 1,
                T::CloseParen | T::CloseBracket | T::CloseBrace => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                }
                _ if depth == 0 && is_stopper(kind) => return,
                _ => {}
            }
            self.bump();
        }
    }

    fn recover_stmt(&mut self) {
        let mut depth = 0u32;
        loop {
            let kind = self.peek();
            match kind {
                T::Eof => return,
                T::Newline | T::Semicolon if depth == 0 => return,
                T::CloseBrace if depth == 0 => return,
                _ if depth == 0 && is_stmt_keyword(kind) => return,
                T::OpenParen | T::OpenBracket | T::OpenBrace => depth += 1,
                T::CloseParen | T::CloseBracket | T::CloseBrace => depth = depth.saturating_sub(1),
                _ => {}
            }
            self.bump();
        }
    }

    fn recover_item(&mut self, from: usize) {
        let mut depth = 0u32;
        loop {
            let kind = self.peek();
            match kind {
                T::Eof | T::Newline | T::Semicolon => return,
                _ if depth == 0 && self.pos > from && is_item_start(kind) => return,
                T::OpenParen | T::OpenBracket | T::OpenBrace => depth += 1,
                T::CloseParen | T::CloseBracket | T::CloseBrace => depth = depth.saturating_sub(1),
                _ => {}
            }
            self.bump();
        }
    }
}

fn is_stmt_keyword(kind: TokenKind) -> bool {
    matches!(
        kind,
        T::Let
            | T::Var
            | T::Fn
            | T::If
            | T::While
            | T::For
            | T::Match
            | T::Scope
            | T::With
            | T::Spawn
            | T::Return
            | T::Break
            | T::Continue
            | T::Yield
    )
}

fn is_item_start(kind: TokenKind) -> bool {
    matches!(
        kind,
        T::Fn | T::Native | T::Struct | T::Class | T::Enum | T::Trait | T::Type | T::Import | T::Pub | T::At | T::Let | T::Var
    )
}

fn is_stopper(kind: TokenKind) -> bool {
    matches!(
        kind,
        T::Let
            | T::Var
            | T::Fn
            | T::Struct
            | T::Class
            | T::Enum
            | T::Trait
            | T::Type
            | T::Import
            | T::Pub
            | T::Native
            | T::While
            | T::For
            | T::Return
            | T::Break
            | T::Continue
            | T::Yield
            | T::Scope
            | T::With
    )
}

fn starts_expr(kind: TokenKind) -> bool {
    matches!(
        kind,
        T::Ident(_)
            | T::Int(_)
            | T::Float(_)
            | T::String(_)
            | T::InterpStart(_)
            | T::True
            | T::False
            | T::Null
            | T::SelfLower
            | T::SelfUpper
            | T::OpenParen
            | T::OpenBracket
            | T::OpenBrace
            | T::Minus
            | T::Bang
            | T::Tilde
            | T::Pipe
            | T::OrOr
            | T::If
            | T::Match
            | T::Spawn
    )
}

fn describe(kind: TokenKind) -> String {
    let text = match kind {
        T::Ident(_) => return "a name".into(),
        T::Int(_) | T::Float(_) => return "a number".into(),
        T::String(_) | T::InterpStart(_) | T::InterpMid(_) | T::InterpEnd(_) => return "a string".into(),
        T::Newline => return "the end of the line".into(),
        T::Eof => return "the end of the file".into(),
        T::Error => return "an invalid token".into(),
        T::Let => "let",
        T::Var => "var",
        T::Fn => "fn",
        T::Struct => "struct",
        T::Class => "class",
        T::Enum => "enum",
        T::Trait => "trait",
        T::Type => "type",
        T::Pub => "pub",
        T::Import => "import",
        T::From => "from",
        T::As => "as",
        T::Native => "native",
        T::Scope => "scope",
        T::Spawn => "spawn",
        T::With => "with",
        T::Return => "return",
        T::If => "if",
        T::Else => "else",
        T::Match => "match",
        T::While => "while",
        T::For => "for",
        T::In => "in",
        T::Break => "break",
        T::Continue => "continue",
        T::Yield => "yield",
        T::SelfLower => "self",
        T::SelfUpper => "Self",
        T::True => "true",
        T::False => "false",
        T::Null => "null",
        T::Super => "super",
        T::Is => "is",
        T::Plus => "+",
        T::Minus => "-",
        T::Star => "*",
        T::Slash => "/",
        T::Percent => "%",
        T::Caret => "^",
        T::Ampersand => "&",
        T::Pipe => "|",
        T::Tilde => "~",
        T::Bang => "!",
        T::EqEq => "==",
        T::NotEq => "!=",
        T::Lt => "<",
        T::LtEq => "<=",
        T::Gt => ">",
        T::GtEq => ">=",
        T::AndAnd => "&&",
        T::OrOr => "||",
        T::Shl => "<<",
        T::Shr => ">>",
        T::Eq => "=",
        T::PlusEq => "+=",
        T::MinusEq => "-=",
        T::StarEq => "*=",
        T::SlashEq => "/=",
        T::PercentEq => "%=",
        T::AmpEq => "&=",
        T::PipeEq => "|=",
        T::CaretEq => "^=",
        T::ShlEq => "<<=",
        T::ShrEq => ">>=",
        T::NullCoalesceEq => "??=",
        T::Arrow => "->",
        T::FatArrow => "=>",
        T::ColonColon => "::",
        T::DotDotEq => "..=",
        T::DotDotDot => "...",
        T::DotDot => "..",
        T::Question => "?",
        T::QuestionDot => "?.",
        T::QuestionQuestion => "??",
        T::OpenParen => "(",
        T::CloseParen => ")",
        T::OpenBrace => "{",
        T::CloseBrace => "}",
        T::OpenBracket => "[",
        T::CloseBracket => "]",
        T::Comma => ",",
        T::Colon => ":",
        T::Dot => ".",
        T::Semicolon => ";",
        T::At => "@",
    };
    format!("`{text}`")
}
