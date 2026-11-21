use mote_base::Id;

use super::Parser;
use crate::ast::{FieldPattern, LiteralPattern, Pattern, PatternKind};
use crate::token::TokenKind as T;

impl Parser<'_> {
    pub(super) fn pattern(&mut self) -> Id<Pattern> {
        self.pattern_with(true)
    }

    pub(super) fn binding_pattern(&mut self) -> Id<Pattern> {
        self.pattern_with(false)
    }

    fn pattern_node(&mut self, kind: PatternKind, start: u32) -> Id<Pattern> {
        let span = self.span_from(start);
        self.ast.patterns.alloc(Pattern { kind, span })
    }

    fn pattern_with(&mut self, annotated: bool) -> Id<Pattern> {
        if !self.enter() {
            self.leave();
            return self.pattern_node(PatternKind::Error, self.start());
        }
        let start = self.start();
        let first = self.range_pattern(annotated);
        let pattern = if self.at(T::Pipe) {
            let mut alternatives = vec![first];
            while self.eat(T::Pipe) {
                alternatives.push(self.range_pattern(annotated));
            }
            self.pattern_node(PatternKind::Or(alternatives), start)
        } else {
            first
        };
        self.leave();
        pattern
    }

    fn range_pattern(&mut self, annotated: bool) -> Id<Pattern> {
        let start = self.start();
        let first = self.primary_pattern(annotated);
        if matches!(self.peek(), T::DotDot | T::DotDotEq) {
            let inclusive = self.bump().kind == T::DotDotEq;
            let end = self.primary_pattern(false);
            return self.pattern_node(PatternKind::Range { start: first, end, inclusive }, start);
        }
        first
    }

    fn primary_pattern(&mut self, annotated: bool) -> Id<Pattern> {
        let start = self.start();
        let kind = match self.peek() {
            T::Ident(name) if name == self.words.wildcard => {
                self.bump();
                if annotated && self.eat(T::Colon) {
                    let ty = self.ty();
                    PatternKind::Type { name: None, ty }
                } else {
                    PatternKind::Wildcard
                }
            }
            T::Ident(_) if self.peek_at(1) == T::At => {
                let name = self.name();
                self.bump();
                let pattern = self.range_pattern(false);
                PatternKind::At { name, pattern }
            }
            T::Ident(_) if annotated && self.peek_at(1) == T::Colon => {
                let name = self.name();
                self.bump();
                let ty = self.ty();
                PatternKind::Type { name: Some(name), ty }
            }
            T::Ident(_) | T::SelfUpper => self.path_pattern(),
            T::Int(literal) => {
                self.bump();
                PatternKind::Literal(LiteralPattern::Int { negative: false, literal })
            }
            T::Minus => {
                self.bump();
                if let T::Int(literal) = self.peek() {
                    self.bump();
                    PatternKind::Literal(LiteralPattern::Int { negative: true, literal })
                } else {
                    self.unexpected("a number");
                    PatternKind::Error
                }
            }
            T::String(text) => {
                self.bump();
                PatternKind::Literal(LiteralPattern::String(text))
            }
            T::True | T::False => PatternKind::Literal(LiteralPattern::Bool(self.bump().kind == T::True)),
            T::Null => {
                self.bump();
                PatternKind::Literal(LiteralPattern::Null)
            }
            T::OpenParen => return self.tuple_pattern(start),
            T::Error => {
                self.bump();
                self.failures += 1;
                PatternKind::Error
            }
            _ => {
                self.unexpected("a pattern");
                PatternKind::Error
            }
        };
        self.pattern_node(kind, start)
    }

    fn path_pattern(&mut self) -> PatternKind {
        let path = self.path();
        match self.peek() {
            T::OpenParen => {
                self.bump();
                let elements = self.list(T::CloseParen, |p| p.pattern());
                PatternKind::TupleStruct { path, elements }
            }
            T::OpenBrace => {
                self.bump();
                let (fields, rest) = self.struct_pattern_fields();
                PatternKind::Struct { path, fields, rest }
            }
            _ if path.segments.len() == 1 => PatternKind::Binding(path.segments[0]),
            _ => PatternKind::Path(path),
        }
    }

    fn struct_pattern_fields(&mut self) -> (Vec<FieldPattern>, bool) {
        self.free(|p| {
            let mut fields = Vec::new();
            let mut rest = false;
            while !p.at(T::CloseBrace) && !p.at(T::Eof) {
                if p.eat(T::DotDot) {
                    rest = true;
                    break;
                }
                let (failures, pos) = (p.failures, p.pos);
                let name = p.name();
                let pattern = if p.eat(T::Colon) { Some(p.pattern_with(false)) } else { None };
                fields.push(FieldPattern { name, pattern });
                if p.failures != failures || p.pos == pos {
                    p.skip_in_list(T::CloseBrace);
                    if p.pos == pos {
                        p.bump();
                    }
                }
                p.eat(T::Comma);
            }
            p.expect(T::CloseBrace);
            (fields, rest)
        })
    }

    fn tuple_pattern(&mut self, start: u32) -> Id<Pattern> {
        self.bump();
        let (mut elements, trailing) = self.list_flagged(T::CloseParen, |p| p.pattern());
        if elements.len() == 1 && !trailing {
            return elements.remove(0);
        }
        self.pattern_node(PatternKind::Tuple(elements), start)
    }
}
