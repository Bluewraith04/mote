use mote_base::{Id, Span};

use super::{is_stmt_keyword, starts_expr, Head, Parser};
use crate::ast::*;
use crate::token::TokenKind as T;

impl Parser<'_> {
    pub(super) fn stmt_node(&mut self, kind: StmtKind, start: u32) -> Id<Stmt> {
        let span = self.span_from(start);
        self.ast.stmts.alloc(Stmt { kind, span })
    }

    pub(super) fn block(&mut self) -> Block {
        self.skip_newlines();
        let start = self.start();
        if !self.expect(T::OpenBrace) {
            return Block { stmts: Vec::new(), span: Span::new(start, start) };
        }
        let stmts = self.free(|p| p.stmts_until_close());
        self.expect(T::CloseBrace);
        Block { stmts, span: self.span_from(start) }
    }

    fn stmts_until_close(&mut self) -> Vec<Id<Stmt>> {
        let mut stmts = Vec::new();
        loop {
            self.skip_separators();
            if self.at(T::CloseBrace) || self.at(T::Eof) {
                return stmts;
            }
            let (failures, pos) = (self.failures, self.pos);
            stmts.push(self.stmt());
            let stuck = self.pos == pos;
            let lost = self.failures != failures && !is_stmt_start(self.peek()) && !self.at(T::CloseBrace);
            if stuck || lost {
                self.recover_stmt();
                if self.pos == pos {
                    self.bump();
                }
            }
        }
    }

    pub(super) fn stmt(&mut self) -> Id<Stmt> {
        let start = self.start();
        if !self.enter() {
            self.leave();
            return self.stmt_node(StmtKind::Error, start);
        }
        let kind = match self.peek() {
            T::Let | T::Var => StmtKind::Binding(self.binding()),
            T::Const => StmtKind::Const(self.const_decl()),
            T::Comptime => self.comptime_stmt(),
            T::If => StmtKind::If(self.if_parts(false)),
            T::While => {
                self.bump();
                let condition = self.condition();
                StmtKind::While { condition, body: self.block() }
            }
            T::For => {
                self.bump();
                let pattern = self.binding_pattern();
                self.expect(T::In);
                let iterable = self.head(Head::Condition, |p| p.expr());
                StmtKind::For { pattern, iterable, body: self.block() }
            }
            T::Match => {
                self.bump();
                let subject = self.head(Head::Subject, |p| p.expr());
                let arms = if self.open_brace() { self.free(|p| p.items_until(T::CloseBrace, Self::match_arm)) } else { Vec::new() };
                StmtKind::Match(Match { subject, arms })
            }
            T::Scope => {
                self.bump();
                StmtKind::Scope(self.block())
            }
            T::With => {
                self.bump();
                let name = self.name();
                self.expect(T::Eq);
                let init = self.head(Head::Condition, |p| p.expr());
                StmtKind::With { name, init, body: self.block() }
            }
            T::Spawn => {
                self.bump();
                StmtKind::Spawn(self.spawn_body(true))
            }
            T::Return => {
                self.bump();
                StmtKind::Return(self.optional_value())
            }
            T::Break => {
                self.bump();
                StmtKind::Break(self.optional_value())
            }
            T::Continue => {
                self.bump();
                StmtKind::Continue
            }
            T::Yield => {
                self.bump();
                StmtKind::Yield(self.expr())
            }
            T::Fn => StmtKind::Function(self.function()),
            kind if starts_expr(kind) => self.expr_or_assign(),
            _ => {
                self.unexpected("a statement");
                StmtKind::Error
            }
        };
        self.leave();
        self.stmt_node(kind, start)
    }

    fn binding(&mut self) -> Binding {
        let kind = if self.eat(T::Let) {
            BindingKind::Let
        } else {
            self.bump();
            BindingKind::Var
        };
        let pattern = self.binding_pattern();
        let ty = if self.eat(T::Colon) { Some(self.ty()) } else { None };
        let init = if self.expect(T::Eq) { self.expr() } else { self.error_expr() };
        Binding { kind, pattern, ty, init }
    }

    pub(super) fn expr_or_assign(&mut self) -> StmtKind {
        let target = self.expr();
        let op = match self.peek() {
            T::Eq => AssignOp::Assign,
            T::PlusEq => AssignOp::Add,
            T::MinusEq => AssignOp::Sub,
            T::StarEq => AssignOp::Mul,
            T::SlashEq => AssignOp::Div,
            T::PercentEq => AssignOp::Rem,
            T::AmpEq => AssignOp::BitAnd,
            T::PipeEq => AssignOp::BitOr,
            T::CaretEq => AssignOp::BitXor,
            T::ShlEq => AssignOp::Shl,
            T::ShrEq => AssignOp::Shr,
            T::NullCoalesceEq => AssignOp::Coalesce,
            _ => return StmtKind::Expr(target),
        };
        self.bump();
        StmtKind::Assign { target, op, value: self.expr() }
    }

    pub(super) fn condition(&mut self) -> Condition {
        if self.eat(T::Let) {
            let pattern = self.pattern();
            self.expect(T::Eq);
            let value = self.head(Head::Condition, |p| p.expr());
            return Condition::Let { pattern, value };
        }
        Condition::Expr(self.head(Head::Condition, |p| p.expr()))
    }

    fn if_parts(&mut self, comptime: bool) -> If {
        if !self.enter() {
            self.leave();
            return If { condition: Condition::Expr(self.error_expr()), then: Block { stmts: Vec::new(), span: self.span() }, otherwise: None };
        }
        self.bump();
        let condition = self.condition();
        let then = self.block();
        let otherwise = if self.else_follows() {
            self.bump();
            let next_comptime = comptime && self.eat(T::Comptime);
            if self.at(T::If) {
                let start = self.start();
                let is_comptime = comptime || next_comptime;
                let parts = self.if_parts(is_comptime);
                let kind = if is_comptime { StmtKind::ComptimeIf(parts) } else { StmtKind::If(parts) };
                Some(Else::If(self.stmt_node(kind, start)))
            } else {
                Some(Else::Block(self.block()))
            }
        } else {
            None
        };
        self.leave();
        If { condition, then, otherwise }
    }

    fn comptime_stmt(&mut self) -> StmtKind {
        self.bump();
        match self.peek() {
            T::If => StmtKind::ComptimeIf(self.if_parts(true)),
            T::For => {
                self.bump();
                let pattern = self.binding_pattern();
                self.expect(T::In);
                let iterable = self.head(Head::Condition, |p| p.expr());
                StmtKind::ComptimeFor { pattern, iterable, body: self.block() }
            }
            T::OpenBrace => StmtKind::ComptimeBlock(self.block()),
            _ => {
                self.unexpected("`if`, `for`, or `{`");
                StmtKind::Error
            }
        }
    }

    pub(super) fn else_follows(&mut self) -> bool {
        if self.at(T::Newline) && self.peek_at(1) == T::Else {
            self.bump();
        }
        self.at(T::Else)
    }

    fn match_arm(&mut self) -> MatchArm {
        let start = self.start();
        let pattern = self.pattern();
        let guard = if self.eat(T::If) { Some(self.expr()) } else { None };
        self.expect(T::FatArrow);
        let body = if self.at(T::OpenBrace) { ArmBody::Block(self.block()) } else { ArmBody::Stmt(self.stmt()) };
        MatchArm { pattern, guard, body, span: self.span_from(start) }
    }
}

pub(super) fn is_stmt_start(kind: T) -> bool {
    is_stmt_keyword(kind) || starts_expr(kind)
}
