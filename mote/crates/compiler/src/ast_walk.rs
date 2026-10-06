//! A mutable walk over every expression of a program, parents before children.

use crate::ast::*;

/// Calls `f` on each expression of `items`, then on the children of what `f` left in place.
pub(crate) fn exprs_mut(items: &mut [Item], f: &mut dyn FnMut(&mut Expr)) {
    for item in items {
        match item {
            Item::Function(d) => function(d, f),
            Item::Struct(s) => s.methods.iter_mut().for_each(|m| function(m, f)),
            Item::Class(c) => c.methods.iter_mut().for_each(|m| function(m, f)),
            Item::Enum(e) => {
                for v in &mut e.variants {
                    if let EnumVariantKind::Unit { discriminant: Some(d) } = &mut v.kind {
                        expr(d, f);
                    }
                }
                e.methods.iter_mut().for_each(|m| function(m, f));
            }
            Item::Trait(t) => {
                for m in &mut t.members {
                    params(&mut m.params, f);
                    if let Some(body) = &mut m.default_body {
                        stmts(body, f);
                    }
                }
            }
            Item::TopLevelStmt(s) => stmt(s, f),
            Item::NativeFunction(_) | Item::TypeAlias(_) | Item::Import(_) => {}
        }
    }
}

fn function(d: &mut FunctionDecl, f: &mut dyn FnMut(&mut Expr)) {
    params(&mut d.params, f);
    stmts(&mut d.body, f);
}

fn params(ps: &mut [Param], f: &mut dyn FnMut(&mut Expr)) {
    for p in ps {
        if let Some(ParameterKind::Regular { default_value: Some(d), .. }) = &mut p.kind {
            expr(d, f);
        }
    }
}

fn stmts(body: &mut [Stmt], f: &mut dyn FnMut(&mut Expr)) {
    body.iter_mut().for_each(|s| stmt(s, f));
}

fn stmt(s: &mut Stmt, f: &mut dyn FnMut(&mut Expr)) {
    match s {
        Stmt::Let { init, .. } | Stmt::Var { init, .. } | Stmt::TupleLet { init, .. } => expr(init, f),
        Stmt::Assign { target, value, .. } | Stmt::CompoundAssign { target, value, .. } => {
            expr(target, f);
            expr(value, f);
        }
        Stmt::Expr { expr: e, .. } | Stmt::Yield { value: e, .. } => expr(e, f),
        Stmt::Return { value, .. } | Stmt::Break { value, .. } => {
            if let Some(v) = value {
                expr(v, f);
            }
        }
        Stmt::If { cond, then_branch, else_branch, .. } => {
            expr(cond, f);
            stmts(then_branch, f);
            if let Some(b) = else_branch {
                stmts(b, f);
            }
        }
        Stmt::While { cond, body, .. } => {
            expr(cond, f);
            stmts(body, f);
        }
        Stmt::ForIn { iter, body, .. } => {
            expr(iter, f);
            stmts(body, f);
        }
        Stmt::Match { expr: e, arms, .. } => {
            expr(e, f);
            match_arms(arms, f);
        }
        Stmt::ScopeBlock { body, .. } | Stmt::SpawnBlock { body, .. } | Stmt::Block { body, .. } => stmts(body, f),
        Stmt::WithBlock { init, body, .. } => {
            expr(init, f);
            stmts(body, f);
        }
        Stmt::Continue { .. } => {}
    }
}

fn match_arms(arms: &mut [MatchArm], f: &mut dyn FnMut(&mut Expr)) {
    for arm in arms {
        if let Some(g) = &mut arm.guard {
            expr(g, f);
        }
        stmts(&mut arm.body, f);
    }
}

fn expr(e: &mut Expr, f: &mut dyn FnMut(&mut Expr)) {
    f(e);
    match e {
        Expr::Binary { left, right, .. } | Expr::NullCoalesce { left, right, .. } => {
            expr(left, f);
            expr(right, f);
        }
        Expr::Unary { expr: x, .. } | Expr::TypeTest { expr: x, .. } | Expr::Try { expr: x, .. } | Expr::Unwrap { expr: x, .. } => expr(x, f),
        Expr::Call { callee, args, .. } => {
            expr(callee, f);
            args.iter_mut().for_each(|a| expr(a, f));
        }
        Expr::MemberAccess { object, .. } => expr(object, f),
        Expr::OptionalChain { object, body, .. } => {
            expr(object, f);
            expr(body, f);
        }
        Expr::Index { object, index, .. } => {
            expr(object, f);
            expr(index, f);
        }
        Expr::Range { start, end, .. } => {
            for x in [start, end].into_iter().flatten() {
                expr(x, f);
            }
        }
        Expr::Ternary { cond, then_expr, else_expr, .. } => {
            expr(cond, f);
            expr(then_expr, f);
            expr(else_expr, f);
        }
        Expr::ListLiteral { elements, .. } | Expr::TupleLiteral { elements, .. } => elements.iter_mut().for_each(|x| expr(x, f)),
        Expr::MapLiteral { entries, .. } => {
            for (k, v) in entries {
                expr(k, f);
                expr(v, f);
            }
        }
        Expr::StructInit { fields, .. } => fields.iter_mut().for_each(|(_, v)| expr(v, f)),
        Expr::Lambda { params: ps, body, .. } => {
            params(ps, f);
            stmts(body, f);
        }
        Expr::If { cond, then_branch, else_branch, .. } => {
            expr(cond, f);
            stmts(then_branch, f);
            stmts(else_branch, f);
        }
        Expr::Match { expr: x, arms, .. } => {
            expr(x, f);
            match_arms(arms, f);
        }
        Expr::Spawn { body, .. } | Expr::Block { body, .. } => stmts(body, f),
        Expr::Int(..) | Expr::Float(..) | Expr::Bool(..) | Expr::String(..) | Expr::Char(..) | Expr::Null(_) | Expr::Ident(..) | Expr::SelfValue(_) | Expr::StaticAccess { .. } => {}
    }
}
