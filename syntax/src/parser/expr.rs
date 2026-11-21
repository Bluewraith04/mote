use mote_base::Id;

use super::{starts_expr, Ctx, Head, Parser, FREE};
use crate::ast::*;
use crate::token::TokenKind as T;

fn binary_op(kind: T) -> Option<(u8, BinaryOp)> {
    Some(match kind {
        T::QuestionQuestion => (1, BinaryOp::Coalesce),
        T::OrOr => (2, BinaryOp::Or),
        T::AndAnd => (3, BinaryOp::And),
        T::Pipe => (4, BinaryOp::BitOr),
        T::Caret => (5, BinaryOp::BitXor),
        T::Ampersand => (6, BinaryOp::BitAnd),
        T::EqEq => (7, BinaryOp::Eq),
        T::NotEq => (7, BinaryOp::NotEq),
        T::Lt => (8, BinaryOp::Lt),
        T::LtEq => (8, BinaryOp::LtEq),
        T::Gt => (8, BinaryOp::Gt),
        T::GtEq => (8, BinaryOp::GtEq),
        T::Shl => (10, BinaryOp::Shl),
        T::Shr => (10, BinaryOp::Shr),
        T::Plus => (12, BinaryOp::Add),
        T::Minus => (12, BinaryOp::Sub),
        T::Star => (13, BinaryOp::Mul),
        T::Slash => (13, BinaryOp::Div),
        T::Percent => (13, BinaryOp::Rem),
        _ => return None,
    })
}

