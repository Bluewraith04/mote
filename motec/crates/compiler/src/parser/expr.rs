//! Expression parsing (Pratt: binding powers, prefix and infix).

use super::*;

impl Parser {
    pub(crate) fn parse_expr(&mut self, rbp: u8) -> Result<Expr, (String, Span)> {
        let tok = self.advance();
        let mut left = self.nud(tok)?;

        while rbp < self.lbp(self.peek_kind()) {
            let op_tok = self.advance();
            left = self.led(left, op_tok)?;
        }

        Ok(left)
    }

    fn parse_nested_expr(&mut self) -> Result<Expr, (String, Span)> {
        let prev = std::mem::replace(&mut self.match_subject, false);
        let e = self.parse_expr(0);
        self.match_subject = prev;
        e
    }

    fn parse_call_arg(&mut self) -> Result<Expr, (String, Span)> {
        if self.check(&TokenKind::Var) {
            let v = self.advance();
            return Err(("an argument takes no `var`; remove it".into(), v.span));
        }
        self.parse_nested_expr()
    }

    fn type_args_follow(&self, offset: usize) -> bool {
        if !matches!(self.peek_at(offset).map(|t| &t.kind), Some(TokenKind::Lt)) {
            return false;
        }
        let mut depth = 0i32;
        let mut i = offset;
        while let Some(tok) = self.peek_at(i) {
            depth += match tok.kind {
                TokenKind::Lt => 1,
                TokenKind::Gt => -1,
                TokenKind::Shr => -2,
                TokenKind::Ident(_) | TokenKind::SelfUpper | TokenKind::Comma | TokenKind::Dot | TokenKind::Question
                | TokenKind::QuestionQuestion | TokenKind::OpenParen | TokenKind::CloseParen | TokenKind::OpenBracket
                | TokenKind::CloseBracket | TokenKind::Arrow | TokenKind::Pipe => 0,
                _ => return false,
            };
            if depth < 0 {
                return false;
            }
            i += 1;
            if depth == 0 {
                return matches!(self.peek_at(i).map(|t| &t.kind), Some(TokenKind::OpenParen | TokenKind::Dot));
            }
        }
        false
    }

    fn parse_type_applied(&mut self, name: String, start: Span) -> Result<Expr, (String, Span)> {
        self.expect(TokenKind::Lt)?;
        let mut args = Vec::new();
        loop {
            args.push(self.parse_nested_type()?);
            if !self.match_token(&TokenKind::Comma) {
                break;
            }
        }
        let close = self.expect_generic_close()?;
        let target = TypeNode::Generic(name, args, start.merge(&close.span));
        if self.match_token(&TokenKind::Dot) {
            let (member, m_span) = self.expect_member_name()?;
            return Ok(Expr::StaticAccess { target, member, span: start.merge(&m_span) });
        }
        Ok(Expr::StaticAccess { span: target.span(), target, member: "new".to_string() })
    }

    pub(crate) fn parse_operand_type(&mut self) -> Result<TypeNode, (String, Span)> {
        let prev = std::mem::replace(&mut self.operand_type, true);
        let ty = self.parse_type();
        self.operand_type = prev;
        ty
    }

    pub(crate) fn lbp(&self, kind: &TokenKind) -> u8 {
        match kind {
            TokenKind::Question => 20,
            TokenKind::QuestionQuestion => 25,
            TokenKind::OrOr => 30,
            TokenKind::AndAnd => 35,
            TokenKind::Pipe => 40,
            TokenKind::Caret => 45,
            TokenKind::Ampersand => 50,
            TokenKind::EqEq | TokenKind::NotEq => 55,
            TokenKind::Lt | TokenKind::LtEq | TokenKind::Gt | TokenKind::GtEq => 60,
            TokenKind::Is => 65,
            TokenKind::As => 70,
            TokenKind::Shl | TokenKind::Shr => 75,
            TokenKind::Plus | TokenKind::Minus => 80,
            TokenKind::Star | TokenKind::Slash | TokenKind::Percent => 85,
            TokenKind::DotDot | TokenKind::DotDotEq => 95,
            TokenKind::Dot
            | TokenKind::QuestionDot
            | TokenKind::ColonColon
            | TokenKind::OpenParen
            | TokenKind::OpenBracket
            | TokenKind::Bang => 100,
            _ => 0,
        }
    }

