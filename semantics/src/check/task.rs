use mote_base::{Id, Span};
use mote_syntax as ast;
use mote_syntax::{Ident, SpawnBody};

use super::closure::{Code, Result};
use super::FnCx;
use crate::cx::Cx;
use crate::defs::{DefKind, Receiver};
use crate::tree::{Expr, ExprKind, FnRef, Local, LocalKind, StmtKind};
use crate::types::{BuiltinTy, TyId, TyKind};

impl Cx<'_> {
    /// `spawn { … }` and `spawn f(args)`: a `Task` of what the body returns.
    pub(crate) fn spawn_expr(&mut self, f: &mut FnCx, body: &SpawnBody, span: Span) -> Id<Expr> {
        let code = match body {
            SpawnBody::Block(block) => Code::Block(block),
            SpawnBody::Expr(expr) => Code::Expr(*expr),
        };
        let scoped = f.task_scopes > 0;
        let (task, _, ret) = self.closure_parts(f, &[], Result::Open, code, None);
        self.check_captures(f, &task.captures, span, "a spawn block");
        if !self.program.types.has_error(ret) && !self.is_sendable(ret) {
            let message = format!("a task cannot return {}", self.show_ty(ret));
            self.error(span, message);
        }
        let ty = self.program.types.builtin(BuiltinTy::Task, vec![ret]);
        f.body.expr(ExprKind::Spawn { task: Box::new(task), scoped }, ty, span)
    }

    /// `scope { … }`: the tasks spawned in it are awaited at its end.
    pub(crate) fn scope_stmt(&mut self, f: &mut FnCx, block: &ast::Block) -> StmtKind {
        f.task_scopes += 1;
        f.open_cleanups += 1;
        let body = self.block(f, block);
        f.open_cleanups -= 1;
        f.task_scopes -= 1;
        StmtKind::Scope(body)
    }

    /// `with name = expr { … }`: the resource is a `let` in the block, closed on every exit.
    pub(crate) fn with_stmt(&mut self, f: &mut FnCx, name: Ident, init: Id<ast::Expr>, body: &ast::Block) -> StmtKind {
        let init_span = self.ast(f.module)[init].span;
        let init = self.expr(f, init, None);
        let ty = f.body.exprs[init].ty;
        let close = self.check_closeable(f, ty, init_span);
        f.push_scope();
        let local = f.body.local(Local { name: name.name, ty, is_var: false, kind: LocalKind::Let, span: name.span });
        self.declare(f, local);
        f.open_cleanups += 1;
        let body = self.block(f, body);
        f.open_cleanups -= 1;
        f.pop_scope();
        StmtKind::With { local, init, body, close }
    }

    /// Reports a resource type that has no `close(self)`; the method of a concrete type, none for a type parameter.
    fn check_closeable(&mut self, f: &FnCx, ty: TyId, span: Span) -> Option<FnRef> {
        if self.program.types.has_error(ty) {
            return None;
        }
        let closed = self.interner.intern("close");
        let param = match self.program.types.kind(ty) {
            TyKind::Param { index, .. } => Some(*index as usize),
            _ => None,
        };
        let (found, ok) = match param {
            Some(index) => {
                let bounded = f.bounds.get(index).is_some_and(|bounds| bounds.iter().any(|bound| self.program.std.closeable == Some(bound.def)));
                (None, bounded)
            }
            None => {
                let candidates = self.methods_for_type(ty, closed);
                let found = candidates.into_iter().find_map(|(method, args)| {
                    let item = &self.program.defs[method];
                    (self.visible(f, method, item.is_pub)
                        && matches!(&item.kind, DefKind::Function(function) if function.receiver == Receiver::Read && function.params.is_empty()))
                    .then_some(FnRef { def: method, args })
                });
                let ok = found.is_some();
                (found, ok)
            }
        };
        if !ok {
            let message = format!("{} has no `close(self)` method", self.show_ty(ty));
            self.error(span, message);
        }
        found
    }
}
