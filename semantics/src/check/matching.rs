use mote_base::{Id, Span};
use mote_syntax as ast;

use super::coerce::plan;
use super::FnCx;
use crate::cx::Cx;
use crate::tree::{Arm, ArmBody, Block, Condition, Expr, ExprKind, Match, Pattern, StmtKind};
use crate::types::TyId;

impl Cx<'_> {
    /// An `if` or `while` condition; a `let` pattern binds its names in the current scope.
    pub(crate) fn condition(&mut self, f: &mut FnCx, condition: &ast::Condition) -> Condition {
        match condition {
            ast::Condition::Expr(condition) => Condition::Expr(self.expr_to(f, *condition, TyId::BOOL)),
            ast::Condition::Let { pattern, value } => {
                let value = self.expr(f, *value, None);
                let ty = f.body.exprs[value].ty;
                let pattern = self.arm_pattern(f, *pattern, ty);
                Condition::Let { pattern, value }
            }
        }
    }

    /// One arm's pattern and guard, in a scope the caller closes.
    fn arm_head(&mut self, f: &mut FnCx, pattern: Id<ast::Pattern>, guard: Option<Id<ast::Expr>>, ty: TyId) -> (Id<Pattern>, Option<Id<Expr>>) {
        let pattern = self.arm_pattern(f, pattern, ty);
        let guard = guard.map(|guard| self.expr_to(f, guard, TyId::BOOL));
        (pattern, guard)
    }

    pub(crate) fn match_stmt(&mut self, f: &mut FnCx, matched: &ast::Match, span: Span) -> StmtKind {
        let subject = self.expr(f, matched.subject, None);
        let ty = f.body.exprs[subject].ty;
        let mut arms = Vec::new();
        let mut covering = Vec::new();
        for arm in &matched.arms {
            f.push_scope();
            let (pattern, guard) = self.arm_head(f, arm.pattern, arm.guard, ty);
            let body = match &arm.body {
                ast::ArmBody::Block(block) => ArmBody::Block(self.block(f, block)),
                ast::ArmBody::Stmt(stmt) => {
                    let stmt = self.stmt(f, *stmt);
                    ArmBody::Block(Block { stmts: vec![stmt], span: arm.span })
                }
            };
            f.pop_scope();
            if guard.is_none() {
                covering.push(pattern);
            }
            arms.push(Arm { pattern, guard, body, span: arm.span });
        }
        self.check_covered(f, ty, &covering, span);
        StmtKind::Match(Match { subject, arms })
    }

    pub(crate) fn value_match(&mut self, f: &mut FnCx, subject: Id<ast::Expr>, arms: &[ast::ValueArm], expected: Option<TyId>, span: Span) -> Id<Expr> {
        let subject = self.expr(f, subject, None);
        let ty = f.body.exprs[subject].ty;
        let mut heads = Vec::new();
        let mut values = Vec::new();
        let mut covering = Vec::new();
        let mut hint = expected;
        for arm in arms {
            f.push_scope();
            let (pattern, guard) = self.arm_head(f, arm.pattern, arm.guard, ty);
            let value = self.expr(f, arm.value, hint);
            f.pop_scope();
            hint = hint.or(Some(f.body.exprs[value].ty));
            if guard.is_none() {
                covering.push(pattern);
            }
            heads.push((pattern, guard, arm.span));
            values.push(value);
        }
        self.check_covered(f, ty, &covering, span);
        let (values, result) = self.join_all(f, &values, span, "branches");
        let arms = heads.into_iter().zip(values).map(|((pattern, guard, span), value)| Arm { pattern, guard, body: ArmBody::Value(value), span }).collect();
        f.body.expr(ExprKind::Match(Match { subject, arms }), result, span)
    }

    /// The type every branch, element or key shares: each fits the widest, or a `null` makes it optional.
    pub(crate) fn join_all(&mut self, f: &mut FnCx, nodes: &[Id<Expr>], span: Span, what: &str) -> (Vec<Id<Expr>>, TyId) {
        let types: Vec<TyId> = nodes.iter().map(|&node| f.body.exprs[node].ty).collect();
        if types.contains(&TyId::ERROR) || types.is_empty() {
            return (nodes.to_vec(), TyId::ERROR);
        }
        let mut target = types[0];
        for &ty in &types[1..] {
            target = if plan(self.program, ty, target).is_some() {
                target
            } else if plan(self.program, target, ty).is_some() {
                ty
            } else if ty == TyId::UNIT {
                self.program.types.optional(target)
            } else if target == TyId::UNIT {
                self.program.types.optional(ty)
            } else {
                let message = format!("{what} have different types: {} and {}", self.show_ty(target), self.show_ty(ty));
                self.error(span, message);
                return (nodes.to_vec(), TyId::ERROR);
            };
        }
        let joined = nodes.iter().map(|&node| self.coerce(f, node, target, span)).collect();
        (joined, target)
    }
}