    pub(crate) fn nud(&mut self, tok: Token) -> Result<Expr, (String, Span)> {
        match tok.kind {
            TokenKind::Int(v) => Ok(Expr::Int(v, tok.span)),
            TokenKind::Float(v) => Ok(Expr::Float(v, tok.span)),
            TokenKind::True => Ok(Expr::Bool(true, tok.span)),
            TokenKind::False => Ok(Expr::Bool(false, tok.span)),
            TokenKind::String(parts) => self.interpolated_string_expr(parts, tok.span),
            TokenKind::Char(v) => Ok(Expr::Char(v, tok.span)),
            TokenKind::Null => Ok(Expr::Null(tok.span)),
            TokenKind::Spawn => self.parse_spawn_expr(tok.span),
            TokenKind::SelfLower => Ok(Expr::SelfValue(tok.span)),
            TokenKind::Ident(ref name) => {
                let mut name = name.clone();
                let mut segs = 0;
                while segs < 2
                    && matches!(self.peek_at(2 * segs).map(|t| &t.kind), Some(TokenKind::Dot))
                    && matches!(self.peek_at(2 * segs + 1).map(|t| &t.kind), Some(TokenKind::Ident(m)) if m.starts_with(char::is_uppercase))
                {
                    segs += 1;
                }
                if segs > 0
                    && matches!(self.peek_at(2 * segs).map(|t| &t.kind), Some(TokenKind::OpenBrace))
                    && !matches!(self.peek_at(2 * segs + 1).map(|t| &t.kind), Some(TokenKind::CloseBrace))
                    && self.looks_like_struct_init(2 * segs)
                {
                    for _ in 0..segs {
                        self.advance();
                        if let TokenKind::Ident(member) = self.advance().kind {
                            name = format!("{name}.{member}");
                        }
                    }
                    segs = 0;
                }
                let mut dotted = segs;
                while dotted < 2
                    && matches!(self.peek_at(2 * dotted).map(|t| &t.kind), Some(TokenKind::Dot))
                    && matches!(self.peek_at(2 * dotted + 1).map(|t| &t.kind), Some(TokenKind::Ident(_)))
                {
                    dotted += 1;
                }
                let qual = (0..=dotted).rev().find(|&k| self.type_args_follow(2 * k));
                if let Some(k) = qual {
                    for _ in 0..k {
                        self.advance();
                        if let TokenKind::Ident(member) = self.advance().kind {
                            name = format!("{name}.{member}");
                        }
                    }
                    return self.parse_type_applied(name, tok.span);
                }
                if self.check(&TokenKind::OpenBrace) && self.looks_like_struct_init(0) {
                    self.advance();
                    self.skip_newlines();
                    let mut fields = Vec::new();
                    while !self.check(&TokenKind::CloseBrace) && !self.is_at_end() {
                        let f_name = self.expect_ident()?;
                        self.expect(TokenKind::Colon)?;
                        let val = self.parse_expr(0)?;
                        fields.push((f_name.0, val));
                        if !self.match_token(&TokenKind::Comma) {
                            self.skip_newlines();
                        }
                    }
                    let close_brace = self.expect(TokenKind::CloseBrace)?;
                    let span = tok.span.merge(&close_brace.span);
                    return Ok(Expr::StructInit {
                        name: name.clone(),
                        target_type: None,
                        fields,
                        span,
                    });
                }
                Ok(Expr::Ident(name.clone(), tok.span))
            }
            TokenKind::Minus => {
                let inner = self.parse_expr(90)?;
                let span = tok.span.merge(&inner.span());
                Ok(Expr::Unary {
                    op: UnaryOp::Neg,
                    expr: Box::new(inner),
                    span,
                })
            }
            TokenKind::Bang => {
                let inner = self.parse_expr(90)?;
                let span = tok.span.merge(&inner.span());
                Ok(Expr::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(inner),
                    span,
                })
            }
            TokenKind::Tilde => {
                let inner = self.parse_expr(90)?;
                let span = tok.span.merge(&inner.span());
                Ok(Expr::Unary {
                    op: UnaryOp::BitNot,
                    expr: Box::new(inner),
                    span,
                })
            }
            TokenKind::Try => {
                let inner = self.parse_expr(90)?;
                let span = tok.span.merge(&inner.span());
                Ok(Expr::Try {
                    expr: Box::new(inner),
                    span,
                })
            }
            TokenKind::OpenParen => {
                if self.check(&TokenKind::CloseParen) {
                    let close = self.advance();
                    return Ok(Expr::TupleLiteral {
                        elements: Vec::new(),
                        span: tok.span.merge(&close.span),
                    });
                }
                let first = self.parse_nested_expr()?;
                if self.match_token(&TokenKind::Comma) {
                    let mut elements = vec![first];
                    while !self.check(&TokenKind::CloseParen) && !self.is_at_end() {
                        elements.push(self.parse_nested_expr()?);
                        if !self.match_token(&TokenKind::Comma) { break; }
                    }
                    let close = self.expect(TokenKind::CloseParen)?;
                    Ok(Expr::TupleLiteral {
                        elements,
                        span: tok.span.merge(&close.span),
                    })
                } else {
                    self.expect(TokenKind::CloseParen)?;
                    Ok(first)
                }
            }
            TokenKind::OpenBracket => {
                let mut elements = Vec::new();
                if !self.check(&TokenKind::CloseBracket) {
                    loop {
                        elements.push(self.parse_nested_expr()?);
                        if !self.match_token(&TokenKind::Comma) || self.check(&TokenKind::CloseBracket) {
                            break;
                        }
                    }
                }
                let close_bracket = self.expect(TokenKind::CloseBracket)?;
                let span = tok.span.merge(&close_bracket.span);
                Ok(Expr::ListLiteral { elements, span })
            }
            TokenKind::Pipe | TokenKind::OrOr => {
                let mut params = Vec::new();
                if tok.kind == TokenKind::Pipe && !self.check(&TokenKind::Pipe) {
                    loop {
                        let is_mut = self.match_token(&TokenKind::Var);
                        if self.check(&TokenKind::Mut) {
                            let m_tok = self.advance();
                            let (name, n_span) = self.expect_ident()?;
                            return Err((format!("write `{name}`, not `mut {name}`"), m_tok.span.merge(&n_span)));
                        }
                        let p_name = self.expect_ident()?;
                        let mut ty = None;
                        if self.match_token(&TokenKind::Colon) {
                            ty = Some(self.parse_operand_type()?);
                        }
                        params.push(Param {
                            name: p_name.0.clone(),
                            ty: ty.clone(),
                            kind: Some(ParameterKind::Regular {
                                name: p_name.0,
                                ty,
                                default_value: None,
                                is_mut,
                            }),
                            is_mut,
                            span: p_name.1,
                        });
                        if !self.match_token(&TokenKind::Comma) {
                            break;
                        }
                    }
                    self.expect(TokenKind::Pipe)?;
                }

                let mut ret_ty = None;
                if self.match_token(&TokenKind::Arrow) {
                    ret_ty = Some(self.parse_type()?);
                }

                let body = if self.check(&TokenKind::OpenBrace) {
                    self.parse_block()?
                } else {
                    let e = self.parse_expr(0)?;
                    vec![Stmt::Expr { span: e.span(), expr: e }]
                };

                let span = tok.span.merge(self.tokens.get(self.pos - 1).map(|t| &t.span).unwrap_or(&tok.span));
                Ok(Expr::Lambda {
                    params,
                    return_type: ret_ty,
                    body,
                    span,
                })
            }
            TokenKind::OpenBrace => {
                let mut entries = Vec::new();
                if !self.check(&TokenKind::CloseBrace) {
                    loop {
                        let key = self.parse_expr(0)?;
                        self.expect(TokenKind::Colon)?;
                        let val = self.parse_expr(0)?;
                        entries.push((key, val));
                        if !self.match_token(&TokenKind::Comma) || self.check(&TokenKind::CloseBrace) {
                            break;
                        }
                    }
                }
                let close_brace = self.expect(TokenKind::CloseBrace)?;
                let span = tok.span.merge(&close_brace.span);
                Ok(Expr::MapLiteral { entries, span })
            }
            other => Err((format!("Unexpected token in expression: {:?}", other), tok.span)),
        }
    }

    fn braces_hold_arms(&self, open: usize) -> bool {
        let mut depth = 0usize;
        let mut idx = open;
        while let Some(tok) = self.peek_at(idx) {
            match tok.kind {
                TokenKind::OpenBrace | TokenKind::OpenParen | TokenKind::OpenBracket => depth += 1,
                TokenKind::CloseBrace | TokenKind::CloseParen | TokenKind::CloseBracket => {
                    depth -= 1;
                    if depth == 0 {
                        return false;
                    }
                }
                TokenKind::FatArrow if depth == 1 => return true,
                TokenKind::Eof => return false,
                _ => {}
            }
            idx += 1;
        }
        false
    }

    pub(crate) fn looks_like_struct_init(&self, start: usize) -> bool {
        let mut idx = start;
        while let Some(tok) = self.peek_at(idx) {
            if tok.kind == TokenKind::Newline {
                idx += 1;
                continue;
            }
            if tok.kind == TokenKind::OpenBrace {
                if self.match_subject && self.braces_hold_arms(idx) {
                    return false;
                }
                let mut inner_idx = idx + 1;
                while let Some(inner_tok) = self.peek_at(inner_idx) {
                    if inner_tok.kind == TokenKind::Newline {
                        inner_idx += 1;
                        continue;
                    }
                    if inner_tok.kind == TokenKind::CloseBrace {
                        return true;
                    }
                    if let TokenKind::Ident(_) = &inner_tok.kind {
                        let mut colon_idx = inner_idx + 1;
                        while let Some(colon_tok) = self.peek_at(colon_idx) {
                            if colon_tok.kind == TokenKind::Newline {
                                colon_idx += 1;
                                continue;
                            }
                            return matches!(colon_tok.kind, TokenKind::Colon);
                        }
                    }
                    break;
                }
            }
            break;
        }
        false
    }

    pub(crate) fn led(&mut self, left: Expr, op_tok: Token) -> Result<Expr, (String, Span)> {
        match op_tok.kind {
            TokenKind::Plus => self.binary_expr(left, BinaryOp::Add, 80, op_tok.span),
            TokenKind::Minus => self.binary_expr(left, BinaryOp::Sub, 80, op_tok.span),
            TokenKind::Star => self.binary_expr(left, BinaryOp::Mul, 85, op_tok.span),
            TokenKind::Slash => self.binary_expr(left, BinaryOp::Div, 85, op_tok.span),
            TokenKind::Percent => self.binary_expr(left, BinaryOp::Mod, 85, op_tok.span),
            TokenKind::Ampersand => self.binary_expr(left, BinaryOp::BitAnd, 50, op_tok.span),
            TokenKind::Pipe => self.binary_expr(left, BinaryOp::BitOr, 40, op_tok.span),
            TokenKind::Caret => self.binary_expr(left, BinaryOp::BitXor, 45, op_tok.span),
            TokenKind::Shl => self.binary_expr(left, BinaryOp::Shl, 75, op_tok.span),
            TokenKind::Shr => self.binary_expr(left, BinaryOp::Shr, 75, op_tok.span),
            TokenKind::EqEq => self.binary_expr(left, BinaryOp::Eq, 55, op_tok.span),
            TokenKind::NotEq => self.binary_expr(left, BinaryOp::NotEq, 55, op_tok.span),
            TokenKind::Lt => self.binary_expr(left, BinaryOp::Lt, 60, op_tok.span),
            TokenKind::LtEq => self.binary_expr(left, BinaryOp::LtEq, 60, op_tok.span),
            TokenKind::Gt => self.binary_expr(left, BinaryOp::Gt, 60, op_tok.span),
            TokenKind::GtEq => self.binary_expr(left, BinaryOp::GtEq, 60, op_tok.span),
            TokenKind::AndAnd => self.binary_expr(left, BinaryOp::And, 35, op_tok.span),
            TokenKind::OrOr => self.binary_expr(left, BinaryOp::Or, 30, op_tok.span),
            TokenKind::QuestionQuestion => {
                let right = self.parse_expr(26)?;
                let span = left.span().merge(&right.span());
                Ok(Expr::NullCoalesce {
                    left: Box::new(left),
                    right: Box::new(right),
                    span,
                })
            }
            TokenKind::Question => {
                if self.ternary_colon_ahead() {
                    let then_expr = self.parse_expr(0)?;
                    self.expect(TokenKind::Colon)?;
                    let else_expr = self.parse_expr(19)?;
                    let span = left.span().merge(&else_expr.span());
                    Ok(Expr::Ternary {
                        cond: Box::new(left),
                        then_expr: Box::new(then_expr),
                        else_expr: Box::new(else_expr),
                        span,
                    })
                } else {
                    let span = left.span().merge(&op_tok.span);
                    Ok(Expr::Try { expr: Box::new(left), span })
                }
            }
            TokenKind::As => Err((
                "`as` only renames an import. To convert, write `T.from(e)`. To check a type, use `is`, a `match` type pattern, or a typed `let`".into(),
                op_tok.span,
            )),
            TokenKind::Is => {
                let target_type = self.parse_operand_type()?;
                let span = left.span().merge(&target_type.span());
                Ok(Expr::TypeTest {
                    expr: Box::new(left),
                    target_type,
                    span,
                })
            }
            TokenKind::Dot => {
                let (member, m_span) = if let TokenKind::Int(n) = self.peek_kind() {
                    let n = *n;
                    let tok = self.advance();
                    (n.to_string(), tok.span)
                } else {
                    self.expect_member_name()?
                };
                let span = left.span().merge(&m_span);
                Ok(Expr::MemberAccess {
                    object: Box::new(left),
                    member,
                    span,
                })
            }
            TokenKind::QuestionDot => {
                let (member, m_span) = self.expect_member_name()?;
                let temp = format!("?.{}", op_tok.span.start);
                let body = Expr::MemberAccess {
                    object: Box::new(Expr::Ident(temp.clone(), op_tok.span)),
                    member,
                    span: op_tok.span.merge(&m_span),
                };
                let span = left.span().merge(&m_span);
                Ok(Expr::OptionalChain { object: Box::new(left), temp, body: Box::new(body), span })
            }
            TokenKind::ColonColon => {
                let (member, m_span) = self.expect_ident()?;
                let msg = match &left {
                    Expr::Ident(s, _) => format!("write `{s}.{member}`, not `::`"),
                    _ => format!("write `.{member}`, not `::`"),
                };
                Err((msg, op_tok.span.merge(&m_span)))
            }
            TokenKind::OpenParen => {
                let mut args = Vec::new();
                if !self.check(&TokenKind::CloseParen) {
                    loop {
                        args.push(self.parse_call_arg()?);
                        if !self.match_token(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                let close = self.expect(TokenKind::CloseParen)?;
                let span = left.span().merge(&close.span);
                if let Expr::OptionalChain { object, temp, body, .. } = left {
                    if let Expr::MemberAccess { .. } = *body {
                        let call_span = body.span().merge(&close.span);
                        let body = Expr::Call { callee: body, args, span: call_span };
                        return Ok(Expr::OptionalChain { object, temp, body: Box::new(body), span });
                    }
                    let chain_span = object.span().merge(&body.span());
                    let left = Expr::OptionalChain { object, temp, body, span: chain_span };
                    return Ok(Expr::Call { callee: Box::new(left), args, span });
                }
                Ok(Expr::Call {
                    callee: Box::new(left),
                    args,
                    span,
                })
            }
            TokenKind::OpenBracket => {
                let index = self.parse_nested_expr()?;
                let close = self.expect(TokenKind::CloseBracket)?;
                let span = left.span().merge(&close.span);
                Ok(Expr::Index {
                    object: Box::new(left),
                    index: Box::new(index),
                    span,
                })
            }
            TokenKind::DotDot => {
                let right = self.parse_expr(96)?;
                let span = left.span().merge(&right.span());
                Ok(Expr::Range {
                    start: Some(Box::new(left)),
                    end: Some(Box::new(right)),
                    inclusive: false,
                    span,
                })
            }
            TokenKind::DotDotEq => {
                let right = self.parse_expr(96)?;
                let span = left.span().merge(&right.span());
                Ok(Expr::Range {
                    start: Some(Box::new(left)),
                    end: Some(Box::new(right)),
                    inclusive: true,
                    span,
                })
            }
            TokenKind::Bang => {
                let span = left.span().merge(&op_tok.span);
                Ok(Expr::Unwrap {
                    expr: Box::new(left),
                    span,
                })
            }
            _ => Err((format!("Unexpected operator {:?}", op_tok.kind), op_tok.span)),
        }
    }

    pub(crate) fn binary_expr(&mut self, left: Expr, op: BinaryOp, rbp: u8, _op_span: Span) -> Result<Expr, (String, Span)> {
        let right = self.parse_expr(rbp)?;
        let span = left.span().merge(&right.span());
        Ok(Expr::Binary {
            left: Box::new(left),
            op,
            right: Box::new(right),
            span,
        })
    }

    pub(crate) fn interpolated_string_expr(
        &mut self,
        parts: Vec<StrPart>,
        span: Span,
    ) -> Result<Expr, (String, Span)> {
        if let [StrPart::Lit(s)] = parts.as_slice() {
            return Ok(Expr::String(s.clone(), span));
        }
        let mut acc: Option<Expr> = None;
        for part in &parts {
            let piece = match part {
                StrPart::Lit(s) => {
                    if s.is_empty() {
                        continue;
                    }
                    Expr::String(s.clone(), span)
                }
                StrPart::Hole(src, pos) => {
                    let inner = self.parse_hole(src, *pos, span)?;
                    Expr::Call {
                        callee: Box::new(Expr::MemberAccess {
                            object: Box::new(inner),
                            member: "to_string".to_string(),
                            span,
                        }),
                        args: Vec::new(),
                        span,
                    }
                }
            };
            acc = Some(match acc {
                None => piece,
                Some(left) => Expr::Binary {
                    left: Box::new(left),
                    op: BinaryOp::Add,
                    right: Box::new(piece),
                    span,
                },
            });
        }
        Ok(acc.unwrap_or_else(|| Expr::String(String::new(), span)))
    }

    pub(crate) fn parse_hole(&self, src: &str, pos: HolePos, span: Span) -> Result<Expr, (String, Span)> {
        let mut lexer = crate::lexer::Lexer::new(src);
        let mut toks = lexer
            .tokenize()
            .map_err(|(m, _)| (format!("in `${{…}}` interpolation: {m}"), span))?;
        let first_col = if pos.lines == 0 { span.col + pos.at } else { pos.col };
        for t in &mut toks {
            let s = t.span;
            let line = span.line + pos.lines + s.line - 1;
            let col = if s.line == 1 { first_col + s.col - 1 } else { s.col };
            t.span = Span::new(span.start + pos.at + s.start, span.start + pos.at + s.end, line, col)
                .with_source(span.source);
        }
        let mut parser = Parser::new(toks);
        let expr = parser
            .parse_expr(0)
            .map_err(|(m, _)| (format!("in `${{…}}` interpolation: {m}"), span))?;
        parser.skip_newlines();
        if !parser.is_at_end() {
            return Err((
                format!("unexpected trailing tokens in `${{{}}}` interpolation", src.trim()),
                span,
            ));
        }
        Ok(expr)
    }
}
