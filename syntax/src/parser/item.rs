use mote_base::Id;

use super::stmt::is_stmt_start;
use super::{is_item_start, Parser};
use crate::ast::*;
use crate::token::TokenKind as T;

impl Parser<'_> {
    pub(super) fn program(&mut self) {
        loop {
            self.skip_separators();
            if self.at(T::Eof) {
                return;
            }
            let (failures, pos) = (self.failures, self.pos);
            let item = self.item();
            self.ast.root.push(item);
            let boundary = matches!(self.peek(), T::Newline | T::Semicolon | T::Eof) || is_item_start(self.peek());
            if self.pos == pos || (self.failures != failures && !boundary) {
                self.recover_item(pos);
                if self.pos == pos {
                    self.bump();
                }
            }
        }
    }

    fn item(&mut self) -> Id<Item> {
        let start = self.start();
        let doc = self.take_doc(start);
        let mut attrs = Vec::new();
        while self.at(T::At) {
            attrs.push(self.attribute());
            self.skip_newlines();
        }
        let is_pub = self.eat(T::Pub);
        let kind = match self.peek() {
            T::Fn => ItemKind::Function(self.function()),
            T::Native => ItemKind::NativeFunction(self.native_function()),
            T::Ident(name) if name == self.words.test && matches!(self.peek_at(1), T::String(_)) => ItemKind::Test(self.test()),
            T::Struct => ItemKind::Record(self.record(RecordKind::Struct)),
            T::Class => ItemKind::Record(self.record(RecordKind::Class)),
            T::Enum => ItemKind::Enum(self.enum_decl()),
            T::Trait => ItemKind::Trait(self.trait_decl()),
            T::Type => ItemKind::TypeAlias(self.type_alias()),
            T::Import => ItemKind::Import(self.import()),
            T::Let | T::Var => ItemKind::Stmt(self.stmt()),
            kind if !is_pub && is_stmt_start(kind) => ItemKind::Stmt(self.stmt()),
            _ => {
                self.unexpected("an item");
                ItemKind::Error
            }
        };
        let span = self.span_from(start);
        self.ast.items.alloc(Item { doc, attrs, is_pub, kind, span })
    }

    fn attribute(&mut self) -> Attribute {
        let start = self.start();
        self.bump();
        let name = self.name();
        let args = if self.eat(T::OpenParen) { self.list(T::CloseParen, |p| p.attribute_arg()) } else { Vec::new() };
        Attribute { name, args, span: self.span_from(start) }
    }

    fn attribute_arg(&mut self) -> AttrArg {
        let start = self.start();
        let mut key = None;
        if matches!(self.peek(), T::Ident(_)) {
            let path = self.path();
            if self.eat(T::Eq) {
                key = Some(path);
            } else {
                return AttrArg { key: None, value: AttrValue::Path(path), span: self.span_from(start) };
            }
        }
        let value = match self.peek() {
            T::Ident(_) => AttrValue::Path(self.path()),
            T::String(text) => {
                self.bump();
                AttrValue::String(text)
            }
            T::Int(literal) => {
                self.bump();
                AttrValue::Int(literal)
            }
            T::Float(literal) => {
                self.bump();
                AttrValue::Float(literal)
            }
            T::True | T::False => AttrValue::Bool(self.bump().kind == T::True),
            _ => {
                self.unexpected("a name or a literal");
                AttrValue::Bool(false)
            }
        };
        AttrArg { key, value, span: self.span_from(start) }
    }

    fn generics(&mut self) -> Vec<GenericParam> {
        if !self.at(T::Lt) {
            return Vec::new();
        }
        self.bump();
        let params = self.free(|p| {
            let mut params = Vec::new();
            loop {
                let name = p.name();
                let mut bounds = Vec::new();
                if p.eat(T::Colon) {
                    bounds.push(p.ty());
                    while p.eat(T::Plus) {
                        bounds.push(p.ty());
                    }
                }
                params.push(GenericParam { name, bounds });
                if !p.eat(T::Comma) || matches!(p.peek(), T::Gt | T::Shr) {
                    return params;
                }
            }
        });
        self.close_angle();
        params
    }

    fn traits(&mut self) -> Vec<Id<Type>> {
        if !self.eat(T::Colon) {
            return Vec::new();
        }
        if !self.expect(T::OpenParen) {
            return Vec::new();
        }
        self.list(T::CloseParen, |p| p.ty())
    }

    fn params(&mut self) -> Vec<Param> {
        if !self.expect(T::OpenParen) {
            return Vec::new();
        }
        self.list(T::CloseParen, |p| p.param())
    }

    fn param(&mut self) -> Param {
        let start = self.start();
        let kind = match self.peek() {
            T::SelfLower => {
                self.bump();
                ParamKind::SelfValue { is_var: false }
            }
            T::Var if self.peek_at(1) == T::SelfLower => {
                self.bump();
                self.bump();
                ParamKind::SelfValue { is_var: true }
            }
            T::DotDotDot => {
                self.bump();
                let name = self.name();
                let ty = if self.eat(T::Colon) { Some(self.ty()) } else { None };
                ParamKind::Variadic { name, ty }
            }
            _ => {
                let is_var = self.eat(T::Var);
                let name = self.name();
                let ty = if self.eat(T::Colon) { Some(self.ty()) } else { None };
                let default = if self.eat(T::Eq) { Some(self.expr()) } else { None };
                ParamKind::Named { is_var, name, ty, default }
            }
        };
        Param { kind, span: self.span_from(start) }
    }

    fn return_type(&mut self) -> Option<Id<Type>> {
        if self.eat(T::Arrow) { Some(self.ty()) } else { None }
    }

    pub(super) fn function(&mut self) -> Function {
        self.expect(T::Fn);
        let name = self.member_name();
        let generics = self.generics();
        let params = self.params();
        let ret = self.return_type();
        let body = self.block();
        Function { name, generics, params, ret, body }
    }

    fn native_function(&mut self) -> NativeFunction {
        self.bump();
        self.expect(T::Fn);
        let name = self.name();
        let generics = self.generics();
        let params = self.params();
        let ret = self.return_type();
        NativeFunction { name, generics, params, ret }
    }

    fn test(&mut self) -> Test {
        self.bump();
        let T::String(title) = self.bump().kind else { unreachable!() };
        Test { title, body: self.block() }
    }

    fn method(&mut self, doc: Option<mote_base::Span>, is_pub: bool, start: u32) -> Method {
        let function = self.function();
        Method { doc, is_pub, function, span: self.span_from(start) }
    }

    fn field(&mut self) -> Field {
        let start = self.start();
        let doc = self.take_doc(start);
        let is_pub = self.eat(T::Pub);
        self.field_rest(doc, is_pub, start)
    }

    fn field_rest(&mut self, doc: Option<mote_base::Span>, is_pub: bool, start: u32) -> Field {
        let binding = match self.peek() {
            T::Let => Some(BindingKind::Let),
            T::Var => Some(BindingKind::Var),
            _ => None,
        };
        if binding.is_some() {
            self.bump();
        }
        let name = self.name();
        let ty = if self.expect(T::Colon) { self.ty() } else { self.error_type() };
        let default = if self.eat(T::Eq) { Some(self.expr()) } else { None };
        Field { doc, is_pub, binding, name, ty, default, span: self.span_from(start) }
    }

    fn member(&mut self) -> Member {
        let start = self.start();
        let doc = self.take_doc(start);
        let is_pub = self.eat(T::Pub);
        if self.at(T::Fn) { Member::Method(self.method(doc, is_pub, start)) } else { Member::Field(self.field_rest(doc, is_pub, start)) }
    }

    fn skip_member(&mut self) {
        let mut depth = 0u32;
        loop {
            match self.peek() {
                T::Eof => return,
                T::CloseBrace if depth == 0 => return,
                T::Pub | T::Fn | T::Var | T::Let if depth == 0 => return,
                T::Ident(_) if depth == 0 && self.peek_at(1) == T::Colon => return,
                T::OpenParen | T::OpenBracket | T::OpenBrace => depth += 1,
                T::CloseParen | T::CloseBracket | T::CloseBrace => depth = depth.saturating_sub(1),
                _ => {}
            }
            self.bump();
        }
    }

    fn record(&mut self, kind: RecordKind) -> Record {
        self.bump();
        let name = self.name();
        let generics = self.generics();
        let traits = self.traits();
        let mut members = Vec::new();
        if self.open_brace() {
            while !self.at(T::CloseBrace) && !self.at(T::Eof) {
                let (failures, pos) = (self.failures, self.pos);
                members.push(self.member());
                if self.failures != failures || self.pos == pos {
                    self.bump_if_stuck(pos);
                    self.skip_member();
                }
            }
            self.expect(T::CloseBrace);
        }
        Record { kind, name, generics, traits, members }
    }

    fn bump_if_stuck(&mut self, pos: usize) {
        if self.pos == pos {
            self.bump();
        }
    }

    fn enum_decl(&mut self) -> Enum {
        self.bump();
        let name = self.name();
        let generics = self.generics();
        let traits = self.traits();
        let mut variants = Vec::new();
        let mut methods = Vec::new();
        if self.open_brace() {
            while !self.at(T::CloseBrace) && !self.at(T::Eof) {
                let (failures, pos) = (self.failures, self.pos);
                let start = self.start();
                let doc = self.take_doc(start);
                if matches!(self.peek(), T::Pub | T::Fn) {
                    let is_pub = self.eat(T::Pub);
                    methods.push(self.method(doc, is_pub, start));
                } else {
                    variants.push(self.variant(doc, start));
                }
                if self.failures != failures || self.pos == pos {
                    self.bump_if_stuck(pos);
                    self.skip_in_list(T::CloseBrace);
                }
                self.eat(T::Comma);
            }
            self.expect(T::CloseBrace);
        }
        Enum { name, generics, traits, variants, methods }
    }

    fn variant(&mut self, doc: Option<mote_base::Span>, start: u32) -> Variant {
        let mut attrs = Vec::new();
        while self.at(T::At) {
            attrs.push(self.attribute());
        }
        let name = self.name();
        let payload = match self.peek() {
            T::OpenParen => {
                self.bump();
                VariantPayload::Tuple(self.list(T::CloseParen, |p| p.ty()))
            }
            T::OpenBrace => {
                self.bump();
                VariantPayload::Struct(self.free(|p| p.items_until(T::CloseBrace, Self::field)))
            }
            T::Eq => {
                self.bump();
                VariantPayload::Discriminant(self.expr())
            }
            _ => VariantPayload::Unit,
        };
        Variant { doc, attrs, name, payload, span: self.span_from(start) }
    }

    fn trait_decl(&mut self) -> Trait {
        self.bump();
        let name = self.name();
        let generics = self.generics();
        let mut members = Vec::new();
        if self.open_brace() {
            while !self.at(T::CloseBrace) && !self.at(T::Eof) {
                let (failures, pos) = (self.failures, self.pos);
                members.push(self.trait_member());
                if self.failures != failures || self.pos == pos {
                    self.bump_if_stuck(pos);
                    self.skip_member();
                }
            }
            self.expect(T::CloseBrace);
        }
        Trait { name, generics, members }
    }

    fn trait_member(&mut self) -> TraitMember {
        let start = self.start();
        let doc = self.take_doc(start);
        self.expect(T::Fn);
        let name = self.member_name();
        let generics = self.generics();
        let params = self.params();
        let ret = self.return_type();
        let body = if self.at(T::OpenBrace) { Some(self.block()) } else { None };
        TraitMember { doc, name, generics, params, ret, body, span: self.span_from(start) }
    }

    fn type_alias(&mut self) -> TypeAlias {
        self.bump();
        let name = self.name();
        let generics = self.generics();
        let target = if self.expect(T::Eq) { self.ty() } else { self.error_type() };
        TypeAlias { name, generics, target }
    }

    fn import(&mut self) -> Import {
        self.bump();
        if !self.eat(T::OpenBrace) {
            let path = self.module_path();
            let alias = if self.eat(T::As) { Some(self.name()) } else { None };
            return Import::Module { path, alias };
        }
        if self.eat(T::Star) {
            self.expect(T::CloseBrace);
            self.expect(T::From);
            return Import::Glob { from: self.module_path() };
        }
        let names = self.list(T::CloseBrace, |p| {
            let name = p.name();
            let alias = if p.eat(T::As) { Some(p.name()) } else { None };
            ImportName { name, alias }
        });
        self.expect(T::From);
        Import::Names { names, from: self.module_path() }
    }

    fn module_path(&mut self) -> ModulePath {
        let start = self.start();
        let mut prefix = Vec::new();
        loop {
            match self.peek() {
                T::Dot => prefix.push(PathPrefix::Dot),
                T::DotDot => prefix.push(PathPrefix::DotDot),
                _ => break,
            }
            self.bump();
        }
        while self.at(T::Super) && self.peek_at(1) == T::Dot {
            self.bump();
            self.bump();
            prefix.push(PathPrefix::Super);
        }
        let mut segments = vec![self.name()];
        while self.at(T::Dot) && matches!(self.peek_at(1), T::Ident(_)) {
            self.bump();
            segments.push(self.name());
        }
        ModulePath { prefix, segments, span: self.span_from(start) }
    }
}
