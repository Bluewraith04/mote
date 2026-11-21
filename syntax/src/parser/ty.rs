use mote_base::Id;

use super::Parser;
use crate::ast::{FnTypeParam, Path, Type, TypeKind};
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
                    let args = self.type_args();
                    return self.type_node(TypeKind::Generic { path, args }, start);
                }
                self.type_node(TypeKind::Named(path), start)
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

    pub(super) fn type_args(&mut self) -> Vec<Id<Type>> {
        self.bump();
        let args = self.free(|p| {
            let mut args = vec![p.ty()];
            while p.eat(T::Comma) && !matches!(p.peek(), T::Gt | T::Shr) {
                args.push(p.ty());
            }
            args
        });
        self.close_angle();
        args
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
