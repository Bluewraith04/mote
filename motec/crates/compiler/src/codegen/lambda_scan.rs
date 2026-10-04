//! Free-variable scan for lambda bodies.

use super::*;

impl CodeGenerator {
    pub(crate) fn lambda_free_vars(&self, body: &[Stmt], bound: &mut HashSet<String>, free: &mut Vec<String>) {
        for s in body {
            self.lambda_fv_stmt(s, bound, free);
        }
    }

    pub(crate) fn lambda_fv_stmt(&self, s: &Stmt, bound: &mut HashSet<String>, free: &mut Vec<String>) {
        match s {
            Stmt::Let { name, init, .. } | Stmt::Var { name, init, .. } => {
                self.lambda_fv_expr(init, bound, free);
                bound.insert(name.clone());
            }
            Stmt::TupleLet { names, init, .. } => {
                self.lambda_fv_expr(init, bound, free);
                for n in names {
                    bound.insert(n.clone());
                }
            }
            Stmt::Assign { target, value, .. } | Stmt::CompoundAssign { target, value, .. } => {
                self.lambda_fv_expr(target, bound, free);
                self.lambda_fv_expr(value, bound, free);
            }
            Stmt::Expr { expr, .. } => self.lambda_fv_expr(expr, bound, free),
            Stmt::Return { value, .. } | Stmt::Break { value, .. } => {
                if let Some(v) = value {
                    self.lambda_fv_expr(v, bound, free);
                }
            }
            Stmt::If { cond, then_branch, else_branch, .. } => {
                self.lambda_fv_expr(cond, bound, free);
                self.lambda_free_vars(then_branch, bound, free);
                if let Some(eb) = else_branch {
                    self.lambda_free_vars(eb, bound, free);
                }
            }
            Stmt::While { cond, body, .. } => {
                self.lambda_fv_expr(cond, bound, free);
                self.lambda_free_vars(body, bound, free);
            }
            Stmt::ForIn { var_name, iter, body, .. } => {
                self.lambda_fv_expr(iter, bound, free);
                bound.insert(var_name.clone());
                self.lambda_free_vars(body, bound, free);
            }
            Stmt::Match { expr, arms, .. } => {
                self.lambda_fv_expr(expr, bound, free);
                for arm in arms {
                    Self::pattern_binding_names(&arm.pattern, bound);
                    if let Some(g) = &arm.guard {
                        self.lambda_fv_expr(g, bound, free);
                    }
                    self.lambda_free_vars(&arm.body, bound, free);
                }
            }
            Stmt::Block { body, .. }
            | Stmt::ScopeBlock { body, .. }
            | Stmt::SpawnBlock { body, .. } => {
                self.lambda_free_vars(body, bound, free);
            }
            Stmt::Yield { value, .. } => self.lambda_fv_expr(value, bound, free),
            Stmt::WithBlock { name, init, body, .. } => {
                self.lambda_fv_expr(init, bound, free);
                bound.insert(name.clone());
                self.lambda_free_vars(body, bound, free);
            }
            Stmt::Continue { .. } => {}
        }
    }

    pub(crate) fn lambda_fv_expr(&self, e: &Expr, bound: &mut HashSet<String>, free: &mut Vec<String>) {
        match e {
            Expr::Ident(name, _) => {
                if !bound.contains(name) && !free.contains(name) {
                    free.push(name.clone());
                }
            }
            Expr::Binary { left, right, .. } | Expr::NullCoalesce { left, right, .. } => {
                self.lambda_fv_expr(left, bound, free);
                self.lambda_fv_expr(right, bound, free);
            }
            Expr::Unary { expr, .. }
            | Expr::TypeTest { expr, .. }
            | Expr::Try { expr, .. }
            | Expr::Unwrap { expr, .. } => self.lambda_fv_expr(expr, bound, free),
            Expr::Spawn { body, .. } => self.lambda_free_vars(body, bound, free),
            Expr::Call { callee, args, .. } => {
                self.lambda_fv_expr(callee, bound, free);
                for a in args {
                    self.lambda_fv_expr(a, bound, free);
                }
            }
            Expr::MemberAccess { object, .. } => self.lambda_fv_expr(object, bound, free),
            Expr::OptionalChain { object, temp, body, .. } => {
                self.lambda_fv_expr(object, bound, free);
                let mut inner = bound.clone();
                inner.insert(temp.clone());
                self.lambda_fv_expr(body, &mut inner, free);
            }
            Expr::Index { object, index, .. } => {
                self.lambda_fv_expr(object, bound, free);
                self.lambda_fv_expr(index, bound, free);
            }
            Expr::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.lambda_fv_expr(s, bound, free);
                }
                if let Some(en) = end {
                    self.lambda_fv_expr(en, bound, free);
                }
            }
            Expr::Ternary { cond, then_expr, else_expr, .. } => {
                self.lambda_fv_expr(cond, bound, free);
                self.lambda_fv_expr(then_expr, bound, free);
                self.lambda_fv_expr(else_expr, bound, free);
            }
            Expr::ListLiteral { elements, .. } | Expr::TupleLiteral { elements, .. } => {
                for el in elements {
                    self.lambda_fv_expr(el, bound, free);
                }
            }
            Expr::MapLiteral { entries, .. } => {
                for (k, v) in entries {
                    self.lambda_fv_expr(k, bound, free);
                    self.lambda_fv_expr(v, bound, free);
                }
            }
            Expr::StructInit { fields, .. } => {
                for (_, v) in fields {
                    self.lambda_fv_expr(v, bound, free);
                }
            }
            Expr::If { cond, then_branch, else_branch, .. } => {
                self.lambda_fv_expr(cond, bound, free);
                self.lambda_free_vars(then_branch, bound, free);
                self.lambda_free_vars(else_branch, bound, free);
            }
            Expr::Match { expr, arms, .. } => {
                self.lambda_fv_expr(expr, bound, free);
                for arm in arms {
                    Self::pattern_binding_names(&arm.pattern, bound);
                    if let Some(g) = &arm.guard {
                        self.lambda_fv_expr(g, bound, free);
                    }
                    self.lambda_free_vars(&arm.body, bound, free);
                }
            }
            Expr::Block { body, .. } => {
                self.lambda_free_vars(body, bound, free)
            }
            Expr::Lambda { params, body, .. } => {
                let mut inner = bound.clone();
                for p in params {
                    inner.insert(p.name.clone());
                }
                self.lambda_free_vars(body, &mut inner, free);
            }
            _ => {}
        }
    }

    pub(crate) fn pattern_binding_names(pat: &Pattern, set: &mut HashSet<String>) {
        match pat {
            Pattern::Identifier { name, subpattern, .. } => {
                set.insert(name.clone());
                if let Some(sub) = subpattern {
                    Self::pattern_binding_names(sub, set);
                }
            }
            Pattern::Type { name: Some(name), .. } => {
                set.insert(name.clone());
            }
            Pattern::Or(alts, _) => {
                for a in alts {
                    Self::pattern_binding_names(a, set);
                }
            }
            Pattern::Tuple(subs, _) => {
                for s in subs {
                    Self::pattern_binding_names(s, set);
                }
            }
            _ => {}
        }
    }
}
