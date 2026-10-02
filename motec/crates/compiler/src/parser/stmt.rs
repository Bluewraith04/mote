//! Statements, patterns and blocks.

use super::*;

impl Parser {
    pub(crate) fn parse_block(&mut self) -> Result<Vec<Stmt>, (String, Span)> {
        self.expect(TokenKind::OpenBrace)?;
        self.skip_newlines();

        let mut stmts = Vec::new();
        while !self.check(&TokenKind::CloseBrace) && !self.is_at_end() {
            let stmt = self.parse_stmt()?;
            stmts.push(stmt);
            self.skip_newlines();
        }

        self.expect(TokenKind::CloseBrace)?;
        Ok(stmts)
    }

    pub(crate) fn parse_stmt(&mut self) -> Result<Stmt, (String, Span)> {
        let removed_arena = matches!(self.peek_kind(), TokenKind::Ident(n) if n == "arena")
            && matches!(self.peek_at(1).map(|t| &t.kind), Some(TokenKind::OpenBrace));
        if removed_arena {
            let span = self.tokens[self.pos].span;
            return Err(("`arena { }` was removed; delete it".to_string(), span));
        }
        match self.peek_kind() {
            TokenKind::Let => self.parse_let_stmt(),
            TokenKind::Var => self.parse_var_stmt(),
            TokenKind::If => self.parse_if_stmt(),
            TokenKind::While => self.parse_while_stmt(),
            TokenKind::For => self.parse_for_stmt(),
            TokenKind::Match => self.parse_match_stmt(),
            TokenKind::Scope => self.parse_scope_stmt(),
            TokenKind::With => self.parse_with_stmt(),
            TokenKind::Spawn => self.parse_spawn_stmt(),
            TokenKind::Return => self.parse_return_stmt(),
            TokenKind::Break => {
                let tok = self.advance();
                let val = if !self.is_statement_terminator() {
                    Some(self.parse_expr(0)?)
                } else {
                    None
                };
                self.consume_statement_terminator();
                Ok(Stmt::Break { value: val, span: tok.span })
            }
            TokenKind::Continue => {
                let tok = self.advance();
                self.consume_statement_terminator();
                Ok(Stmt::Continue { span: tok.span })
            }
            TokenKind::Yield => {
                let tok = self.advance();
                let value = self.parse_expr(0)?;
                self.consume_statement_terminator();
                Ok(Stmt::Yield { value, span: tok.span })
            }
            _ => self.parse_expr_or_assign_stmt(),
        }
    }

    pub(crate) fn parse_let_stmt(&mut self) -> Result<Stmt, (String, Span)> {
        let let_tok = self.expect(TokenKind::Let)?;
        if self.check(&TokenKind::OpenParen) {
            return self.parse_tuple_let(let_tok, false);
        }
        let name_tok = self.expect_ident()?;

        let mut ty = None;
        if self.match_token(&TokenKind::Colon) {
            ty = Some(self.parse_type()?);
        }

        self.expect(TokenKind::Eq)?;
        let init = self.parse_expr(0)?;
        let span = let_tok.span.merge(&init.span());
        self.consume_statement_terminator();

        Ok(Stmt::Let {
            name: name_tok.0,
            ty,
            init,
            is_pub: false,
            span,
        })
    }

    pub(crate) fn parse_var_stmt(&mut self) -> Result<Stmt, (String, Span)> {
        let var_tok = self.expect(TokenKind::Var)?;
        if self.check(&TokenKind::OpenParen) {
            return self.parse_tuple_let(var_tok, true);
        }
        let name_tok = self.expect_ident()?;

        let mut ty = None;
        if self.match_token(&TokenKind::Colon) {
            ty = Some(self.parse_type()?);
        }

        self.expect(TokenKind::Eq)?;
        let init = self.parse_expr(0)?;
        let span = var_tok.span.merge(&init.span());
        self.consume_statement_terminator();

        Ok(Stmt::Var {
            name: name_tok.0,
            ty,
            init,
            is_pub: false,
            span,
        })
    }

