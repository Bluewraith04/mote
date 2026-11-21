use mote_base::Id;

use super::FnCx;
use crate::cx::Cx;
use crate::tree::{BinaryOp, Block, Condition, Conversion, Else, Expr, ExprKind, IfStmt, Local, Stmt, StmtKind, UnaryOp};
use crate::types::{TyId, TyKind};

impl Cx<'_> {
    /// The local an expression reads, when its type may narrow: a `let` name or parameter.
    fn narrowable(f: &FnCx, expr: Id<Expr>) -> Option<Id<Local>> {
        match &f.body.exprs[expr].kind {
            ExprKind::Convert { conversion: Conversion::Narrow, expr } => Self::narrowable(f, *expr),
            ExprKind::Local(local) if !f.body.locals[*local].is_var => Some(*local),
            _ => None,
        }
    }

    /// The type left once `removed` is ruled out, or `None` when nothing is left or it does not apply.
    fn without(&mut self, ty: TyId, removed: TyId) -> Option<TyId> {
        match self.program.types.kind(ty).clone() {
            TyKind::Optional(inner) if removed == TyId::UNIT => Some(inner),
            TyKind::Union(members) => {
                let gone: Vec<TyId> = match self.program.types.kind(removed) {
                    TyKind::Union(gone) => gone.clone(),
                    _ => vec![removed],
                };
                let rest: Vec<TyId> = members.into_iter().filter(|member| !gone.contains(member)).collect();
                (!rest.is_empty()).then(|| self.program.types.union(&rest))
            }
            _ => None,
        }
    }

    /// The names a condition narrows when it turns out `outcome`.
    pub(crate) fn facts(&mut self, f: &FnCx, condition: Id<Expr>, outcome: bool) -> Vec<(Id<Local>, TyId)> {
        match f.body.exprs[condition].kind.clone() {
            ExprKind::Unary { op: UnaryOp::Not, operand } => self.facts(f, operand, !outcome),
            ExprKind::Binary { op: BinaryOp::And, left, right } if outcome => {
                let mut facts = self.facts(f, left, true);
                facts.extend(self.facts(f, right, true));
                facts
            }
            ExprKind::Binary { op: BinaryOp::Or, left, right } if !outcome => {
                let mut facts = self.facts(f, left, false);
                facts.extend(self.facts(f, right, false));
                facts
            }
            ExprKind::Binary { op: op @ (BinaryOp::Eq | BinaryOp::NotEq), left, right } => {
                if (op == BinaryOp::NotEq) != outcome {
                    return Vec::new();
                }
                let subject = match (&f.body.exprs[left].kind, &f.body.exprs[right].kind) {
                    (_, ExprKind::Null) => left,
                    (ExprKind::Null, _) => right,
                    _ => return Vec::new(),
                };
                let Some(local) = Self::narrowable(f, subject) else { return Vec::new() };
                match self.without(f.body.exprs[subject].ty, TyId::UNIT) {
                    Some(ty) => vec![(local, ty)],
                    None => Vec::new(),
                }
            }
            ExprKind::Is { expr, ty } => {
                let Some(local) = Self::narrowable(f, expr) else { return Vec::new() };
                if outcome {
                    return vec![(local, ty)];
                }
                match self.without(f.body.exprs[expr].ty, ty) {
                    Some(rest) => vec![(local, rest)],
                    None => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    /// Narrows names in the innermost scope.
    pub(crate) fn assume(&mut self, f: &mut FnCx, condition: &Condition, outcome: bool) {
        if let Condition::Expr(condition) = condition {
            for (local, ty) in self.facts(f, *condition, outcome) {
                f.narrow(local, ty);
            }
        }
    }

    /// What holds after a statement that finished: a guard that leaves its block rules its condition out.
    pub(crate) fn after_facts(&mut self, f: &FnCx, stmt: Id<Stmt>) -> Vec<(Id<Local>, TyId)> {
        let StmtKind::If(branch) = f.body.stmts[stmt].kind.clone() else { return Vec::new() };
        self.after_if(f, &branch)
    }

    fn after_if(&mut self, f: &FnCx, branch: &IfStmt) -> Vec<(Id<Local>, TyId)> {
        let Condition::Expr(condition) = branch.condition else { return Vec::new() };
        let leaves = |block: &Block| Self::block_diverges(&f.body, block, true);
        let then_leaves = leaves(&branch.then);
        match &branch.otherwise {
            None if then_leaves => self.facts(f, condition, false),
            Some(Else::Block(block)) => match (then_leaves, leaves(block)) {
                (true, false) => self.facts(f, condition, false),
                (false, true) => self.facts(f, condition, true),
                _ => Vec::new(),
            },
            Some(Else::If(other)) if then_leaves => {
                let mut facts = self.facts(f, condition, false);
                facts.extend(self.after_facts(f, *other));
                facts
            }
            _ => Vec::new(),
        }
    }
}
