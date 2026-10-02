//! Narrowing optionals by checks, and no unchecked member access.

use super::*;

#[derive(Clone)]
pub(super) struct Narrowing {
    name: String,
    depth: usize,
    ty: Type,
    unwraps: u8,
    scope: usize,
}

#[derive(Default)]
pub(super) struct Facts {
    pub when_true: Vec<Narrowing>,
    pub when_false: Vec<Narrowing>,
}

impl TypeChecker {
    pub(super) fn narrowed(&mut self, name: &str, depth: usize, span: Span) -> Option<Type> {
        let n = self.narrowings.iter().rev().find(|n| n.name == name && n.depth == depth)?;
        let ty = n.ty.clone();
        if n.unwraps > 0 {
            self.narrowed_reads.insert(span, n.unwraps);
        }
        Some(ty)
    }

    pub(super) fn assume(&mut self, facts: Vec<Narrowing>) {
        let scope = self.scopes.len();
        self.narrowings.extend(facts.into_iter().map(|n| Narrowing { scope, ..n }));
    }

    pub(super) fn drop_narrowings(&mut self) {
        let len = self.scopes.len();
        self.narrowings.retain(|n| n.scope <= len);
    }

    pub(super) fn take_rewraps(&mut self) -> HashMap<Span, i8> {
        let mut out: HashMap<Span, i8> = std::mem::take(&mut self.some_lifts).into_iter().map(|(s, n)| (s, n as i8)).collect();
        for (span, n) in std::mem::take(&mut self.narrowed_reads) {
            *out.entry(span).or_insert(0) -= n as i8;
        }
        out
    }

    fn narrowable(&self, e: &Expr) -> Option<(String, usize, Type, u8)> {
        let Expr::Ident(name, _) = e else { return None };
        let (info, depth) = self.lookup_symbol_depth(name)?;
        if info.is_mutable {
            return None;
        }
        let (ty, unwraps) = self.narrowings.iter().rev().find(|n| n.name == *name && n.depth == depth).map_or((info.ty, 0), |n| (n.ty.clone(), n.unwraps));
        Some((name.clone(), depth, ty, unwraps))
    }

    fn present(&self, e: &Expr) -> Option<Narrowing> {
        let (name, depth, ty, unwraps) = self.narrowable(e)?;
        let Type::Nullable(inner) = ty else { return None };
        let unwraps = unwraps + Self::may_be_cell(&inner) as u8;
        Some(Narrowing { name, depth, ty: *inner, unwraps, scope: 0 })
    }

    fn may_be_cell(ty: &Type) -> bool {
        match ty {
            Type::Union(_) => ty.members().iter().any(Self::may_be_cell),
            _ => matches!(ty, Type::Nullable(_) | Type::Null | Type::Param(_) | Type::Any),
        }
    }

    fn is_none_literal(&self, e: &Expr) -> bool {
        matches!(e, Expr::Null(_)) || self.option_ctor(e) == Some("None")
    }

    pub(super) fn facts(&self, cond: &Expr) -> Facts {
        match cond {
            Expr::Binary { left, op: op @ (BinaryOp::Eq | BinaryOp::NotEq), right, .. } => {
                let subject = if self.is_none_literal(right) {
                    left
                } else if self.is_none_literal(left) {
                    right
                } else {
                    return Facts::default();
                };
                let facts = self.present(subject).into_iter().collect();
                if *op == BinaryOp::NotEq {
                    Facts { when_true: facts, when_false: Vec::new() }
                } else {
                    Facts { when_true: Vec::new(), when_false: facts }
                }
            }
            Expr::TypeTest { expr, span, .. } => {
                let Some(target) = self.type_tests.get(span) else { return Facts::default() };
                let Some((name, depth, ty, unwraps)) = self.narrowable(expr) else { return Facts::default() };
                if Self::may_be_cell(target) {
                    return Facts::default();
                }
                let payload = match &ty {
                    Type::Nullable(inner) => inner,
                    t => t,
                };
                let hit = if ty == Type::Any || target.is_assignable_to(&ty) { Some(target.clone()) } else { Self::union_hit(payload, target) };
                let rest = Self::union_rest(payload, target).map(|r| if matches!(ty, Type::Nullable(_)) { Type::Nullable(Box::new(r)) } else { r });
                let fact = |ty: Type| Narrowing { name: name.clone(), depth, ty, unwraps, scope: 0 };
                Facts { when_true: hit.into_iter().map(fact).collect(), when_false: rest.into_iter().map(fact).collect() }
            }
            Expr::Unary { op: UnaryOp::Not, expr, .. } => {
                let f = self.facts(expr);
                Facts { when_true: f.when_false, when_false: f.when_true }
            }
            Expr::Binary { left, op: BinaryOp::And, right, .. } => {
                let mut when_true = self.facts(left).when_true;
                when_true.extend(self.facts(right).when_true);
                Facts { when_true, when_false: Vec::new() }
            }
            Expr::Binary { left, op: BinaryOp::Or, right, .. } => {
                let mut when_false = self.facts(left).when_false;
                when_false.extend(self.facts(right).when_false);
                Facts { when_true: Vec::new(), when_false }
            }
            _ => Facts::default(),
        }
    }

    pub(super) fn check_assuming(&mut self, facts: Vec<Narrowing>, e: &Expr) -> Type {
        let len = self.narrowings.len();
        self.assume(facts);
        let ty = self.check_expr(e);
        self.narrowings.truncate(len);
        ty
    }

    pub(super) fn always_exits(body: &[Stmt]) -> bool {
        match body.last() {
            Some(Stmt::Return { .. } | Stmt::Break { .. } | Stmt::Continue { .. }) => true,
            Some(Stmt::If { then_branch, else_branch: Some(else_b), .. }) => Self::always_exits(then_branch) && Self::always_exits(else_b),
            _ => false,
        }
    }

    pub(super) fn optional_access(&mut self, ty: &Type, member: &str, span: Span) {
        let msg = format!(
            "`{}` may be `None`, so `.{member}` needs `?.`, `!`, `??`, `match`, or a `!= null` check first",
            Self::describe(ty)
        );
        self.errors.push((msg, span));
    }

    pub(super) fn optional_operand(&mut self, sym: &str, ty: &Type, span: Span) {
        let msg = format!("`{}` may be `None`, so `{sym}` needs `!`, `??`, or a `!= null` check first", Self::describe(ty));
        self.errors.push((msg, span));
    }
}
