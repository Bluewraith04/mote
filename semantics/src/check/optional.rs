use mote_base::{Id, Span};
use mote_syntax as ast;
use mote_syntax::Ident;

use super::coerce::plan;
use super::FnCx;
use crate::cx::Cx;
use crate::tree::{Arm, ArmBody, Expr, ExprKind, Local, LocalKind, Match, PatternKind};
use crate::types::{TyId, TyKind};

impl Cx<'_> {
    pub(crate) fn optional_inner(&self, ty: TyId) -> Option<TyId> {
        match self.program.types.kind(ty) {
            TyKind::Optional(inner) => Some(*inner),
            _ => None,
        }
    }

    /// A name for a value the compiler binds itself.
    fn fresh_local(&mut self, f: &mut FnCx, ty: TyId, span: Span) -> (Id<Local>, Id<Expr>) {
        let name = self.interner.intern("?");
        let local = f.body.local(Local { name, ty, is_var: false, kind: LocalKind::Pattern, span });
        (local, f.body.expr(ExprKind::Local(local), ty, span))
    }

    /// `match subject { Some(local) => some, None => none }`.
    #[allow(clippy::too_many_arguments)]
    fn optional_match(&mut self, f: &mut FnCx, subject: Id<Expr>, local: Id<Local>, some: Id<Expr>, none: Id<Expr>, ty: TyId, span: Span) -> Id<Expr> {
        let subject_ty = f.body.exprs[subject].ty;
        let inner = f.body.locals[local].ty;
        let bind = f.body.pattern(PatternKind::Bind(local), inner, span);
        let some_pattern = f.body.pattern(PatternKind::OptionalSome(bind), subject_ty, span);
        let none_pattern = f.body.pattern(PatternKind::OptionalNone, subject_ty, span);
        let arms = vec![Arm { pattern: some_pattern, guard: None, body: ArmBody::Value(some), span }, Arm { pattern: none_pattern, guard: None, body: ArmBody::Value(none), span }];
        f.body.expr(ExprKind::Match(Match { subject, arms }), ty, span)
    }

    /// `a ?? b`.
    pub(crate) fn coalesce(&mut self, f: &mut FnCx, left: Id<ast::Expr>, right: Id<ast::Expr>, span: Span) -> Id<Expr> {
        let subject = self.expr(f, left, None);
        let ty = f.body.exprs[subject].ty;
        let Some(inner) = self.optional_inner(ty) else {
            if ty != TyId::ERROR {
                let message = format!("`??` needs an optional on the left, found {}", self.show_ty(ty));
                self.error(span, message);
            }
            self.expr(f, right, None);
            return self.error_expr(f, span);
        };
        let default = self.expr(f, right, Some(inner));
        let default_ty = f.body.exprs[default].ty;
        if default_ty == TyId::ERROR {
            return self.error_expr(f, span);
        }
        let target = if plan(self.program, default_ty, inner).is_some() { inner } else { ty };
        let (local, value) = self.fresh_local(f, inner, span);
        let some = self.coerce(f, value, target, span);
        let none = self.coerce(f, default, target, span);
        self.optional_match(f, subject, local, some, none, target, span)
    }

    /// Runs `member` on the payload of an optional; the result is `None` when the optional is.
    pub(crate) fn optional_chain<F>(&mut self, f: &mut FnCx, subject: Id<Expr>, span: Span, member: F) -> Id<Expr>
    where
        F: FnOnce(&mut Self, &mut FnCx, Id<Expr>) -> Id<Expr>,
    {
        let ty = f.body.exprs[subject].ty;
        let Some(inner) = self.optional_inner(ty) else {
            if ty != TyId::ERROR {
                let message = format!("`?.` needs an optional, found {}", self.show_ty(ty));
                self.error(span, message);
            }
            return self.error_expr(f, span);
        };
        let (local, payload) = self.fresh_local(f, inner, span);
        let value = member(self, f, payload);
        let value_ty = f.body.exprs[value].ty;
        if value_ty == TyId::ERROR {
            return self.error_expr(f, span);
        }
        let result = match self.program.types.kind(value_ty) {
            _ if value_ty == TyId::UNIT => TyId::UNIT,
            TyKind::Optional(_) => value_ty,
            _ => self.program.types.optional(value_ty),
        };
        let some = self.coerce(f, value, result, span);
        let none = f.body.expr(ExprKind::Null, result, span);
        self.optional_match(f, subject, local, some, none, result, span)
    }

    pub(crate) fn optional_member(&mut self, f: &mut FnCx, object: Id<ast::Expr>, name: Ident, span: Span) -> Id<Expr> {
        let subject = self.expr(f, object, None);
        self.optional_chain(f, subject, span, |this, f, payload| this.field(f, payload, name, span))
    }

    /// The payload type of an optional or `Result`, and the error type of a `Result`.
    fn payload_of(&self, ty: TyId) -> Option<(TyId, Option<TyId>)> {
        match self.program.types.kind(ty) {
            TyKind::Optional(inner) => Some((*inner, None)),
            TyKind::Adt { def, args } if self.program.std.result == Some(*def) && args.len() == 2 => {
                Some((args[0].as_type()?, args[1].as_type()))
            }
            _ => None,
        }
    }

    /// `e?`.
    pub(crate) fn propagate(&mut self, f: &mut FnCx, operand: Id<ast::Expr>, span: Span) -> Id<Expr> {
        let value = self.expr(f, operand, None);
        let ty = f.body.exprs[value].ty;
        if ty == TyId::ERROR {
            return self.error_expr(f, span);
        }
        let Some((payload, error)) = self.payload_of(ty) else {
            let message = format!("`?` needs an optional or a Result, found {}", self.show_ty(ty));
            self.error(span, message);
            return self.error_expr(f, span);
        };
        let ret = f.ret;
        if ret != TyId::ERROR {
            match (error, self.payload_of(ret)) {
                (None, Some((_, None))) => {}
                (None, _) => self.error(span, "`?` on an optional needs a function returning an optional"),
                (Some(error), Some((_, Some(fits)))) => {
                    if plan(self.program, error, fits).is_none() {
                        let message = format!("the error {} does not fit {}", self.show_ty(error), self.show_ty(fits));
                        self.error(span, message);
                    }
                }
                (Some(_), _) => self.error(span, "`?` on a Result needs a function returning a Result"),
            }
        }
        f.body.expr(ExprKind::Propagate(value), payload, span)
    }

    /// `e!`.
    pub(crate) fn unwrap(&mut self, f: &mut FnCx, operand: Id<ast::Expr>, span: Span) -> Id<Expr> {
        let value = self.expr(f, operand, None);
        let ty = f.body.exprs[value].ty;
        if ty == TyId::ERROR {
            return self.error_expr(f, span);
        }
        let Some((payload, _)) = self.payload_of(ty) else {
            let message = format!("`!` needs an optional or a Result, found {}", self.show_ty(ty));
            self.error(span, message);
            return self.error_expr(f, span);
        };
        f.body.expr(ExprKind::Unwrap(value), payload, span)
    }

    /// `e is T`.
    pub(crate) fn type_test(&mut self, f: &mut FnCx, operand: Id<ast::Expr>, written: Id<ast::Type>, span: Span) -> Id<Expr> {
        let value = self.expr(f, operand, None);
        let target = self.written_type(f, written);
        let subject = f.body.exprs[value].ty;
        if subject == TyId::ERROR || target == TyId::ERROR {
            return self.error_expr(f, span);
        }
        if !self.type_can_match(subject, target) {
            let message = format!("{} is never {}", self.show_ty(subject), self.show_ty(target));
            self.error(span, message);
            return self.error_expr(f, span);
        }
        f.body.expr(ExprKind::Is { expr: value, ty: target }, TyId::BOOL, span)
    }

    /// `Some(v)`, `Ok(v)` and `Err(e)` where no name or expected enum claims them.
    pub(crate) fn prelude_call(&mut self, f: &mut FnCx, name: Ident, args: &[ast::Arg], expected: Option<TyId>, span: Span) -> Option<Id<Expr>> {
        match self.text(name.name).as_str() {
            "Some" => Some(self.some_value(f, args, expected, span)),
            "Ok" | "Err" => {
                let def = self.program.std.result?;
                let index = self.variant_index(def, name.name)?;
                match self.instance(def, Vec::new(), expected, span) {
                    Some(ty) => Some(self.variant_value(f, ty, index, args, span)),
                    None => Some(self.error_expr(f, span)),
                }
            }
            _ => None,
        }
    }

    fn some_value(&mut self, f: &mut FnCx, args: &[ast::Arg], expected: Option<TyId>, span: Span) -> Id<Expr> {
        if args.len() != 1 || args[0].name.is_some() {
            self.error(span, "`Some` takes one value");
            return self.error_expr(f, span);
        }
        let hint = expected.and_then(|ty| self.optional_inner(ty));
        let value = self.expr(f, args[0].value, hint);
        let ty = f.body.exprs[value].ty;
        if ty == TyId::ERROR {
            return self.error_expr(f, span);
        }
        let target = match expected {
            Some(expected) if self.optional_inner(expected).is_some() => expected,
            _ => self.program.types.optional(ty),
        };
        self.coerce(f, value, target, span)
    }
}