    fn parse_tuple_let(&mut self, start_tok: Token, is_mut: bool) -> Result<Stmt, (String, Span)> {
        self.expect(TokenKind::OpenParen)?;
        let mut names = Vec::new();
        loop {
            names.push(self.expect_ident()?.0);
            if !self.match_token(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(TokenKind::CloseParen)?;
        if names.len() < 2 {
            return Err((
                "a tuple destructuring pattern needs at least two names".into(),
                start_tok.span,
            ));
        }
        self.expect(TokenKind::Eq)?;
        let init = self.parse_expr(0)?;
        let span = start_tok.span.merge(&init.span());
        self.consume_statement_terminator();
        Ok(Stmt::TupleLet { names, is_mut, is_pub: false, init, span })
    }

    pub(crate) fn parse_if_stmt(&mut self) -> Result<Stmt, (String, Span)> {
        let if_tok = self.expect(TokenKind::If)?;
        let cond = self.parse_expr(0)?;
        self.skip_newlines();
        let then_branch = self.parse_block()?;

        self.skip_newlines();
        let else_branch = if self.match_token(&TokenKind::Else) {
            self.skip_newlines();
            if self.check(&TokenKind::If) {
                Some(vec![self.parse_if_stmt()?])
            } else {
                Some(self.parse_block()?)
            }
        } else {
            None
        };

        let span = if_tok.span.merge(self.tokens.get(self.pos - 1).map(|t| &t.span).unwrap_or(&if_tok.span));
        Ok(Stmt::If {
            cond,
            then_branch,
            else_branch,
            span,
        })
    }

    pub(crate) fn parse_while_stmt(&mut self) -> Result<Stmt, (String, Span)> {
        let while_tok = self.expect(TokenKind::While)?;
        let cond = self.parse_expr(0)?;
        self.skip_newlines();
        let body = self.parse_block()?;
        let span = while_tok.span.merge(self.tokens.get(self.pos - 1).map(|t| &t.span).unwrap_or(&while_tok.span));

        Ok(Stmt::While { cond, body, span })
    }

    pub(crate) fn parse_for_stmt(&mut self) -> Result<Stmt, (String, Span)> {
        let for_tok = self.expect(TokenKind::For)?;
        let var_name = self.expect_ident()?;
        self.expect(TokenKind::In)?;
        let iter = self.parse_expr(0)?;
        self.skip_newlines();
        let body = self.parse_block()?;
        let span = for_tok.span.merge(self.tokens.get(self.pos - 1).map(|t| &t.span).unwrap_or(&for_tok.span));

        Ok(Stmt::ForIn {
            var_name: var_name.0,
            iter,
            body,
            span,
        })
    }

    pub(crate) fn parse_match_stmt(&mut self) -> Result<Stmt, (String, Span)> {
        let match_tok = self.expect(TokenKind::Match)?;
        let prev = std::mem::replace(&mut self.match_subject, true);
        let expr = self.parse_expr(0);
        self.match_subject = prev;
        let expr = expr?;
        self.skip_newlines();
        self.expect(TokenKind::OpenBrace)?;
        self.skip_newlines();

        let mut arms = Vec::new();
        while !self.check(&TokenKind::CloseBrace) && !self.is_at_end() {
            let pattern = self.parse_pattern()?;
            let guard = if self.match_token(&TokenKind::If) {
                Some(self.parse_expr(0)?)
            } else {
                None
            };
            self.expect(TokenKind::FatArrow)?;
            self.skip_newlines();
            let body = if self.check(&TokenKind::OpenBrace) {
                self.parse_block()?
            } else {
                let e = self.parse_stmt()?;
                vec![e]
            };
            let span = pattern.span().merge(self.tokens.get(self.pos - 1).map(|t| &t.span).unwrap_or(&pattern.span()));
            arms.push(MatchArm { pattern, guard, body, span });

            if !self.match_token(&TokenKind::Comma) {
                self.skip_newlines();
                if self.check(&TokenKind::CloseBrace) { break; }
            }
            self.skip_newlines();
        }
        let close = self.expect(TokenKind::CloseBrace)?;

        Ok(Stmt::Match {
            expr,
            arms,
            span: match_tok.span.merge(&close.span),
        })
    }

    pub(crate) fn parse_pattern(&mut self) -> Result<Pattern, (String, Span)> {
        let first = self.parse_range_pattern()?;
        if !self.check(&TokenKind::Pipe) {
            return Ok(first);
        }
        let mut alts = vec![first];
        while self.match_token(&TokenKind::Pipe) {
            alts.push(self.parse_range_pattern()?);
        }
        let span = alts[0].span().merge(&alts[alts.len() - 1].span());
        Ok(Pattern::Or(alts, span))
    }

    pub(crate) fn parse_range_pattern(&mut self) -> Result<Pattern, (String, Span)> {
        let start = self.parse_primary_pattern()?;
        let inclusive = if self.check(&TokenKind::DotDotEq) {
            self.advance();
            true
        } else if self.check(&TokenKind::DotDot) {
            self.advance();
            false
        } else {
            return Ok(start);
        };
        let end = self.parse_primary_pattern()?;
        let span = start.span().merge(&end.span());
        Ok(Pattern::Range {
            start: Box::new(start),
            end: Box::new(end),
            inclusive,
            span,
        })
    }

    pub(crate) fn parse_primary_pattern(&mut self) -> Result<Pattern, (String, Span)> {
        let tok = self.advance();
        match tok.kind {
            TokenKind::Ident(ref name) if name == "_" && self.check(&TokenKind::Colon) => self.parse_type_pattern(None, tok.span),
            TokenKind::Ident(ref name) if name == "_" => Ok(Pattern::Wildcard(tok.span)),
            TokenKind::Int(v) => Ok(Pattern::Literal(Box::new(Expr::Int(v, tok.span)), tok.span)),
            TokenKind::Minus => {
                let n = self.advance();
                if let TokenKind::Int(v) = n.kind {
                    let span = tok.span.merge(&n.span);
                    Ok(Pattern::Literal(Box::new(Expr::Int(-v, span)), span))
                } else {
                    Err(("expected an integer after `-` in a pattern".into(), n.span))
                }
            }
            TokenKind::String(ref parts) => {
                let s = plain_string_parts(parts).ok_or_else(|| {
                    ("string interpolation `${…}` is not allowed in a pattern".to_string(), tok.span)
                })?;
                Ok(Pattern::Literal(Box::new(Expr::String(s, tok.span)), tok.span))
            }
            TokenKind::True => Ok(Pattern::Literal(Box::new(Expr::Bool(true, tok.span)), tok.span)),
            TokenKind::False => Ok(Pattern::Literal(Box::new(Expr::Bool(false, tok.span)), tok.span)),
            TokenKind::Null => Ok(Pattern::Literal(Box::new(Expr::Null(tok.span)), tok.span)),
            TokenKind::Ident(ref name) => {
                let mut head = name.clone();
                let mut variant: Option<String> = None;
                let mut end_span = tok.span;
                while self.check(&TokenKind::Dot) {
                    self.advance();
                    let (seg, seg_span) = self.expect_ident()?;
                    if let Some(prev) = variant.replace(seg) {
                        head = format!("{head}.{prev}");
                    }
                    end_span = seg_span;
                }

                if self.check(&TokenKind::OpenParen) {
                    self.advance();
                    let mut elems = Vec::new();
                    if !self.check(&TokenKind::CloseParen) {
                        loop {
                            elems.push(self.parse_pattern()?);
                            if !self.match_token(&TokenKind::Comma) { break; }
                        }
                    }
                    let close = self.expect(TokenKind::CloseParen)?;
                    return Ok(Pattern::Enum {
                        target: TypeNode::Named(head, tok.span),
                        variant,
                        payload: EnumPatternPayload::Tuple(elems),
                        span: tok.span.merge(&close.span),
                    });
                }

                if self.check(&TokenKind::OpenBrace) {
                    self.advance();
                    self.skip_newlines();
                    let mut fields = Vec::new();
                    let mut has_rest = false;
                    while !self.check(&TokenKind::CloseBrace) && !self.is_at_end() {
                        if self.match_token(&TokenKind::DotDot) {
                            has_rest = true;
                            self.skip_newlines();
                            break;
                        }
                        let (f_name, f_span) = self.expect_ident()?;
                        let sub = if self.match_token(&TokenKind::Colon) {
                            Some(self.parse_pattern()?)
                        } else {
                            None
                        };
                        fields.push(FieldPattern { name: f_name, pattern: sub, span: f_span });
                        self.match_token(&TokenKind::Comma);
                        self.skip_newlines();
                    }
                    let close = self.expect(TokenKind::CloseBrace)?;
                    return Ok(Pattern::Enum {
                        target: TypeNode::Named(head, tok.span),
                        variant,
                        payload: EnumPatternPayload::Struct { fields, has_rest },
                        span: tok.span.merge(&close.span),
                    });
                }

                if variant.is_some() {
                    return Ok(Pattern::Enum {
                        target: TypeNode::Named(head, tok.span),
                        variant,
                        payload: EnumPatternPayload::None,
                        span: tok.span.merge(&end_span),
                    });
                }

                if variant.is_none() && self.check(&TokenKind::Colon) {
                    return self.parse_type_pattern(Some(head), tok.span);
                }

                if self.match_token(&TokenKind::At) {
                    let sub = self.parse_pattern()?;
                    let span = tok.span.merge(&sub.span());
                    Ok(Pattern::Identifier {
                        name: head,
                        is_mut: false,
                        subpattern: Some(Box::new(sub)),
                        span,
                    })
                } else {
                    Ok(Pattern::Identifier {
                        name: head,
                        is_mut: false,
                        subpattern: None,
                        span: tok.span,
                    })
                }
            }
            TokenKind::OpenParen => {
                let mut elements = Vec::new();
                if !self.check(&TokenKind::CloseParen) {
                    loop {
                        elements.push(self.parse_pattern()?);
                        if !self.match_token(&TokenKind::Comma) { break; }
                    }
                }
                let close = self.expect(TokenKind::CloseParen)?;
                Ok(Pattern::Tuple(elements, tok.span.merge(&close.span)))
            }
            _ => Err((format!("Invalid pattern token {:?}", tok.kind), tok.span)),
        }
    }

    fn parse_type_pattern(&mut self, name: Option<String>, start: Span) -> Result<Pattern, (String, Span)> {
        self.expect(TokenKind::Colon)?;
        let target = self.parse_operand_type()?;
        let span = start.merge(&target.span());
        Ok(Pattern::Type { name, target, span })
    }

    pub(crate) fn parse_with_stmt(&mut self) -> Result<Stmt, (String, Span)> {
        let with_tok = self.expect(TokenKind::With)?;
        let name_tok = self.expect_ident()?;
        self.expect(TokenKind::Eq)?;
        let init = self.parse_expr(0)?;
        self.skip_newlines();
        let body = self.parse_block()?;
        let span = with_tok.span.merge(self.tokens.get(self.pos - 1).map(|t| &t.span).unwrap_or(&with_tok.span));
        Ok(Stmt::WithBlock { name: name_tok.0, init, body, span })
    }

    pub(crate) fn parse_scope_stmt(&mut self) -> Result<Stmt, (String, Span)> {
        let scope_tok = self.expect(TokenKind::Scope)?;
        self.skip_newlines();
        let body = self.parse_block()?;
        let span = scope_tok.span.merge(self.tokens.get(self.pos - 1).map(|t| &t.span).unwrap_or(&scope_tok.span));
        Ok(Stmt::ScopeBlock { body, span })
    }

    pub(crate) fn parse_spawn_stmt(&mut self) -> Result<Stmt, (String, Span)> {
        let spawn_tok = self.expect(TokenKind::Spawn)?;
        let (body, span) = self.parse_spawn_body_after_keyword(spawn_tok.span)?;
        Ok(Stmt::SpawnBlock { body, span })
    }

    pub(crate) fn parse_spawn_expr(&mut self, spawn_span: Span) -> Result<Expr, (String, Span)> {
        let (body, span) = self.parse_spawn_body_after_keyword(spawn_span)?;
        Ok(Expr::Spawn { body, span })
    }

    pub(crate) fn parse_spawn_body_after_keyword(&mut self, spawn_span: Span) -> Result<(Vec<Stmt>, Span), (String, Span)> {
        self.skip_newlines();
        let body = if self.check(&TokenKind::OpenBrace) {
            self.parse_block()?
        } else {
            let expr = self.parse_expr(90)?;
            vec![Stmt::Expr { span: expr.span(), expr }]
        };
        let span = spawn_span.merge(self.tokens.get(self.pos - 1).map(|t| &t.span).unwrap_or(&spawn_span));
        Ok((body, span))
    }

    pub(crate) fn parse_return_stmt(&mut self) -> Result<Stmt, (String, Span)> {
        let ret_tok = self.expect(TokenKind::Return)?;
        let value = if !self.is_statement_terminator() {
            Some(self.parse_expr(0)?)
        } else {
            None
        };
        let span = ret_tok.span.merge(self.tokens.get(self.pos - 1).map(|t| &t.span).unwrap_or(&ret_tok.span));
        self.consume_statement_terminator();
        Ok(Stmt::Return { value, span })
    }

    pub(crate) fn parse_expr_or_assign_stmt(&mut self) -> Result<Stmt, (String, Span)> {
        let expr = self.parse_expr(0)?;

        if self.match_token(&TokenKind::Eq) {
            let value = self.parse_expr(0)?;
            let span = expr.span().merge(&value.span());
            self.consume_statement_terminator();
            Ok(Stmt::Assign {
                target: expr,
                value,
                span,
            })
        } else if let Some(op) = self.match_compound_assign_op() {
            let value = self.parse_expr(0)?;
            let span = expr.span().merge(&value.span());
            self.consume_statement_terminator();
            Ok(Stmt::CompoundAssign {
                target: expr,
                op,
                value,
                span,
            })
        } else {
            let span = expr.span();
            self.consume_statement_terminator();
            Ok(Stmt::Expr { expr, span })
        }
    }

    pub(crate) fn match_compound_assign_op(&mut self) -> Option<AssignOp> {
        let op = match self.peek_kind() {
            TokenKind::PlusEq => Some(AssignOp::AddAssign),
            TokenKind::MinusEq => Some(AssignOp::SubAssign),
            TokenKind::StarEq => Some(AssignOp::MulAssign),
            TokenKind::SlashEq => Some(AssignOp::DivAssign),
            TokenKind::PercentEq => Some(AssignOp::ModAssign),
            TokenKind::AmpEq => Some(AssignOp::BitAndAssign),
            TokenKind::PipeEq => Some(AssignOp::BitOrAssign),
            TokenKind::CaretEq => Some(AssignOp::BitXorAssign),
            TokenKind::ShlEq => Some(AssignOp::ShlAssign),
            TokenKind::ShrEq => Some(AssignOp::ShrAssign),
            TokenKind::NullCoalesceEq => Some(AssignOp::NullCoalesceAssign),
            _ => None,
        };
        if op.is_some() {
            self.advance();
        }
        op
    }

}
