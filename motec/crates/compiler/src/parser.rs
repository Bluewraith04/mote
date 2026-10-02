use crate::ast::*;
use crate::span::Span;
use crate::token::{HolePos, StrPart, Token, TokenKind};

mod expr;
mod stmt;

/// Turns tokens into a [`Program`].
pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    stable_marks: Vec<StableMark>,
    test_marks: Vec<TestMark>,
    test_counter: usize,
    operand_type: bool,
    match_subject: bool,
    json_derive: Option<Span>,
    args_derive: Option<Span>,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0, stable_marks: Vec::new(), test_marks: Vec::new(), test_counter: 0, operand_type: false, match_subject: false, json_derive: None, args_derive: None }
    }

    pub fn parse(&mut self) -> Result<Program, (String, Span)> {
        let mut items = Vec::new();
        self.skip_newlines();

        while !self.is_at_end() {
            let item = self.parse_item()?;
            items.push(item);
            self.skip_newlines();
        }

        if let Some(span) = self.json_derive.take() {
            let path = ModulePath::new(vec!["std".into(), "data".into(), "json".into()], false, 0, span);
            items.push(Item::Import(ImportDecl { path, from_path: None, alias: Some("__json".into()), symbols: Vec::new(), is_pub: false, span }));
        }
        if let Some(span) = self.args_derive.take() {
            let path = ModulePath::new(vec!["std".into(), "dev".into(), "args".into()], false, 0, span);
            items.push(Item::Import(ImportDecl { path, from_path: None, alias: Some("__args".into()), symbols: Vec::new(), is_pub: false, span }));
        }

        let span = if let Some(first) = items.first() {
            let last = items.last().unwrap();
            first.span().merge(&last.span())
        } else {
            Span::new(0, 0, 1, 1)
        };

        Ok(Program {
            items,
            span,
            stable_marks: std::mem::take(&mut self.stable_marks),
            test_marks: std::mem::take(&mut self.test_marks),
        })
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or_else(|| self.tokens.last().unwrap())
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.peek().kind
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset)
    }

    fn ternary_colon_ahead(&self) -> bool {
        let mut depth: i32 = 0;
        let mut i = self.pos;
        while let Some(tok) = self.tokens.get(i) {
            match &tok.kind {
                TokenKind::OpenParen | TokenKind::OpenBracket | TokenKind::OpenBrace => depth += 1,
                TokenKind::CloseParen | TokenKind::CloseBracket | TokenKind::CloseBrace => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                }
                TokenKind::Colon if depth == 0 => return true,
                TokenKind::Newline | TokenKind::Semicolon | TokenKind::Eof | TokenKind::Comma
                    if depth == 0 =>
                {
                    return false
                }
                _ => {}
            }
            i += 1;
        }
        false
    }

    fn is_at_end(&self) -> bool {
        matches!(self.peek_kind(), TokenKind::Eof)
    }

    fn advance(&mut self) -> Token {
        if !self.is_at_end() {
            let tok = self.tokens[self.pos].clone();
            self.pos += 1;
            tok
        } else {
            self.peek().clone()
        }
    }

    fn check(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(self.peek_kind()) == std::mem::discriminant(kind)
    }

    fn check_at(&self, offset: usize, kind: &TokenKind) -> bool {
        if let Some(tok) = self.peek_at(offset) {
            std::mem::discriminant(&tok.kind) == std::mem::discriminant(kind)
        } else {
            false
        }
    }

    fn match_token(&mut self, kind: &TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: TokenKind) -> Result<Token, (String, Span)> {
        if self.check(&kind) {
            Ok(self.advance())
        } else {
            let current = self.peek();
            Err((
                format!("Expected {:?}, found {:?}", kind, current.kind),
                current.span,
            ))
        }
    }

    fn expect_generic_close(&mut self) -> Result<Token, (String, Span)> {
        if self.check(&TokenKind::Shr) {
            let tok = self.tokens[self.pos].clone();
            self.tokens[self.pos].kind = TokenKind::Gt;
            return Ok(Token { kind: TokenKind::Gt, ..tok });
        }
        self.expect(TokenKind::Gt)
    }

    fn expect_ident(&mut self) -> Result<(String, Span), (String, Span)> {
        let tok = self.peek().clone();
        match tok.kind {
            TokenKind::Ident(s) => {
                self.advance();
                Ok((s, tok.span))
            }
            _ => Err((format!("Expected identifier, found {:?}", tok.kind), tok.span)),
        }
    }

    fn expect_member_name(&mut self) -> Result<(String, Span), (String, Span)> {
        if self.check(&TokenKind::From) {
            let tok = self.advance();
            return Ok(("from".to_string(), tok.span));
        }
        self.expect_ident()
    }

    fn skip_newlines(&mut self) {
        while self.match_token(&TokenKind::Newline) || self.match_token(&TokenKind::Semicolon) {}
    }

    fn is_statement_terminator(&self) -> bool {
        matches!(
            self.peek_kind(),
            TokenKind::Newline | TokenKind::Semicolon | TokenKind::CloseBrace | TokenKind::Eof
        )
    }

    fn consume_statement_terminator(&mut self) {
        if self.match_token(&TokenKind::Semicolon) || self.match_token(&TokenKind::Newline) {
            self.skip_newlines();
        }
    }

    fn parse_item(&mut self) -> Result<Item, (String, Span)> {
        let attrs = self.parse_attributes()?;
        let is_pub = self.match_token(&TokenKind::Pub);

        let is_test_decl = matches!(self.peek_kind(), TokenKind::Ident(name) if name == "test")
            && matches!(self.peek_at(1).map(|t| &t.kind), Some(TokenKind::String(_)))
            && matches!(self.peek_at(2).map(|t| &t.kind), Some(TokenKind::OpenBrace));

        let mut test_decl_display_name: Option<String> = None;
        let item = if is_test_decl {
            if is_pub {
                return Err(("a `test` block cannot be `pub`".to_string(), self.peek().span));
            }
            let (func, display_name) = self.parse_test_decl()?;
            test_decl_display_name = Some(display_name);
            Ok(Item::Function(func))
        } else {
            match self.peek_kind() {
                TokenKind::Fn => {
                    let func = self.parse_function_decl(is_pub)?;
                    Ok(Item::Function(func))
                }
                TokenKind::Native if self.check_at(1, &TokenKind::Fn) => {
                    let func = self.parse_native_fn_decl(is_pub)?;
                    Ok(Item::NativeFunction(func))
                }
                TokenKind::Struct => {
                    let s = self.parse_struct_decl(is_pub)?;
                    Ok(Item::Struct(s))
                }
                TokenKind::Enum => {
                    let e = self.parse_enum_decl(is_pub)?;
                    Ok(Item::Enum(e))
                }
                TokenKind::Class => {
                    let c = self.parse_class_decl(is_pub)?;
                    Ok(Item::Class(c))
                }
                TokenKind::Trait => {
                    let t = self.parse_trait_decl(is_pub)?;
                    Ok(Item::Trait(t))
                }
                TokenKind::Impl => {
                    return Err((
                        "there is no `impl`: write the methods in the type's body and list traits as `struct T: (Trait)`".to_string(),
                        self.peek().span,
                    ));
                }
                TokenKind::Type => {
                    let a = self.parse_type_alias_decl(is_pub)?;
                    Ok(Item::TypeAlias(a))
                }
                TokenKind::Import => {
                    let imp = self.parse_import_decl(is_pub)?;
                    Ok(Item::Import(imp))
                }
                _ => {
                    let mut stmt = self.parse_stmt()?;
                    if is_pub {
                        match &mut stmt {
                            Stmt::Let { is_pub, .. } | Stmt::Var { is_pub, .. } => *is_pub = true,
                            _ => {
                                return Err((
                                    "`pub` is only valid on a declaration or a top-level `let` / `var`"
                                        .to_string(),
                                    stmt.span(),
                                ))
                            }
                        }
                    }
                    Ok(Item::TopLevelStmt(stmt))
                }
            }
        }?;
        let has_stable = attrs.iter().any(|a| a.name == "stable");
        let stable_span = attrs.iter().find(|a| a.name == "stable").map(|a| a.span);
        let has_test_attr = attrs.iter().any(|a| a.name == "test");
        let ignored = attrs.iter().any(|a| a.name == "ignore");
        if !is_test_decl && ignored && !has_test_attr {
            let span = attrs.iter().find(|a| a.name == "ignore").unwrap().span;
            return Err(("`@ignore` needs a `test` block or `@test` to skip".to_string(), span));
        }
        if self.json_derive.is_none() && crate::derive::uses(&attrs, "Json") {
            self.json_derive = attrs.iter().find(|a| a.name == "derive").map(|a| a.span);
        }
        if self.args_derive.is_none() && crate::derive::uses(&attrs, "Args") {
            self.args_derive = attrs.iter().find(|a| a.name == "derive").map(|a| a.span);
        }
        let item = crate::derive::apply_attributes(attrs, item)?;
        if has_stable
            && let Some((kind, _, name, signature)) = crate::stable::describe(&item) {
                self.stable_marks.push(StableMark { name, kind: kind.to_string(), signature, span: stable_span.unwrap() });
            }
        if (is_test_decl || has_test_attr)
            && let Item::Function(f) = &item {
                if !f.params.is_empty() {
                    return Err(("a discovered test takes no parameters".to_string(), f.span));
                }
                let display_name = test_decl_display_name.unwrap_or_else(|| f.name.clone());
                self.test_marks.push(TestMark { display_name, fn_name: f.name.clone(), ignored, span: f.span });
            }
        Ok(item)
    }

    fn parse_attributes(&mut self) -> Result<Vec<Attribute>, (String, Span)> {
        let mut attrs = Vec::new();
        while self.check(&TokenKind::At) {
            let at_tok = self.expect(TokenKind::At)?;
            let (name, name_span) = self.expect_ident()?;
            let mut args = Vec::new();
            let mut values = Vec::new();
            let mut end_span = name_span;
            if self.match_token(&TokenKind::OpenParen) {
                if !self.check(&TokenKind::CloseParen) {
                    loop {
                        let (arg, _) = self.expect_ident()?;
                        if self.match_token(&TokenKind::Eq) {
                            values.push((arg, self.parse_attribute_text()?));
                        } else {
                            args.push(arg);
                        }
                        if !self.match_token(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                let close = self.expect(TokenKind::CloseParen)?;
                end_span = close.span;
            }
            attrs.push(Attribute { name, args, values, span: at_tok.span.merge(&end_span) });
            self.skip_newlines();
        }
        Ok(attrs)
    }

    fn parse_attribute_text(&mut self) -> Result<String, (String, Span)> {
        let tok = self.advance();
        match &tok.kind {
            TokenKind::String(parts) if parts.iter().all(|p| matches!(p, StrPart::Lit(_))) => {
                Ok(parts.iter().map(|p| if let StrPart::Lit(s) = p { s.as_str() } else { "" }).collect())
            }
            _ => Err(("an attribute value is a plain string, as in `key = \"text\"`".to_string(), tok.span)),
        }
    }

    fn parse_generic_params(&mut self) -> Result<Vec<GenericParam>, (String, Span)> {
        let mut params = Vec::new();
        if self.match_token(&TokenKind::Lt) {
            loop {
                let (name, span) = self.expect_ident()?;
                let mut bounds = Vec::new();
                if self.match_token(&TokenKind::Colon) {
                    bounds.push(self.parse_type()?);
                    while self.match_token(&TokenKind::Plus) {
                        bounds.push(self.parse_type()?);
                    }
                }
                params.push(GenericParam { name, bounds, span });
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect_generic_close()?;
        }
        Ok(params)
    }

    fn parse_function_decl(&mut self, is_pub: bool) -> Result<FunctionDecl, (String, Span)> {
        let fn_tok = self.expect(TokenKind::Fn)?;
        let name_tok = self.expect_member_name()?;
        let generic_params = self.parse_generic_params()?;

        let params = self.parse_param_list()?;

        let mut return_type = None;
        if self.match_token(&TokenKind::Arrow) {
            return_type = Some(self.parse_type()?);
        }

        self.skip_newlines();
        let body = self.parse_block()?;
        let span = fn_tok.span.merge(self.tokens.get(self.pos - 1).map(|t| &t.span).unwrap_or(&fn_tok.span));

        Ok(FunctionDecl {
            name: name_tok.0,
            generic_params,
            params,
            return_type,
            body,
            is_pub,
            span,
        })
    }

    fn parse_test_decl(&mut self) -> Result<(FunctionDecl, String), (String, Span)> {
        let test_tok = self.advance();
        let name_tok = self.advance();
        let display_name = match name_tok.kind {
            TokenKind::String(parts) if parts.len() == 1 => match &parts[0] {
                StrPart::Lit(s) => s.clone(),
                StrPart::Hole(..) => {
                    return Err(("a test's name cannot contain `${…}` interpolation".to_string(), name_tok.span))
                }
            },
            TokenKind::String(_) => {
                return Err(("a test's name cannot contain `${…}` interpolation".to_string(), name_tok.span))
            }
            _ => return Err(("expected a string literal after `test`".to_string(), name_tok.span)),
        };

        self.skip_newlines();
        let body = self.parse_block()?;
        let span = test_tok.span.merge(self.tokens.get(self.pos - 1).map(|t| &t.span).unwrap_or(&test_tok.span));

        let name = format!("__mote_test_{}", self.test_counter);
        self.test_counter += 1;

        Ok((
            FunctionDecl {
                name,
                generic_params: Vec::new(),
                params: Vec::new(),
                return_type: None,
                body,
                is_pub: false,
                span,
            },
            display_name,
        ))
    }

    fn parse_native_fn_decl(&mut self, is_pub: bool) -> Result<NativeFnDecl, (String, Span)> {
        let native_tok = self.expect(TokenKind::Native)?;
        self.expect(TokenKind::Fn)?;
        let name_tok = self.expect_ident()?;

        if self.check(&TokenKind::Lt) {
            return Err((
                "`native fn` cannot have generic parameters".to_string(),
                self.peek().span,
            ));
        }

        let params = self.parse_param_list()?;

        let mut return_type = None;
        if self.match_token(&TokenKind::Arrow) {
            return_type = Some(self.parse_type()?);
        }

        if self.check(&TokenKind::OpenBrace) {
            return Err((
                "`native fn` has no body — it declares a function implemented in a loaded native library".to_string(),
                self.peek().span,
            ));
        }
        let after_newline = self.pos > 0 && self.tokens[self.pos].span.line > self.tokens[self.pos - 1].span.line;
        if !self.is_statement_terminator() && !after_newline {
            return Err((
                format!("Expected end of `native fn` declaration, found {:?}", self.peek_kind()),
                self.peek().span,
            ));
        }

        let end_span = self.tokens.get(self.pos.saturating_sub(1)).map(|t| t.span).unwrap_or(native_tok.span);
        let span = native_tok.span.merge(&end_span);

        Ok(NativeFnDecl {
            name: name_tok.0,
            params,
            return_type,
            is_pub,
            span,
        })
    }

    fn parse_param_list(&mut self) -> Result<Vec<Param>, (String, Span)> {
        self.expect(TokenKind::OpenParen)?;
        let mut params = Vec::new();
        if !self.check(&TokenKind::CloseParen) {
            loop {
                if self.check(&TokenKind::Ampersand) {
                    let a_tok = self.advance();
                    let is_mut = self.match_token(&TokenKind::Mut);
                    let s_tok = self.expect(TokenKind::SelfLower)?;
                    let want = if is_mut { "var self" } else { "self" };
                    return Err((format!("write `{want}`"), a_tok.span.merge(&s_tok.span)));
                }
                if self.check(&TokenKind::Mut) {
                    let m_tok = self.advance();
                    let (name, n_span) = if self.check(&TokenKind::SelfLower) {
                        ("self".to_string(), self.advance().span)
                    } else {
                        self.expect_ident()?
                    };
                    return Err((format!("write `var {name}`"), m_tok.span.merge(&n_span)));
                }
                let var_tok = self.check(&TokenKind::Var).then(|| self.advance());
                let is_var = var_tok.is_some();
                if self.check(&TokenKind::SelfLower) {
                    let s_tok = self.advance();
                    let span = var_tok.as_ref().map_or(s_tok.span, |v| v.span.merge(&s_tok.span));
                    params.push(Param {
                        name: "self".into(),
                        ty: None,
                        kind: Some(ParameterKind::SelfValue { is_var }),
                        is_mut: is_var,
                        span,
                    });
                } else if !is_var && self.check(&TokenKind::DotDotDot) {
                    let dots_tok = self.advance();
                    let p_name_tok = self.expect_ident()?;
                    let mut ty = None;
                    if self.match_token(&TokenKind::Colon) {
                        ty = Some(self.parse_type()?);
                    }
                    if self.check(&TokenKind::Eq) {
                        return Err(("a variadic parameter cannot have a default value".into(), self.peek().span));
                    }
                    params.push(Param {
                        name: p_name_tok.0.clone(),
                        ty: ty.clone(),
                        kind: Some(ParameterKind::Variadic { name: p_name_tok.0, ty }),
                        is_mut: false,
                        span: dots_tok.span.merge(&p_name_tok.1),
                    });
                } else {
                    let is_mut = is_var;
                    let p_name_tok = self.expect_ident()?;
                    let mut ty = None;
                    if self.match_token(&TokenKind::Colon) {
                        ty = Some(self.parse_type()?);
                    }
                    let default_val = if self.match_token(&TokenKind::Eq) {
                        Some(self.parse_expr(0)?)
                    } else {
                        None
                    };
                    params.push(Param {
                        name: p_name_tok.0.clone(),
                        ty: ty.clone(),
                        kind: Some(ParameterKind::Regular {
                            name: p_name_tok.0,
                            ty,
                            default_value: default_val,
                            is_mut,
                        }),
                        is_mut,
                        span: p_name_tok.1,
                    });
                }

                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(TokenKind::CloseParen)?;
        Ok(params)
    }

    fn parse_trait_list(&mut self, kind: &str) -> Result<Vec<TypeNode>, (String, Span)> {
        let mut traits = Vec::new();
        if !self.check(&TokenKind::Colon) {
            return Ok(traits);
        }
        self.advance();
        if !self.check(&TokenKind::OpenParen) {
            let what = if kind == "class" { format!("a class has no parent: list traits as `{kind} C: (Trait)`") } else { format!("list traits in parentheses: `{kind} C: (Trait)`") };
            return Err((what, self.peek().span));
        }
        self.advance();
        loop {
            traits.push(self.parse_type()?);
            if !self.match_token(&TokenKind::Comma) || self.check(&TokenKind::CloseParen) {
                break;
            }
        }
        self.expect(TokenKind::CloseParen)?;
        Ok(traits)
    }

    fn parse_struct_decl(&mut self, is_pub: bool) -> Result<StructDecl, (String, Span)> {
        let struct_tok = self.expect(TokenKind::Struct)?;
        let name_tok = self.expect_ident()?;
        let generic_params = self.parse_generic_params()?;
        let traits = self.parse_trait_list("struct")?;

        self.skip_newlines();
        self.expect(TokenKind::OpenBrace)?;
        self.skip_newlines();

        let mut fields = Vec::new();
        let mut methods = Vec::new();
        while !self.check(&TokenKind::CloseBrace) && !self.is_at_end() {
            let attrs = self.parse_attributes()?;
            let is_member_pub = self.match_token(&TokenKind::Pub);
            if self.check(&TokenKind::Fn) {
                if let Some(a) = attrs.first() {
                    return Err((format!("`@{}` cannot go on a method", a.name), a.span));
                }
                methods.push(self.parse_function_decl(is_member_pub)?);
                self.skip_newlines();
                continue;
            }
            let is_mut = self.match_token(&TokenKind::Var);
            if !is_mut {
                self.match_token(&TokenKind::Let);
            }
            let f_name_tok = self.expect_ident()?;
            self.expect(TokenKind::Colon)?;
            let f_ty = self.parse_type()?;
            self.skip_newlines();

            fields.push(FieldDecl {
                name: f_name_tok.0,
                ty: f_ty,
                is_mutable: is_mut,
                is_pub: is_member_pub,
                attrs,
                span: f_name_tok.1,
            });
        }
        let close_brace = self.expect(TokenKind::CloseBrace)?;

        Ok(StructDecl {
            name: name_tok.0,
            generic_params,
            traits,
            fields,
            methods,
            is_pub,
            span: struct_tok.span.merge(&close_brace.span),
        })
    }

    fn parse_enum_decl(&mut self, is_pub: bool) -> Result<EnumDecl, (String, Span)> {
        let enum_tok = self.expect(TokenKind::Enum)?;
        let name_tok = self.expect_ident()?;
        let generic_params = self.parse_generic_params()?;
        let traits = self.parse_trait_list("enum")?;

        self.skip_newlines();
        self.expect(TokenKind::OpenBrace)?;
        self.skip_newlines();

        let mut variants = Vec::new();
        let mut methods = Vec::new();
        while !self.check(&TokenKind::CloseBrace) && !self.is_at_end() {
            let variant_attrs = self.parse_attributes()?;
            if self.check(&TokenKind::Fn) || (self.check(&TokenKind::Pub) && self.check_at(1, &TokenKind::Fn)) {
                if let Some(a) = variant_attrs.first() {
                    return Err((format!("`@{}` cannot go on a method", a.name), a.span));
                }
                let is_method_pub = self.match_token(&TokenKind::Pub);
                methods.push(self.parse_function_decl(is_method_pub)?);
                self.skip_newlines();
                continue;
            }
            if !methods.is_empty() {
                return Err(("variants come before methods in an enum body".to_string(), self.peek().span));
            }
            let (v_name, v_span) = self.expect_ident()?;
            let kind = if self.check(&TokenKind::OpenParen) {
                self.advance();
                let mut tuple_types = Vec::new();
                if !self.check(&TokenKind::CloseParen) {
                    loop {
                        tuple_types.push(self.parse_type()?);
                        if !self.match_token(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(TokenKind::CloseParen)?;
                EnumVariantKind::Tuple(tuple_types)
            } else if self.check(&TokenKind::OpenBrace) {
                self.advance();
                let mut struct_fields = Vec::new();
                while !self.check(&TokenKind::CloseBrace) && !self.is_at_end() {
                    let attrs = self.parse_attributes()?;
                    let is_mut = self.match_token(&TokenKind::Var);
                    if !is_mut {
                        self.match_token(&TokenKind::Let);
                    }
                    let f_name_tok = self.expect_ident()?;
                    self.expect(TokenKind::Colon)?;
                    let f_ty = self.parse_type()?;
                    self.match_token(&TokenKind::Comma);
                    self.skip_newlines();
                    struct_fields.push(FieldDecl {
                        name: f_name_tok.0,
                        ty: f_ty,
                        is_mutable: is_mut,
                        is_pub: true,
                        attrs,
                        span: f_name_tok.1,
                    });
                }
                self.expect(TokenKind::CloseBrace)?;
                EnumVariantKind::Struct(struct_fields)
            } else if self.match_token(&TokenKind::Eq) {
                let disc = self.parse_expr(0)?;
                EnumVariantKind::Unit { discriminant: Some(disc) }
            } else {
                EnumVariantKind::Unit { discriminant: None }
            };

            variants.push(EnumVariant {
                name: v_name,
                kind,
                attrs: variant_attrs,
                span: v_span,
            });

            self.match_token(&TokenKind::Comma);
            self.skip_newlines();
            if self.check(&TokenKind::CloseBrace) {
                break;
            }
            self.skip_newlines();
        }
        let close_brace = self.expect(TokenKind::CloseBrace)?;

        Ok(EnumDecl {
            name: name_tok.0,
            generic_params,
            traits,
            variants,
            methods,
            is_pub,
            span: enum_tok.span.merge(&close_brace.span),
        })
    }

    fn parse_class_decl(&mut self, is_pub: bool) -> Result<ClassDecl, (String, Span)> {
        let class_tok = self.expect(TokenKind::Class)?;
        let name_tok = self.expect_ident()?;
        let generic_params = self.parse_generic_params()?;

        let traits = self.parse_trait_list("class")?;

        self.skip_newlines();
        self.expect(TokenKind::OpenBrace)?;
        self.skip_newlines();

        let mut fields = Vec::new();
        let mut methods = Vec::new();

        while !self.check(&TokenKind::CloseBrace) && !self.is_at_end() {
            let attrs = self.parse_attributes()?;
            let is_method_pub = self.match_token(&TokenKind::Pub);
            if self.check(&TokenKind::Fn) {
                if let Some(a) = attrs.first() {
                    return Err((format!("`@{}` cannot go on a method", a.name), a.span));
                }
                let method = self.parse_function_decl(is_method_pub)?;
                methods.push(method);
            } else {
                let is_mut = self.match_token(&TokenKind::Var);
                if !is_mut {
                    self.match_token(&TokenKind::Let);
                }
                let f_name_tok = self.expect_ident()?;
                self.expect(TokenKind::Colon)?;
                let f_ty = self.parse_type()?;
                self.skip_newlines();

                fields.push(FieldDecl {
                    name: f_name_tok.0,
                    ty: f_ty,
                    is_mutable: is_mut,
                    is_pub: is_method_pub,
                    attrs,
                    span: f_name_tok.1,
                });
            }
            self.skip_newlines();
        }
        let close_brace = self.expect(TokenKind::CloseBrace)?;

        Ok(ClassDecl {
            name: name_tok.0,
            generic_params,
            traits,
            fields,
            methods,
            is_pub,
            span: class_tok.span.merge(&close_brace.span),
        })
    }

    fn parse_trait_decl(&mut self, is_pub: bool) -> Result<TraitDecl, (String, Span)> {
        let trait_tok = self.expect(TokenKind::Trait)?;
        let name_tok = self.expect_ident()?;
        let generic_params = self.parse_generic_params()?;

        self.skip_newlines();
        self.expect(TokenKind::OpenBrace)?;
        self.skip_newlines();

        let mut members = Vec::new();
        while !self.check(&TokenKind::CloseBrace) && !self.is_at_end() {
            let fn_tok = self.expect(TokenKind::Fn)?;
            let m_name_tok = self.expect_ident()?;
            let m_generics = self.parse_generic_params()?;

            let params = self.parse_param_list()?;

            let mut return_type = None;
            if self.match_token(&TokenKind::Arrow) {
                return_type = Some(self.parse_type()?);
            }

            self.skip_newlines();
            let default_body = if self.check(&TokenKind::OpenBrace) {
                Some(self.parse_block()?)
            } else {
                self.consume_statement_terminator();
                None
            };

            members.push(TraitMember {
                name: m_name_tok.0,
                generic_params: m_generics,
                params,
                return_type,
                default_body,
                span: fn_tok.span,
            });
            self.skip_newlines();
        }
        let close_brace = self.expect(TokenKind::CloseBrace)?;

        Ok(TraitDecl {
            name: name_tok.0,
            generic_params,
            members,
            is_pub,
            span: trait_tok.span.merge(&close_brace.span),
        })
    }

    fn parse_type_alias_decl(&mut self, is_pub: bool) -> Result<TypeAliasDecl, (String, Span)> {
        let type_tok = self.expect(TokenKind::Type)?;
        let name_tok = self.expect_ident()?;
        let generic_params = self.parse_generic_params()?;
        self.expect(TokenKind::Eq)?;
        let target = self.parse_type()?;
        let span = type_tok.span.merge(&target.span());
        self.consume_statement_terminator();

        Ok(TypeAliasDecl {
            name: name_tok.0,
            generic_params,
            target,
            is_pub,
            span,
        })
    }

    fn parse_module_path(&mut self) -> Result<ModulePath, (String, Span)> {
        let start_span = self.peek().span;
        let mut relative_depth = 0;
        let mut is_relative = false;

        loop {
            if self.match_token(&TokenKind::DotDot) {
                is_relative = true;
                relative_depth += 2;
            } else if self.match_token(&TokenKind::Dot) {
                is_relative = true;
                relative_depth += 1;
            } else {
                break;
            }
        }

        if self.check(&TokenKind::Super) && !is_relative {
            is_relative = true;
            relative_depth = 1;
        }
        while self.match_token(&TokenKind::Super) {
            relative_depth += 1;
            self.expect(TokenKind::Dot)?;
        }

        let mut segments = Vec::new();
        let first = self.expect_ident()?;
        segments.push(first.0);

        while self.match_token(&TokenKind::Dot) {
            let next = self.expect_ident()?;
            segments.push(next.0);
        }

        let end_span = self.peek().span;
        Ok(ModulePath {
            segments,
            is_relative,
            relative_depth,
            span: start_span.merge(&end_span),
        })
    }

    fn parse_import_decl(&mut self, is_pub: bool) -> Result<ImportDecl, (String, Span)> {
        let import_tok = self.expect(TokenKind::Import)?;
        if self.check(&TokenKind::Native) {
            return Err(("there is no `import native`".into(), self.peek().span));
        }

        let mut symbols = Vec::new();
        if self.match_token(&TokenKind::OpenBrace) {
            while !self.check(&TokenKind::CloseBrace) && !self.is_at_end() {
                let name = self.expect_ident()?;
                let mut alias = None;
                if self.match_token(&TokenKind::As) {
                    let a = self.expect_ident()?;
                    alias = Some(a.0);
                }
                symbols.push(ImportSymbol {
                    name: name.0,
                    alias,
                    span: name.1,
                });
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect(TokenKind::CloseBrace)?;
            if !self.match_token(&TokenKind::From) && !self.match_token(&TokenKind::In) {
                return Err(("Expected 'from' after import symbol list".into(), self.peek().span));
            }
        }

        let path = self.parse_module_path()?;
        let mut from_path = None;
        if self.match_token(&TokenKind::From) {
            from_path = Some(self.parse_module_path()?);
        }

        let mut alias = None;
        if self.match_token(&TokenKind::As) {
            let a = self.expect_ident()?;
            alias = Some(a.0);
        }

        let span = import_tok.span.merge(&path.span);
        Ok(ImportDecl {
            path,
            from_path,
            alias,
            symbols,
            is_pub,
            span,
        })
    }

    /// A type; `|` joins members loosest, and after `is`/`as` a union needs parentheses.
    pub(crate) fn parse_type(&mut self) -> Result<TypeNode, (String, Span)> {
        let first = self.parse_type_member()?;
        if self.operand_type || !self.check(&TokenKind::Pipe) {
            return Ok(first);
        }
        let mut members = vec![first];
        while self.match_token(&TokenKind::Pipe) {
            members.push(self.parse_type_member()?);
        }
        let span = members[0].span().merge(&members[members.len() - 1].span());
        Ok(TypeNode::Union(members, span))
    }

    fn parse_nested_type(&mut self) -> Result<TypeNode, (String, Span)> {
        let prev = std::mem::replace(&mut self.operand_type, false);
        let ty = self.parse_type();
        self.operand_type = prev;
        ty
    }

    fn parse_type_member(&mut self) -> Result<TypeNode, (String, Span)> {
        let start_span = self.peek().span;
        let is_send_marker = matches!(&self.peek().kind, TokenKind::Ident(n) if n == "Send")
            && matches!(self.peek_at(1).map(|t| &t.kind), Some(TokenKind::OpenParen));
        if is_send_marker {
            self.advance();
            return match self.parse_type_member()? {
                TypeNode::Function(params, ret, _, span) => Ok(TypeNode::Function(params, ret, true, start_span.merge(&span))),
                other => Err(("`Send` marks a function type, like `Send (Int) -> Int`".into(), other.span())),
            };
        }
        let mut ty = if self.match_token(&TokenKind::OpenParen) {
            let mut types = Vec::new();
            let mut trailing_comma = false;
            if !self.check(&TokenKind::CloseParen) {
                loop {
                    if self.check(&TokenKind::Var) {
                        let v = self.advance();
                        let inner = self.parse_nested_type()?;
                        let span = v.span.merge(&inner.span());
                        types.push(TypeNode::VarParam(Box::new(inner), span));
                    } else {
                        types.push(self.parse_nested_type()?);
                    }
                    if !self.match_token(&TokenKind::Comma) {
                        break;
                    }
                    if self.check(&TokenKind::CloseParen) {
                        trailing_comma = true;
                        break;
                    }
                }
            }
            let close = self.expect(TokenKind::CloseParen)?;
            if !self.check(&TokenKind::Arrow)
                && let Some(v) = types.iter().find(|t| matches!(t, TypeNode::VarParam(..))) {
                    return Err(("`var` marks a parameter of a function type".into(), v.span()));
                }
            if self.match_token(&TokenKind::Arrow) {
                let ret = self.parse_type()?;
                let span = start_span.merge(&ret.span());
                TypeNode::Function(types, Box::new(ret), false, span)
            } else if types.len() == 1 && !trailing_comma {
                types.pop().unwrap()
            } else {
                TypeNode::Tuple(types, start_span.merge(&close.span))
            }
        } else if self.match_token(&TokenKind::OpenBracket) {
            let elem = self.parse_nested_type()?;
            let size = if self.match_token(&TokenKind::Semicolon) {
                Some(Box::new(self.parse_expr(0)?))
            } else {
                None
            };
            let close = self.expect(TokenKind::CloseBracket)?;
            TypeNode::Array(Box::new(elem), size, start_span.merge(&close.span))
        } else if self.match_token(&TokenKind::SelfUpper) {
            TypeNode::SelfType(start_span)
        } else {
            let tok = self.advance();
            match tok.kind {
                TokenKind::Ident(ref name) => match name.as_str() {
                    "Int" => TypeNode::Int(tok.span),
                    "Float" => TypeNode::Float(tok.span),
                    "Bool" => TypeNode::Bool(tok.span),
                    "Char" => TypeNode::Char(tok.span),
                    "String" => TypeNode::String(tok.span),
                    "Null" => TypeNode::Null(tok.span),
                    first => {
                        let mut qualified = first.to_string();
                        while self.check(&TokenKind::Dot)
                            && matches!(self.peek_at(1).map(|t| &t.kind), Some(TokenKind::Ident(_)))
                        {
                            self.advance();
                            if let TokenKind::Ident(member) = self.advance().kind {
                                qualified = format!("{qualified}.{member}");
                            }
                        }
                        let other = qualified.as_str();
                        if self.match_token(&TokenKind::Lt) {
                            let mut args = Vec::new();
                            loop {
                                args.push(self.parse_nested_type()?);
                                if !self.match_token(&TokenKind::Comma) {
                                    break;
                                }
                            }
                            let close = self.expect_generic_close()?;
                            TypeNode::Generic(other.to_string(), args, tok.span.merge(&close.span))
                        } else {
                            TypeNode::Named(other.to_string(), tok.span)
                        }
                    }
                },
                _ => return Err((format!("Expected type, found {:?}", tok.kind), tok.span)),
            }
        };

        loop {
            if self.check(&TokenKind::QuestionQuestion) && self.ends_type_at(1) {
                self.advance();
                let span = ty.span().merge(&self.peek().span);
                ty = TypeNode::Nullable(Box::new(TypeNode::Nullable(Box::new(ty), span)), span);
                continue;
            }
            if !self.check(&TokenKind::Question) {
                break;
            }
            let mut is_ternary = false;
            let suffix = !self.operand_type || self.ends_type_at(1);
            let mut offset = 1;
            let mut depth = 0;
            while let Some(tok) = self.peek_at(offset).filter(|_| !suffix) {
                match tok.kind {
                    TokenKind::OpenParen | TokenKind::OpenBracket | TokenKind::OpenBrace => depth += 1,
                    TokenKind::CloseParen | TokenKind::CloseBracket | TokenKind::CloseBrace => {
                        if depth == 0 { break; }
                        depth -= 1;
                    }
                    TokenKind::Colon if depth == 0 => {
                        is_ternary = true;
                        break;
                    }
                    TokenKind::Newline | TokenKind::Semicolon | TokenKind::Eof | TokenKind::Eq => break,
                    _ => {}
                }
                offset += 1;
            }
            if is_ternary {
                break;
            }
            self.advance();
            let span = ty.span().merge(&self.peek().span);
            ty = TypeNode::Nullable(Box::new(ty), span);
        }

        Ok(ty)
    }

    fn ends_type_at(&self, offset: usize) -> bool {
        self.peek_at(offset).is_none_or(|t| {
            matches!(
                t.kind,
                TokenKind::Eq | TokenKind::Comma | TokenKind::CloseParen | TokenKind::CloseBracket | TokenKind::Gt | TokenKind::Shr
                    | TokenKind::OpenBrace | TokenKind::CloseBrace | TokenKind::Newline | TokenKind::Semicolon | TokenKind::Eof
                    | TokenKind::Question | TokenKind::QuestionQuestion | TokenKind::Pipe | TokenKind::FatArrow | TokenKind::If
            )
        })
    }

}

fn plain_string_parts(parts: &[StrPart]) -> Option<String> {
    let mut out = String::new();
    for p in parts {
        match p {
            StrPart::Lit(s) => out.push_str(s),
            StrPart::Hole(..) => return None,
        }
    }
    Some(out)
}

