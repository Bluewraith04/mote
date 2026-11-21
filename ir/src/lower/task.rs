use mote_base::{Id, Span};
use mote_semantics::{Block, Closure, DefKind, Expr, FnRef, GenericArg, Local, Payload, TyId, TyKind};

use super::{Cleanup, Lowering};
use crate::ir::{Callee, FuncId, Op, Operand, Terminator};

impl Lowering<'_> {
    /// `spawn`: the body is a function of its own; the arrays it captures are copied into the task, and so is its result.
    pub(super) fn spawn(&mut self, task: &Closure, scoped: bool, ty: TyId, span: Span) -> Operand {
        let ret = match self.program.types.kind(ty) {
            TyKind::Builtin { args, .. } => args.first().copied().unwrap_or(TyId::UNIT),
            _ => TyId::UNIT,
        };
        let mut captures = Vec::new();
        for &local in &task.captures {
            let Some(&reg) = self.regs.get(&local) else { return self.unsupported(span, "capture of an unsupported local") };
            captures.push(self.detach(Operand::Reg(reg), self.body.locals[local].ty, span));
        }
        let detach_result = self.holds_array(ret);
        let func = self.closure_function(task, ret, detach_result);
        let scope = if scoped { self.scopes.last().copied() } else { None };
        self.b.value(ty, Op::Spawn { func, captures, scope }, span)
    }

    /// `scope { }`: the tasks spawned in it are waited for on every way out.
    pub(super) fn scope_stmt(&mut self, block: &Block, span: Span) {
        let token = self.b.value(TyId::INT, Op::ScopeBegin, span);
        self.scopes.push(token);
        self.cleanups.push(Cleanup::Scope(token));
        self.block(block);
        self.cleanups.pop();
        self.scopes.pop();
        if !self.b.is_terminated() {
            self.scope_end(token, self.cleanups.len(), span);
        }
    }

    /// Waits for the tasks of a scope opened after the first `outer` cleanups; in a function that returns a `Result` of `Error`, a task nobody awaited that faulted returns `Err`.
    fn scope_end(&mut self, token: Operand, outer: usize, span: Span) {
        if !self.returns_error() {
            self.b.emit(None, Op::ScopeEnd(token), span);
            return;
        }
        let optional = self.program.types.optional(self.ret);
        let failure = self.b.value(optional, Op::ScopeEnd(token), span);
        let failed = self.b.value(TyId::BOOL, Op::IsSome(failure), span);
        let (fail, ok) = (self.b.block(), self.b.block());
        self.b.terminate(Terminator::Branch { condition: failed, then: fail, otherwise: ok });
        self.b.switch_to(fail);
        let error = self.b.value(self.ret, Op::SomePayload(failure), span);
        self.return_from(Some(error), outer);
        self.b.switch_to(ok);
    }

    fn returns_error(&self) -> bool {
        let types = &self.program.types;
        match types.kind(self.ret) {
            TyKind::Adt { def, args } if self.program.std.result == Some(*def) => {
                args.get(1)
                    .and_then(GenericArg::as_type)
                    .is_some_and(|error| matches!(types.kind(error), TyKind::Adt { def, .. } if self.program.std.error == Some(*def)))
            }
            _ => false,
        }
    }

    /// `with name = init { }`: the resource is closed on every way out of the block.
    pub(super) fn with_stmt(&mut self, local: Id<Local>, init: Id<Expr>, body: &Block, close: &Option<FnRef>, span: Span) {
        let value = self.expr(init);
        self.bind_local(local, value, span);
        let Some(&func) = close.as_ref().and_then(|function| self.instances.get(function)) else {
            self.unsupported(span, "with without close");
            self.block(body);
            return;
        };
        let resource = Operand::Reg(self.regs[&local]);
        self.b.emit(None, Op::Defer { resource, func }, span);
        self.cleanups.push(Cleanup::Close { resource, func });
        self.block(body);
        self.cleanups.pop();
        if !self.b.is_terminated() {
            self.close_call(resource, func, span);
        }
    }

    fn close_call(&mut self, resource: Operand, func: FuncId, span: Span) {
        self.b.emit(None, Op::Undefer, span);
        self.b.emit(None, Op::Call { callee: Callee::Function(func), args: vec![resource], outs: Vec::new() }, span);
    }

    /// Undoes the scopes and `with`s opened after the first `depth`, innermost first.
    pub(super) fn run_cleanups(&mut self, depth: usize, span: Span) {
        self.run_cleanups_between(depth, self.cleanups.len(), span);
    }

    /// Undoes the cleanups from `depth` up to, not including, `top`, innermost first.
    pub(super) fn run_cleanups_between(&mut self, depth: usize, top: usize, span: Span) {
        for index in (depth..top).rev() {
            match self.cleanups[index].clone() {
                Cleanup::Scope(token) => self.scope_end(token, index, span),
                Cleanup::Close { resource, func } => self.close_call(resource, func, span),
            }
        }
    }

    /// The value as a copy for another task when it holds a `[T]`.
    pub(super) fn detach(&mut self, operand: Operand, ty: TyId, span: Span) -> Operand {
        if self.holds_array(ty) { self.b.value(ty, Op::Detach(operand), span) } else { operand }
    }

    /// Whether a value of the type reaches a `[T]` without going through a handle or a collection.
    pub(super) fn holds_array(&mut self, ty: TyId) -> bool {
        self.holds_array_in(ty, &mut Vec::new())
    }

    fn holds_array_in(&mut self, ty: TyId, seen: &mut Vec<TyId>) -> bool {
        if seen.contains(&ty) {
            return false;
        }
        seen.push(ty);
        match self.program.types.kind(ty).clone() {
            TyKind::Array(_) => true,
            TyKind::Optional(inner) => self.holds_array_in(inner, seen),
            TyKind::Tuple(items) | TyKind::Union(items) => items.into_iter().any(|item| self.holds_array_in(item, seen)),
            TyKind::Builtin { .. } => false,
            TyKind::Adt { def, args } => {
                if self.program.std.map == Some(def) || self.program.std.set == Some(def) {
                    return false;
                }
                let fields: Vec<TyId> = match &self.program.defs[def].kind {
                    DefKind::Record(record) => record.fields.iter().map(|field| field.ty).collect(),
                    DefKind::Enum(item) => item
                        .variants
                        .iter()
                        .flat_map(|variant| match &variant.payload {
                            Payload::Unit => Vec::new(),
                            Payload::Tuple(types) => types.clone(),
                            Payload::Struct(fields) => fields.iter().map(|field| field.ty).collect(),
                        })
                        .collect(),
                    _ => return false,
                };
                fields.into_iter().any(|field| {
                    let field = self.program.types.subst_generic(field, &args);
                    self.holds_array_in(field, seen)
                })
            }
            _ => false,
        }
    }
}
