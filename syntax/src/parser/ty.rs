use mote_base::Id;

use super::{Ctx, Parser, FREE};
use crate::ast::{FnTypeParam, GenericArg, Path, Type, TypeKind};
use crate::token::TokenKind as T;

impl Parser<'_> {
    pub(super) fn path(&mut self) -> Path {
        let start = self.start();
        let mut segments = vec![self.path_head()];
        while self.at(T::Dot) && matches!(self.peek_at(1), T::Ident(_)) {
            self.bump();
            segments.push(self.name());
        }
        Path { segments, span: self.span_from(start) }
    }

    fn path_head(&mut self) -> crate::ast::Ident {
        if self.at(T::SelfUpper) {
            let span = self.bump().span;
            return crate::ast::Ident { name: self.words.self_upper, span };
        }
        self.name()
    }

    fn type_node(&mut self, kind: TypeKind, start: u32) -> Id<Type> {
        let span = self.span_from(start);
        self.ast.types.alloc(Type { kind, span })
    }

    pub(super) fn ty(&mut self) -> Id<Type> {
        if !self.enter() {
            self.leave();
            return self.error_type();
        }
        let start = self.start();
        let first = self.nullable_ty();
        let ty = if self.at(T::Pipe) && !self.ctx.no_union {
            let mut members = vec![first];
            while self.eat(T::Pipe) {
                members.push(self.nullable_ty());
            }
            self.type_node(TypeKind::Union(members), start)
        } else {
            first
        };
        self.leave();
        ty
    }

    fn nullable_ty(&mut self) -> Id<Type> {
        let start = self.start();
        let mut ty = self.primary_ty();
        loop {
            let wraps = match self.peek() {
                T::Question => 1,
                T::QuestionQuestion => 2,
                _ => break,
            };
            self.bump();
            for _ in 0..wraps {
                ty = self.type_node(TypeKind::Nullable(ty), start);
            }
        }
        ty
    }

    fn primary_ty(&mut self) -> Id<Type> {
        let start = self.start();
        match self.peek() {
            T::Ident(name) => {
                if name == self.words.send && self.peek_at(1) == T::OpenParen {
                    self.bump();
                    return self.paren_ty(start, true);
                }
                let path = self.path();
                if self.at(T::Lt) {
                    let args = self.generic_args();
                    return self.type_node(TypeKind::Generic { path, args }, start);
                }
                self.type_node(TypeKind::Named(path), start)
            }
            T::At => {
                self.bump();
                let name = self.name();
                let generic_args = if self.at(T::Lt) { self.generic_args() } else { Vec::new() };
                let args = if self.expect(T::OpenParen) { self.list(T::CloseParen, |p| p.arg()) } else { Vec::new() };
                self.type_node(TypeKind::Builtin { name, generic_args, args }, start)
            }
            T::SelfUpper => {
                self.bump();
                self.type_node(TypeKind::SelfType, start)
            }
            T::OpenParen => self.paren_ty(start, false),
            T::OpenBracket => {
                self.bump();
                let (element, len) = self.free(|p| {
                    let element = p.ty();
                    let len = if p.eat(T::Semicolon) { Some(p.expr()) } else { None };
                    (element, len)
                });
                self.expect(T::CloseBracket);
                self.type_node(TypeKind::Array { element, len }, start)
            }
            T::Error => {
                self.bump();
                self.failures += 1;
                self.type_node(TypeKind::Error, start)
            }
            _ => {
                self.unexpected("a type");
                self.error_type()
            }
        }
    }

    pub(super) fn generic_args(&mut self) -> Vec<GenericArg> {
        self.bump();
        let args = self.free(|p| {
            let mut args = vec![p.generic_arg()];
            while p.eat(T::Comma) && !matches!(p.peek(), T::Gt | T::Shr) {
                args.push(p.generic_arg());
            }
            args
        });
        self.close_angle();
        args
    }

    fn generic_arg(&mut self) -> GenericArg {
        if self.looks_like_expr_arg() {
            let ctx = Ctx { no_gt: true, ..FREE };
            GenericArg::Expr(self.with_ctx(ctx, |p| p.expr()))
        } else {
            GenericArg::Type(self.ty())
        }
    }

    fn looks_like_expr_arg(&self) -> bool {
        match self.peek() {
            T::Int(_)
            | T::Float(_)
            | T::String(_)
            | T::InterpStart(_)
            | T::True
            | T::False
            | T::Null
            | T::Minus
            | T::Bang
            | T::Tilde
            | T::Comptime
            | T::OpenBrace => true,
            T::At => self.at_builtin_looks_like_expr(),
            T::OpenBracket => self.bracket_arg_looks_like_expr(),
            T::OpenParen => self.paren_arg_looks_like_expr(),
            T::Ident(name) if name == self.words.send && self.peek_at(1) == T::OpenParen => false,
            T::Ident(_) | T::SelfUpper => self.path_arg_looks_like_expr(),
            _ => false,
        }
    }

    fn at_builtin_looks_like_expr(&self) -> bool {
        let mut i = 1;
        let is_type_builtin = match self.peek_at(i) {
            T::Ident(sym) => {
                sym == self.words.type_of
                    || sym == self.words.type_arg
                    || sym == self.words.field_type
                    || sym == self.words.variant_field_type
            }
            _ => return true,
        };
        i += 1;
        if self.peek_at(i) == T::Lt && self.type_args_follow(i) {
            let (next, closed_outer) = self.skip_balanced_angles(i);
            if closed_outer {
                return false;
            }
            i = next;
        }
        if self.peek_at(i) == T::OpenParen {
            i = self.skip_balanced_group(i);
        }
        if Self::is_expr_arg_binop(self.peek_at(i)) {
            return true;
        }
        !is_type_builtin && !matches!(self.peek_at(i), T::Question | T::QuestionQuestion | T::Pipe)
    }

    fn bracket_arg_looks_like_expr(&self) -> bool {
        if matches!(
            self.peek_at(1),
            T::Int(_) | T::Float(_) | T::String(_) | T::InterpStart(_) | T::True | T::False | T::Null | T::Minus | T::Bang | T::Tilde | T::Comptime
        ) {
            return true;
        }
        let mut depth = 0i32;
        let mut i = 0;
        loop {
            match self.peek_at(i) {
                T::OpenBracket | T::OpenParen | T::OpenBrace => depth += 1,
                T::CloseBracket | T::CloseParen | T::CloseBrace => {
                    depth -= 1;
                    if depth <= 0 {
                        i += 1;
                        return Self::is_expr_arg_binop(self.peek_at(i));
                    }
                }
                T::Comma if depth == 1 => return true,
                T::Semicolon if depth == 1 => return false,
                T::Eof => return false,
                _ => {}
            }
            i += 1;
        }
    }

    fn paren_arg_looks_like_expr(&self) -> bool {
        let mut depth = 0i32;
        let mut i = 0;
        let mut has_expr_token = false;
        loop {
            let tok = self.peek_at(i);
            if tok == T::Lt && depth > 0 && i > 0 && matches!(self.peek_at(i - 1), T::Ident(_) | T::SelfUpper)
                && let Some(next) = self.try_skip_type_angles(i)
            {
                i = next;
                continue;
            }
            match tok {
                T::OpenParen | T::OpenBracket | T::OpenBrace => depth += 1,
                T::CloseParen | T::CloseBracket | T::CloseBrace => {
                    depth -= 1;
                    if depth <= 0 {
                        i += 1;
                        if self.peek_at(i) == T::Arrow {
                            return false;
                        }
                        return has_expr_token || Self::is_expr_arg_binop(self.peek_at(i));
                    }
                }
                T::Int(_)
                | T::Float(_)
                | T::String(_)
                | T::InterpStart(_)
                | T::True
                | T::False
                | T::Null
                | T::Minus
                | T::Bang
                | T::Tilde
                | T::Comptime
                | T::Plus
                | T::Star
                | T::Slash
                | T::Percent
                | T::Caret
                | T::Shl
                | T::Shr
                | T::EqEq
                | T::NotEq
                | T::LtEq
                | T::Gt
                | T::GtEq
                | T::AndAnd
                | T::OrOr
                | T::DotDot
                | T::DotDotEq
                | T::QuestionQuestion => has_expr_token = true,
                T::Eof => return false,
                _ => {}
            }
            i += 1;
        }
    }

    fn try_skip_type_angles(&self, mut i: usize) -> Option<usize> {
        let mut angle = 0i32;
        let mut group = 0i32;
        loop {
            match self.peek_at(i) {
                T::OpenParen | T::OpenBracket | T::OpenBrace => group += 1,
                T::CloseParen | T::CloseBracket | T::CloseBrace => {
                    if group == 0 {
                        return None;
                    }
                    group -= 1;
                }
                T::Lt if group == 0 => angle += 1,
                T::Gt if group == 0 => {
                    angle -= 1;
                    if angle <= 0 {
                        let next = self.peek_at(i + 1);
                        return matches!(next, T::Comma | T::CloseParen | T::Question | T::Pipe | T::Arrow | T::Dot)
                            .then_some(i + 1);
                    }
                }
                T::Shr if group == 0 => {
                    angle -= 2;
                    if angle <= 0 {
                        let next = self.peek_at(i + 1);
                        return matches!(next, T::Comma | T::CloseParen | T::Question | T::Pipe | T::Arrow | T::Dot)
                            .then_some(i + 1);
                    }
                }
                T::AndAnd | T::OrOr | T::EqEq | T::NotEq | T::LtEq | T::GtEq if group == 0 => return None,
                T::Eof => return None,
                _ => {}
            }
            i += 1;
        }
    }

    fn path_arg_looks_like_expr(&self) -> bool {
        let mut i = 1;
        while self.peek_at(i) == T::Dot && matches!(self.peek_at(i + 1), T::Ident(_)) {
            i += 2;
        }
        if self.peek_at(i) == T::Lt {
            let (next, closed_outer) = self.skip_balanced_angles(i);
            if closed_outer {
                return false;
            }
            i = next;
        }
        matches!(self.peek_at(i), T::OpenParen | T::OpenBracket) || Self::is_expr_arg_binop(self.peek_at(i))
    }

    fn skip_balanced_angles(&self, mut i: usize) -> (usize, bool) {
        let mut angle = 0i32;
        let mut group = 0i32;
        loop {
            match self.peek_at(i) {
                T::OpenParen | T::OpenBracket | T::OpenBrace => group += 1,
                T::CloseParen | T::CloseBracket | T::CloseBrace => group = (group - 1).max(0),
                T::Lt if group == 0 => angle += 1,
                T::Gt if group == 0 => {
                    angle -= 1;
                    if angle <= 0 {
                        return (i + 1, false);
                    }
                }
                T::Shr if group == 0 => {
                    angle -= 2;
                    if angle <= 0 {
                        return (i + 1, angle < 0);
                    }
                }
                T::Eof => return (i, false),
                _ => {}
            }
            i += 1;
        }
    }

    fn skip_balanced_group(&self, mut i: usize) -> usize {
        let mut depth = 0i32;
        loop {
            match self.peek_at(i) {
                T::OpenParen | T::OpenBracket | T::OpenBrace => depth += 1,
                T::CloseParen | T::CloseBracket | T::CloseBrace => {
                    depth -= 1;
                    if depth <= 0 {
                        return i + 1;
                    }
                }
                T::Eof => return i,
                _ => {}
            }
            i += 1;
        }
    }

    fn is_expr_arg_binop(tok: T) -> bool {
        matches!(
            tok,
            T::Plus
                | T::Minus
                | T::Star
                | T::Slash
                | T::Percent
                | T::Ampersand
                | T::Caret
                | T::Shl
                | T::EqEq
                | T::NotEq
                | T::LtEq
                | T::AndAnd
                | T::OrOr
                | T::DotDot
                | T::DotDotEq
                | T::QuestionQuestion
        )
    }

    fn paren_ty(&mut self, start: u32, send: bool) -> Id<Type> {
        if !self.expect(T::OpenParen) {
            return self.error_type();
        }
        let (elements, trailing) = self.list_flagged(T::CloseParen, |p| {
            let is_var = p.eat(T::Var);
            (is_var, p.ty())
        });
        if self.eat(T::Arrow) {
            let ret = self.ty();
            let params = elements.into_iter().map(|(is_var, ty)| FnTypeParam { is_var, ty }).collect();
            return self.type_node(TypeKind::Function { send, params, ret }, start);
        }
        if send {
            self.unexpected("`->`");
        }
        if let Some(&(_, ty)) = elements.iter().find(|(is_var, _)| *is_var) {
            let span = self.ast.types[ty].span;
            self.error("`var` belongs to a function type's parameters", span);
        }
        let mut types: Vec<Id<Type>> = elements.into_iter().map(|(_, ty)| ty).collect();
        if types.len() == 1 && !trailing {
            return self.type_node(TypeKind::Paren(types.remove(0)), start);
        }
        self.type_node(TypeKind::Tuple(types), start)
    }
}