impl Parser<'_> {
    pub(super) fn expr_node(&mut self, kind: ExprKind, start: u32) -> Id<Expr> {
        let span = self.span_from(start);
        self.ast.exprs.alloc(Expr { kind, span })
    }

    pub(super) fn expr(&mut self) -> Id<Expr> {
        self.expr_bp(0)
    }

    fn expr_bp(&mut self, min: u8) -> Id<Expr> {
        if !self.enter() {
            self.leave();
            return self.error_expr();
        }
        let mut left = self.unary();
        loop {
            if matches!(self.ast.exprs[left].kind, ExprKind::Error) {
                break;
            }
            let kind = self.peek();
            let start = self.ast.exprs[left].span.start;
            match kind {
                T::Is if min <= 9 => {
                    self.bump();
                    let ty = self.ty();
                    left = self.expr_node(ExprKind::TypeTest { expr: left, ty }, start);
                }
                T::As if min <= 14 => {
                    self.bump();
                    let ty = self.ty();
                    left = self.expr_node(ExprKind::Cast { expr: left, ty }, start);
                }
                T::DotDot | T::DotDotEq if min <= 11 && !self.ctx.no_range => {
                    let inclusive = self.bump().kind == T::DotDotEq;
                    let end = self.expr_bp(12);
                    left = self.expr_node(ExprKind::Range { start: left, end, inclusive }, start);
                    if matches!(self.peek(), T::DotDot | T::DotDotEq) {
                        let span = self.span();
                        self.error("ranges do not chain", span);
                    }
                }
                _ => {
                    if self.ctx.no_gt && matches!(kind, T::Gt | T::GtEq | T::Shr) {
                        break;
                    }
                    let Some((prec, op)) = binary_op(kind) else { break };
                    if prec < min {
                        break;
                    }
                    self.bump();
                    let right = self.expr_bp(if op == BinaryOp::Coalesce { prec } else { prec + 1 });
                    left = self.expr_node(ExprKind::Binary { op, left, right }, start);
                    if (prec == 7 || prec == 8) && binary_op(self.peek()).is_some_and(|(next, _)| next == prec) {
                        let span = self.span();
                        self.error("comparisons do not chain", span);
                    }
                }
            }
        }
        self.leave();
        left
    }

    fn unary(&mut self) -> Id<Expr> {
        let op = match self.peek() {
            T::Minus => UnaryOp::Neg,
            T::Bang => UnaryOp::Not,
            T::Tilde => UnaryOp::BitNot,
            _ => return self.postfix(),
        };
        if !self.enter() {
            self.leave();
            return self.error_expr();
        }
        let start = self.start();
        self.bump();
        let operand = self.unary();
        self.leave();
        self.expr_node(ExprKind::Unary { op, operand }, start)
    }

    fn postfix(&mut self) -> Id<Expr> {
        let mut expr = self.primary();
        let start = self.ast.exprs[expr].span.start;
        loop {
            if matches!(self.ast.exprs[expr].kind, ExprKind::Error) {
                break;
            }
            let kind = match self.peek() {
                T::OpenParen if !self.starts_line() => {
                    self.bump();
                    let args = self.list(T::CloseParen, |p| p.arg());
                    ExprKind::Call { callee: expr, args }
                }
                T::OpenBracket => self.index(expr),
                T::Dot => {
                    self.bump();
                    if let T::Int(index) = self.peek() {
                        self.bump();
                        ExprKind::TupleIndex { object: expr, index }
                    } else {
                        let name = self.member_name();
                        let args = self.member_type_args();
                        ExprKind::Member { object: expr, name, args }
                    }
                }
                T::QuestionDot => {
                    self.bump();
                    let name = self.member_name();
                    let args = self.member_type_args();
                    ExprKind::OptionalMember { object: expr, name, args }
                }
                T::Bang => {
                    self.bump();
                    ExprKind::Unwrap(expr)
                }
                T::Question => {
                    self.bump();
                    ExprKind::Propagate(expr)
                }
                _ => break,
            };
            expr = self.expr_node(kind, start);
        }
        expr
    }

    pub(super) fn arg(&mut self) -> Arg {
        if matches!(self.peek(), T::Ident(_)) && self.peek_at(1) == T::Eq {
            let name = self.name();
            self.bump();
            return Arg { name: Some(name), value: self.expr() };
        }
        Arg { name: None, value: self.expr() }
    }

    fn member_type_args(&mut self) -> Vec<GenericArg> {
        if self.at(T::Lt) && self.type_args_follow(0) {
            self.generic_args()
        } else {
            Vec::new()
        }
    }

    fn index(&mut self, object: Id<Expr>) -> ExprKind {
        self.bump();
        let ctx = Ctx { no_range: true, ..FREE };
        let (first, range) = self.with_ctx(ctx, |p| {
            let first = if matches!(p.peek(), T::DotDot | T::DotDotEq) { None } else { Some(p.expr()) };
            if !matches!(p.peek(), T::DotDot | T::DotDotEq) {
                return (first, None);
            }
            let inclusive = p.bump().kind == T::DotDotEq;
            let end = if matches!(p.peek(), T::CloseBracket | T::DotDot) { None } else { Some(p.expr()) };
            let step = if p.eat(T::DotDot) { Some(p.expr()) } else { None };
            (first, Some((inclusive, end, step)))
        });
        self.expect(T::CloseBracket);
        match (first, range) {
            (start, Some((inclusive, end, step))) => ExprKind::Slice { object, start, end, inclusive, step },
            (Some(index), None) => ExprKind::Index { object, index },
            (None, None) => ExprKind::Error,
        }
    }

    fn primary(&mut self) -> Id<Expr> {
        let start = self.start();
        let kind = match self.peek() {
            T::Int(literal) => {
                self.bump();
                ExprKind::Int(literal)
            }
            T::Float(literal) => {
                self.bump();
                ExprKind::Float(literal)
            }
            T::String(text) => {
                self.bump();
                ExprKind::String(text)
            }
            T::True | T::False => ExprKind::Bool(self.bump().kind == T::True),
            T::Null => {
                self.bump();
                ExprKind::Null
            }
            T::SelfLower => {
                self.bump();
                ExprKind::SelfValue
            }
            T::InterpStart(_) => return self.interpolated(),
            T::Ident(_) | T::SelfUpper => return self.name_expr(),
            T::OpenParen => return self.paren_expr(),
            T::OpenBracket => return self.list_expr(),
            T::OpenBrace => return self.map_expr(),
            T::Pipe | T::OrOr => return self.lambda(),
            T::If => return self.if_expr(),
            T::Match => return self.match_expr(),
            T::At => return self.builtin_expr(),
            T::Comptime => return self.comptime_expr(),
            T::Spawn => {
                self.bump();
                ExprKind::Spawn(self.spawn_body(false))
            }
            T::Error => {
                self.bump();
                self.failures += 1;
                ExprKind::Error
            }
            _ => {
                self.unexpected("an expression");
                ExprKind::Error
            }
        };
        self.expr_node(kind, start)
    }

    pub(super) fn spawn_body(&mut self, full: bool) -> SpawnBody {
        if self.at(T::OpenBrace) {
            return SpawnBody::Block(self.block());
        }
        SpawnBody::Expr(if full { self.expr() } else { self.unary() })
    }

    fn interpolated(&mut self) -> Id<Expr> {
        let start = self.start();
        let T::InterpStart(head) = self.bump().kind else { unreachable!() };
        let mut parts = Vec::new();
        loop {
            let value = if matches!(self.peek(), T::InterpMid(_) | T::InterpEnd(_)) {
                self.failures += 1;
                self.error_expr()
            } else {
                self.free(|p| p.expr())
            };
            match self.peek() {
                T::InterpMid(text) => {
                    self.bump();
                    parts.push(InterpPart { value, text });
                }
                T::InterpEnd(text) => {
                    self.bump();
                    parts.push(InterpPart { value, text });
                    break;
                }
                _ => {
                    self.unexpected("`}`");
                    let text = self.skip_hole();
                    parts.push(InterpPart { value, text: text.0 });
                    if text.1 {
                        break;
                    }
                }
            }
        }
        self.expr_node(ExprKind::Interpolated { head, parts }, start)
    }

    fn skip_hole(&mut self) -> (mote_base::Symbol, bool) {
        let mut depth = 0u32;
        loop {
            match self.peek() {
                T::InterpStart(_) => depth += 1,
                T::InterpMid(text) if depth == 0 => {
                    self.bump();
                    return (text, false);
                }
                T::InterpEnd(text) if depth == 0 => {
                    self.bump();
                    return (text, true);
                }
                T::InterpEnd(_) => depth -= 1,
                T::Eof => return (self.words.wildcard, true),
                _ => {}
            }
            self.bump();
        }
    }

    fn name_expr(&mut self) -> Id<Expr> {
        let start = self.start();
        let mut segments = 1;
        while self.peek_at(2 * segments - 1) == T::Dot && matches!(self.peek_at(2 * segments), T::Ident(_)) {
            segments += 1;
        }
        let after = 2 * segments - 1;
        match self.peek_at(after) {
            T::Lt if self.type_args_follow(after) => {
                let path = self.path();
                let args = self.generic_args();
                if self.at(T::OpenBrace) {
                    return self.struct_init(path, args, start);
                }
                self.expr_node(ExprKind::TypeApply { path, args }, start)
            }
            T::OpenBrace if self.struct_init_follows(after) => {
                let path = self.path();
                self.struct_init(path, Vec::new(), start)
            }
            _ => {
                let kind = match self.bump().kind {
                    T::Ident(name) => ExprKind::Ident(name),
                    _ => ExprKind::Ident(self.words.self_upper),
                };
                self.expr_node(kind, start)
            }
        }
    }

    fn builtin_expr(&mut self) -> Id<Expr> {
        let start = self.start();
        self.bump();
        let name = self.name();
        let generic_args = if self.at(T::Lt) && self.type_args_follow(0) { self.generic_args() } else { Vec::new() };
        let args = if self.expect(T::OpenParen) { self.list(T::CloseParen, |p| p.arg()) } else { Vec::new() };
        self.expr_node(ExprKind::Builtin { name, generic_args, args }, start)
    }

    fn struct_init(&mut self, path: Path, args: Vec<GenericArg>, start: u32) -> Id<Expr> {
        self.bump();
        let fields = self.free(|p| p.items_until(T::CloseBrace, Self::field_init));
        self.expr_node(ExprKind::StructInit { path, args, fields }, start)
    }

    fn field_init(&mut self) -> FieldInit {
        if self.eat(T::DotDot) {
            return FieldInit::Spread(self.expr());
        }
        let name = self.name();
        if self.eat(T::Colon) { FieldInit::Named { name, value: self.expr() } } else { FieldInit::Shorthand(name) }
    }

    pub(super) fn type_args_follow(&self, offset: usize) -> bool {
        let mut depth = 0i32;
        let mut group = 0i32;
        let mut i = offset;
        loop {
            let tok = self.peek_at(i);
            match tok {
                T::OpenParen | T::OpenBracket => group += 1,
                T::CloseParen | T::CloseBracket => {
                    if group == 0 {
                        return false;
                    }
                    group -= 1;
                }
                _ if group > 0 => {
                    if tok == T::Eof {
                        return false;
                    }
                }
                T::Lt => depth += 1,
                T::Gt => depth -= 1,
                T::Shr => depth -= 2,
                T::Ident(_)
                | T::SelfUpper
                | T::Comma
                | T::Dot
                | T::Question
                | T::QuestionQuestion
                | T::Arrow
                | T::Pipe
                | T::Var
                | T::Plus
                | T::Minus
                | T::Star
                | T::Slash
                | T::Percent
                | T::Ampersand
                | T::Caret
                | T::Shl
                | T::Bang
                | T::Tilde
                | T::True
                | T::False
                | T::Null
                | T::Int(_)
                | T::Float(_)
                | T::String(_)
                | T::At => {}
                _ => return false,
            }
            if depth < 0 {
                return false;
            }
            i += 1;
            if depth == 0 && group == 0 {
                return matches!(self.peek_at(i), T::OpenParen | T::Dot | T::CloseParen | T::Comma | T::Question | T::Pipe) || (self.peek_at(i) == T::OpenBrace && self.struct_init_follows(i));
            }
        }
    }

    fn struct_init_follows(&self, open: usize) -> bool {
        let restricted = self.ctx.head != Head::Free;
        match (self.peek_at(open + 1), self.peek_at(open + 2)) {
            (T::CloseBrace, _) => !restricted,
            (T::DotDot, _) => true,
            (T::Ident(_), T::Colon) => !(self.ctx.head == Head::Subject && self.braces_hold_arms(open)),
            (T::Ident(_), T::Comma) => true,
            (T::Ident(_), T::CloseBrace) => !restricted,
            _ => false,
        }
    }

    fn braces_hold_arms(&self, open: usize) -> bool {
        let mut depth = 0u32;
        let mut i = open;
        loop {
            match self.peek_at(i) {
                T::OpenBrace | T::OpenParen | T::OpenBracket => depth += 1,
                T::CloseBrace | T::CloseParen | T::CloseBracket => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return false;
                    }
                }
                T::FatArrow if depth == 1 => return true,
                T::Eof => return false,
                _ => {}
            }
            i += 1;
        }
    }

    fn paren_expr(&mut self) -> Id<Expr> {
        let start = self.start();
        self.bump();
        let (mut elements, trailing) = self.list_flagged(T::CloseParen, |p| p.expr());
        let kind = if elements.len() == 1 && !trailing { ExprKind::Paren(elements.remove(0)) } else { ExprKind::Tuple(elements) };
        self.expr_node(kind, start)
    }

    fn repeat_follows(&self) -> bool {
        let mut depth = 0u32;
        let mut i = 1;
        loop {
            match self.peek_at(i) {
                T::Semicolon if depth == 0 => return true,
                T::OpenBrace | T::OpenParen | T::OpenBracket => depth += 1,
                T::CloseBrace | T::CloseParen | T::CloseBracket => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                }
                T::Eof => return false,
                _ => {}
            }
            i += 1;
        }
    }

    fn list_expr(&mut self) -> Id<Expr> {
        let start = self.start();
        let repeat = self.repeat_follows();
        self.bump();
        if repeat {
            let (value, len) = self.free(|p| {
                let value = p.expr();
                p.expect(T::Semicolon);
                (value, p.expr())
            });
            self.expect(T::CloseBracket);
            return self.expr_node(ExprKind::ArrayRepeat { value, len }, start);
        }
        let elements = self.list(T::CloseBracket, |p| p.expr());
        self.expr_node(ExprKind::List(elements), start)
    }

    fn map_expr(&mut self) -> Id<Expr> {
        let start = self.start();
        self.bump();
        let entries = self.list(T::CloseBrace, |p| {
            let key = p.expr();
            p.expect(T::Colon);
            MapEntry { key, value: p.expr() }
        });
        self.expr_node(ExprKind::Map(entries), start)
    }

    fn lambda(&mut self) -> Id<Expr> {
        let start = self.start();
        let header = Ctx { no_union: true, ..FREE };
        let params = if self.eat(T::OrOr) {
            Vec::new()
        } else {
            self.bump();
            self.list(T::Pipe, |p| {
                let is_var = p.eat(T::Var);
                let name = p.name();
                let ty = if p.eat(T::Colon) { Some(p.with_ctx(header, |p| p.ty())) } else { None };
                LambdaParam { is_var, name, ty }
            })
        };
        let ret = if self.eat(T::Arrow) { Some(self.with_ctx(header, |p| p.ty())) } else { None };
        let body = if self.at(T::OpenBrace) { LambdaBody::Block(self.block()) } else { self.lambda_value() };
        self.expr_node(ExprKind::Lambda { params, ret, body }, start)
    }

    /// A lambda body without braces: an expression, or an assignment read as a block of one statement.
    fn lambda_value(&mut self) -> LambdaBody {
        let start = self.start();
        match self.expr_or_assign() {
            StmtKind::Expr(value) => LambdaBody::Expr(value),
            kind => {
                let stmt = self.stmt_node(kind, start);
                LambdaBody::Block(Block { stmts: vec![stmt], span: self.span_from(start) })
            }
        }
    }

    fn if_expr(&mut self) -> Id<Expr> {
        self.if_expr_inner(false, self.start())
    }

    fn comptime_expr(&mut self) -> Id<Expr> {
        let start = self.start();
        self.bump();
        match self.peek() {
            T::If => self.if_expr_inner(true, start),
            T::OpenBrace => {
                let block = self.block();
                self.expr_node(ExprKind::Comptime(block), start)
            }
            _ => {
                self.unexpected("`if` or `{`");
                self.error_expr()
            }
        }
    }

    fn if_expr_inner(&mut self, comptime: bool, start: u32) -> Id<Expr> {
        if !self.enter() {
            self.leave();
            return self.error_expr();
        }
        self.bump();
        let condition = self.condition();
        let then = self.branch();
        let otherwise = if self.else_follows() {
            self.bump();
            let next_comptime = comptime && self.eat(T::Comptime);
            if self.at(T::If) {
                ElseExpr::If(self.if_expr_inner(comptime || next_comptime, self.start()))
            } else {
                ElseExpr::Value(self.branch())
            }
        } else {
            let span = self.span();
            self.error("a value `if` needs an `else`", span);
            ElseExpr::Value(self.error_expr())
        };
        self.leave();
        let kind = if comptime {
            ExprKind::ComptimeIf { condition, then, otherwise }
        } else {
            ExprKind::If { condition, then, otherwise }
        };
        self.expr_node(kind, start)
    }

    fn branch(&mut self) -> Id<Expr> {
        self.skip_newlines();
        if !self.expect(T::OpenBrace) {
            return self.error_expr();
        }
        let value = self.free(|p| p.expr());
        if !self.at(T::CloseBrace) {
            let span = self.span();
            self.error("a branch holds one expression", span);
            self.skip_in_list(T::CloseBrace);
        }
        self.expect(T::CloseBrace);
        value
    }

    fn match_expr(&mut self) -> Id<Expr> {
        let start = self.start();
        self.bump();
        let subject = self.head(Head::Subject, |p| p.expr());
        let arms = if self.open_brace() { self.free(|p| p.items_until(T::CloseBrace, Self::value_arm)) } else { Vec::new() };
        self.expr_node(ExprKind::Match { subject, arms }, start)
    }

    fn value_arm(&mut self) -> ValueArm {
        let start = self.start();
        let pattern = self.pattern();
        let guard = if self.eat(T::If) { Some(self.expr()) } else { None };
        self.expect(T::FatArrow);
        let value = if self.at(T::OpenBrace) && self.block_follows() {
            let span = self.span();
            self.error("an arm holds one expression", span);
            self.skip_block();
            self.error_expr()
        } else {
            self.expr()
        };
        ValueArm { pattern, guard, value, span: self.span_from(start) }
    }

    /// Whether the `{` here opens statements: it starts with a statement keyword, or holds no `:` outside its nested brackets.
    fn block_follows(&self) -> bool {
        if matches!(
            self.peek_at(1),
            T::Return | T::Let | T::Var | T::Const | T::Comptime | T::Break | T::Continue | T::Yield | T::While | T::For | T::With | T::Scope
        ) {
            return true;
        }
        let mut depth = 0u32;
        let mut i = 1;
        loop {
            match self.peek_at(i) {
                T::Colon if depth == 0 => return false,
                T::OpenBrace | T::OpenParen | T::OpenBracket => depth += 1,
                T::CloseBrace | T::CloseParen | T::CloseBracket => {
                    if depth == 0 {
                        return i > 1;
                    }
                    depth -= 1;
                }
                T::Eof => return false,
                _ => {}
            }
            i += 1;
        }
    }

    /// Skips from the `{` here to its matching `}`.
    fn skip_block(&mut self) {
        let mut depth = 0u32;
        loop {
            match self.peek() {
                T::OpenBrace => depth += 1,
                T::CloseBrace => {
                    depth -= 1;
                    if depth == 0 {
                        self.bump();
                        return;
                    }
                }
                T::Eof => return,
                _ => {}
            }
            self.bump();
        }
    }

    pub(super) fn open_brace(&mut self) -> bool {
        self.skip_newlines();
        self.expect(T::OpenBrace)
    }

    pub(super) fn optional_value(&mut self) -> Option<Id<Expr>> {
        if starts_expr(self.peek()) { Some(self.expr()) } else { None }
    }
}
