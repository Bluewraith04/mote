use mote_base::{Id, Span};
use mote_syntax as ast;
use mote_syntax::{Ident, LambdaBody, LambdaParam, ParamKind};

use super::{Frame, FnCx};
use crate::cx::Cx;
use crate::tree::{Block, Closure, Expr, ExprKind, Local, LocalKind, PatternKind, StmtKind};
use crate::types::{FnParam, FnTy, TyId, TyKind};

/// A closure parameter as written.
pub(super) struct Written {
    is_var: bool,
    name: Ident,
    ty: Option<Id<ast::Type>>,
}

/// What fixes a closure's result type.
pub(super) enum Result {
    Written(TyId),
    /// The expected type, else the body.
    Open,
}

/// A closure's body as written.
pub(super) enum Code<'a> {
    Expr(Id<ast::Expr>),
    Block(&'a ast::Block),
}

impl Cx<'_> {
    /// The function type an expected type asks of a lambda.
    pub(crate) fn function_hint(&mut self, f: &FnCx, expected: Option<TyId>) -> Option<FnTy> {
        let ty = self.solved(f, expected?);
        let ty = match self.program.types.kind(ty) {
            TyKind::Optional(inner) => *inner,
            _ => ty,
        };
        match self.program.types.kind(ty) {
            TyKind::Function(function) => Some(function.clone()),
            _ => None,
        }
    }

    /// `|a, b| expr` and `|a: Int| { block }`.
    pub(crate) fn lambda(&mut self, f: &mut FnCx, params: &[LambdaParam], ret: Option<Id<ast::Type>>, body: &LambdaBody, expected: Option<TyId>, span: Span) -> Id<Expr> {
        let written: Vec<Written> = params.iter().map(|param| Written { is_var: param.is_var, name: param.name, ty: param.ty }).collect();
        let result = match ret {
            Some(ty) => Result::Written(self.written_type(f, ty)),
            None => Result::Open,
        };
        let code = match body {
            LambdaBody::Expr(expr) => Code::Expr(*expr),
            LambdaBody::Block(block) => Code::Block(block),
        };
        self.closure(f, &written, result, code, expected, span)
    }

    /// `fn name(params) -> Ret { body }` inside a body: a `let` of a closure that sees what came before it and itself.
    pub(crate) fn nested_fn(&mut self, f: &mut FnCx, function: &ast::Function, span: Span) -> StmtKind {
        if !function.generics.is_empty() {
            self.error(function.name.span, "a nested function takes no type parameters");
            return StmtKind::Error;
        }
        let mut written = Vec::new();
        for param in &function.params {
            match &param.kind {
                ParamKind::Named { is_var, name, ty, default: None } => written.push(Written { is_var: *is_var, name: *name, ty: *ty }),
                _ => {
                    self.error(param.span, "a nested function takes plain parameters");
                    return StmtKind::Error;
                }
            }
        }
        let ret = function.ret.map_or(TyId::UNIT, |ty| self.written_type(f, ty));
        let params: Vec<FnParam> = written.iter().map(|param| FnParam { ty: param.ty.map_or(TyId::ERROR, |ty| self.written_type(f, ty)), is_var: param.is_var }).collect();
        let ty = self.program.types.function(params, ret, false);
        let local = f.body.local(Local { name: function.name.name, ty, is_var: false, kind: LocalKind::Let, span: function.name.span });
        self.declare(f, local);
        let init = self.closure(f, &written, Result::Written(ret), Code::Block(&function.body), None, span);
        let pattern = f.body.pattern(PatternKind::Bind(local), ty, function.name.span);
        StmtKind::Let { pattern, init }
    }

    /// A lambda or nested function: code that belongs to no `scope` around it.
    fn closure(&mut self, f: &mut FnCx, params: &[Written], result: Result, code: Code, expected: Option<TyId>, span: Span) -> Id<Expr> {
        let mut hint = self.function_hint(f, expected);
        if let Some(found) = &hint
            && found.params.len() != params.len()
        {
            let message = format!("expected a function of {} parameters, found {}", found.params.len(), params.len());
            self.error(span, message);
            hint = None;
        }
        let task_scopes = std::mem::take(&mut f.task_scopes);
        let (closure, types, ret) = self.closure_parts(f, params, result, code, hint.as_ref());
        f.task_scopes = task_scopes;
        let send = hint.as_ref().is_some_and(|found| found.send);
        if send {
            self.check_captures(f, &closure.captures, span, "a lambda");
            if !self.program.types.has_error(ret) && !self.is_sendable(ret) {
                let message = format!("a Send function cannot return {}", self.show_ty(ret));
                self.error(span, message);
            }
        }
        let ty = self.program.types.function(types, ret, send);
        f.body.expr(ExprKind::Closure(Box::new(closure)), ty, span)
    }

    /// Checks a closure's parameters and body in a new frame.
    pub(super) fn closure_parts(&mut self, f: &mut FnCx, params: &[Written], result: Result, code: Code, hint: Option<&FnTy>) -> (Closure, Vec<FnParam>, TyId) {
        let saved = (f.ret, f.ret_open, f.loops, f.top_level, f.yields.take());
        f.frames.push(Frame { base: f.scopes.len(), captures: Vec::new() });
        f.push_scope();
        f.loops = 0;
        f.top_level = false;
        let mut locals = Vec::new();
        let mut types = Vec::new();
        for (index, param) in params.iter().enumerate() {
            let ty = match param.ty {
                Some(ty) => self.written_type(f, ty),
                None => match hint.map(|found| found.params[index].ty) {
                    Some(ty) if !self.program.types.has_infer(ty) => ty,
                    _ => {
                        let message = format!("parameter `{}` needs a type", self.text(param.name.name));
                        self.error(param.name.span, message);
                        TyId::ERROR
                    }
                },
            };
            let is_var = param.is_var || hint.is_some_and(|found| found.params[index].is_var);
            let local = f.body.local(Local { name: param.name.name, ty, is_var, kind: LocalKind::Param, span: param.name.span });
            self.declare(f, local);
            locals.push(local);
            types.push(FnParam { ty, is_var });
        }
        let ret = match result {
            Result::Written(ty) => Some(ty),
            Result::Open => hint.map(|found| found.ret).filter(|&ty| !self.program.types.has_infer(ty)),
        };
        f.ret = ret.unwrap_or(TyId::ERROR);
        f.ret_open = ret.is_none();
        let body = match code {
            Code::Expr(expr) => {
                let node = self.expr(f, expr, ret);
                let node_span = f.body.exprs[node].span;
                let stmt = match ret {
                    Some(TyId::UNIT) => StmtKind::Expr(node),
                    Some(ty) => StmtKind::Return(Some(self.coerce(f, node, ty, node_span))),
                    None => {
                        f.ret = f.body.exprs[node].ty;
                        StmtKind::Return(Some(node))
                    }
                };
                Block { stmts: vec![f.body.stmt(stmt, node_span)], span: node_span }
            }
            Code::Block(block) => {
                let checked = self.block_stmts(f, block);
                if f.ret_open {
                    f.ret = TyId::UNIT;
                } else if f.ret != TyId::UNIT && f.ret != TyId::ERROR && !Self::block_diverges(&f.body, &checked, false) {
                    self.error(block.span, "missing return");
                }
                checked
            }
        };
        let ret = f.ret;
        (f.ret, f.ret_open, f.loops, f.top_level, f.yields) = saved;
        f.pop_scope();
        let frame = f.frames.pop().expect("a closure pushed its frame");
        (Closure { params: locals, captures: frame.captures, body }, types, ret)
    }
}
