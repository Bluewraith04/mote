//! Shared cells for captured `var`s: a local a lambda captures and code assigns lives in a one-slot heap object.

use super::*;

impl CodeGenerator {
    pub(crate) fn cell_names(&self, body: &[Stmt]) -> HashSet<String> {
        let mut captured = Vec::new();
        let mut assigned = HashSet::new();
        walk_stmts(body, &mut |node| match node {
            Node::Stmt(Stmt::Assign { target: Expr::Ident(n, _), .. })
            | Node::Stmt(Stmt::CompoundAssign { target: Expr::Ident(n, _), .. }) => {
                assigned.insert(n.clone());
            }
            Node::Expr(Expr::Lambda { params, body, .. }) => {
                let mut bound: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();
                self.lambda_free_vars(body, &mut bound, &mut captured);
            }
            _ => {}
        });
        captured.into_iter().filter(|n| assigned.contains(n)).collect()
    }

    pub(crate) fn emit_new_cell(&mut self, value_reg: u8) -> u8 {
        let type_idx = match self.cell_type {
            Some(idx) => idx,
            None => {
                let idx = self.type_descriptors.len() as u16;
                self.type_descriptors.push(TypeDescriptor::with_field_count(idx as u64, 1));
                self.cell_type = Some(idx);
                idx
            }
        };
        let cell = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::NEWOBJ, cell, type_idx));
        self.current_insts.push(encode_r3(Opcode::SETFIELD, cell, 0, value_reg));
        cell
    }

    pub(crate) fn bind_cell(&mut self, name: &str, value_reg: u8) {
        let cell = self.emit_new_cell(value_reg);
        self.reg_alloc.free_temp(value_reg);
        self.reg_alloc.bind_var(name, cell);
        self.reg_alloc.mark_cell(name);
    }
}

pub(crate) enum Node<'a> {
    Stmt(&'a Stmt),
    Expr(&'a Expr),
}

pub(crate) fn walk_stmts<'a>(body: &'a [Stmt], f: &mut dyn FnMut(Node<'a>)) {
    for s in body {
        walk_stmt(s, f);
    }
}

fn walk_stmt<'a>(s: &'a Stmt, f: &mut dyn FnMut(Node<'a>)) {
    f(Node::Stmt(s));
    match s {
        Stmt::Let { init, .. } | Stmt::Var { init, .. } | Stmt::TupleLet { init, .. } => walk_expr(init, f),
        Stmt::Assign { target, value, .. } | Stmt::CompoundAssign { target, value, .. } => {
            walk_expr(target, f);
            walk_expr(value, f);
        }
        Stmt::Expr { expr, .. } | Stmt::Yield { value: expr, .. } => walk_expr(expr, f),
        Stmt::Return { value, .. } | Stmt::Break { value, .. } => {
            if let Some(v) = value {
                walk_expr(v, f);
            }
        }
        Stmt::If { cond, then_branch, else_branch, .. } => {
            walk_expr(cond, f);
            walk_stmts(then_branch, f);
            if let Some(eb) = else_branch {
                walk_stmts(eb, f);
            }
        }
        Stmt::While { cond, body, .. } => {
            walk_expr(cond, f);
            walk_stmts(body, f);
        }
        Stmt::ForIn { iter, body, .. } => {
            walk_expr(iter, f);
            walk_stmts(body, f);
        }
        Stmt::Match { expr, arms, .. } => {
            walk_expr(expr, f);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    walk_expr(g, f);
                }
                walk_stmts(&arm.body, f);
            }
        }
        Stmt::Block { body, .. } | Stmt::ScopeBlock { body, .. } | Stmt::SpawnBlock { body, .. } => walk_stmts(body, f),
        Stmt::WithBlock { init, body, .. } => {
            walk_expr(init, f);
            walk_stmts(body, f);
        }
        Stmt::Continue { .. } => {}
    }
}

fn walk_expr<'a>(e: &'a Expr, f: &mut dyn FnMut(Node<'a>)) {
    f(Node::Expr(e));
    match e {
        Expr::Binary { left, right, .. } | Expr::NullCoalesce { left, right, .. } => {
            walk_expr(left, f);
            walk_expr(right, f);
        }
        Expr::Unary { expr, .. }
        | Expr::TypeTest { expr, .. }
        | Expr::Try { expr, .. }
        | Expr::Unwrap { expr, .. } => walk_expr(expr, f),
        Expr::Spawn { body, .. } | Expr::Block { body, .. } | Expr::Lambda { body, .. } => walk_stmts(body, f),
        Expr::Call { callee, args, .. } => {
            walk_expr(callee, f);
            for a in args {
                walk_expr(a, f);
            }
        }
        Expr::MemberAccess { object, .. } => walk_expr(object, f),
        Expr::OptionalChain { object, body, .. } => {
            walk_expr(object, f);
            walk_expr(body, f);
        }
        Expr::Index { object, index, .. } => {
            walk_expr(object, f);
            walk_expr(index, f);
        }
        Expr::Range { start, end, .. } => {
            for x in [start, end].into_iter().flatten() {
                walk_expr(x, f);
            }
        }
        Expr::Ternary { cond, then_expr, else_expr, .. } => {
            walk_expr(cond, f);
            walk_expr(then_expr, f);
            walk_expr(else_expr, f);
        }
        Expr::ListLiteral { elements, .. } | Expr::TupleLiteral { elements, .. } => {
            for el in elements {
                walk_expr(el, f);
            }
        }
        Expr::MapLiteral { entries, .. } => {
            for (k, v) in entries {
                walk_expr(k, f);
                walk_expr(v, f);
            }
        }
        Expr::StructInit { fields, .. } => {
            for (_, v) in fields {
                walk_expr(v, f);
            }
        }
        Expr::If { cond, then_branch, else_branch, .. } => {
            walk_expr(cond, f);
            walk_stmts(then_branch, f);
            walk_stmts(else_branch, f);
        }
        Expr::Match { expr, arms, .. } => {
            walk_expr(expr, f);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    walk_expr(g, f);
                }
                walk_stmts(&arm.body, f);
            }
        }
        _ => {}
    }
}
