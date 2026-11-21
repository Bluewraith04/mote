use mote_base::{Id, Span};
use mote_syntax as ast;

use super::call::Args;
use super::FnCx;
use crate::comptime;
use crate::cx::Cx;
use crate::defs::{DefId, Receiver};
use crate::tree::{Conversion, Expr, ExprKind, ForKind};
use crate::types::{BuiltinTy, ConstArg, ConstVal, GenericArg, TyId, TyKind};

/// What an expected type asks of a list or array literal.
#[derive(Clone, PartialEq)]
enum Shape {
    List(TyId),
    Array(TyId),
    Fixed(TyId, ConstArg),
}

impl Shape {
    fn element(&self) -> TyId {
        match self {
            Shape::List(element) | Shape::Array(element) | Shape::Fixed(element, _) => *element,
        }
    }
}

impl Cx<'_> {
    /// A canonical std ADT (`Map`, `Set`, `Range`, etc.), or an error when std has not defined it.
    pub(crate) fn std_type(&mut self, def: Option<DefId>, name: &str, args: Vec<TyId>, span: Span) -> TyId {
        match def {
            Some(def) => self.program.types.adt_types(def, args),
            None => {
                self.error(span, format!("std has no `{name}` declaration"));
                TyId::ERROR
            }
        }
    }

    pub(crate) fn without_optional(&self, expected: Option<TyId>) -> Option<TyId> {
        let ty = expected?;
        match self.program.types.kind(ty) {
            TyKind::Optional(inner) => Some(*inner),
            _ => Some(ty),
        }
    }

    fn shape_of(&self, expected: Option<TyId>) -> Option<Shape> {
        let ty = self.without_optional(expected)?;
        match self.program.types.kind(ty) {
            TyKind::Array(element) => Some(Shape::Array(*element)),
            TyKind::FixedArray(element, len) => Some(Shape::Fixed(*element, len.clone())),
            TyKind::Builtin { kind: BuiltinTy::List, args } => Some(Shape::List(args[0])),
            TyKind::Adt { def, args } if self.program.std.span == Some(*def) || self.program.std.mut_span == Some(*def) => {
                args.first().and_then(GenericArg::as_type).map(Shape::Array)
            }
            _ => None,
        }
    }

    pub(crate) fn map_args(&self, ty: TyId) -> Option<(TyId, TyId)> {
        let TyKind::Adt { def, args } = self.program.types.kind(ty) else { return None };
        if self.program.std.map != Some(*def) || args.len() < 2 {
            return None;
        }
        Some((args[0].as_type()?, args[1].as_type()?))
    }

    fn range_arg(&self, ty: TyId) -> Option<TyId> {
        let TyKind::Adt { def, args } = self.program.types.kind(ty) else { return None };
        if self.program.std.range != Some(*def) {
            return None;
        }
        args.first().and_then(GenericArg::as_type)
    }

    /// `[a, b]`: a `List`, or an array where the context asks for one.
    pub(crate) fn list_literal(&mut self, f: &mut FnCx, items: &[Id<ast::Expr>], expected: Option<TyId>, span: Span) -> Id<Expr> {
        let shape = self.shape_of(expected);
        let mut nodes: Vec<Id<Expr>> = Vec::new();
        for &item in items {
            let node = match &shape {
                Some(shape) => self.expr_to(f, item, shape.element()),
                None => {
                    let hint = nodes.first().map(|&first| f.body.exprs[first].ty);
                    self.expr(f, item, hint)
                }
            };
            nodes.push(node);
        }
        let element = match &shape {
            Some(shape) => self.solved(f, shape.element()),
            None if nodes.is_empty() => {
                self.error(span, "an empty list needs its type from context");
                return self.error_expr(f, span);
            }
            None => {
                let (joined, element) = self.join_all(f, &nodes, span, "elements");
                if element == TyId::ERROR {
                    return self.error_expr(f, span);
                }
                nodes = joined;
                element
            }
        };
        match shape {
            Some(Shape::Array(_)) => {
                let ty = self.program.types.array(element);
                f.body.expr(ExprKind::Array(nodes), ty, span)
            }
            Some(Shape::Fixed(_, ref len_arg)) => {
                let len_arg = self.solved_const(f, len_arg);
                let actual = ConstArg::from_u32(nodes.len() as u32);
                match len_arg {
                    ConstArg::Infer(_) => {
                        self.unify_const(f, &len_arg, &actual);
                        let ty = self.program.types.fixed_array_arg(element, actual);
                        f.body.expr(ExprKind::Array(nodes), ty, span)
                    }
                    ConstArg::Val(ref v) => {
                        let expected_len = v.as_u32().unwrap_or(0);
                        if nodes.len() != expected_len as usize {
                            let message = format!("expected {expected_len} elements, found {}", nodes.len());
                            self.error(span, message);
                            return self.error_expr(f, span);
                        }
                        let ty = self.program.types.fixed_array_arg(element, len_arg);
                        f.body.expr(ExprKind::Array(nodes), ty, span)
                    }
                    other => {
                        let ty = self.program.types.fixed_array_arg(element, other);
                        f.body.expr(ExprKind::Array(nodes), ty, span)
                    }
                }
            }
            _ => {
                let ty = self.list_of(element, span);
                f.body.expr(ExprKind::List(nodes), ty, span)
            }
        }
    }

    /// `[v; n]`: an array of `n` copies; `[T; N]` where the context says so.
    pub(crate) fn array_repeat(&mut self, f: &mut FnCx, value: Id<ast::Expr>, len: Id<ast::Expr>, expected: Option<TyId>, span: Span) -> Id<Expr> {
        let shape = self.shape_of(expected).filter(|shape| !matches!(shape, Shape::List(_)));
        let value_span = self.ast(f.module)[value].span;
        let value = match &shape {
            Some(shape) => self.expr_to(f, value, shape.element()),
            None => self.expr(f, value, None),
        };
        let element = f.body.exprs[value].ty;
        if element == TyId::ERROR {
            return self.error_expr(f, span);
        }
        if self.changes_inside(element) {
            let message = format!("`[v; n]` copies its value, and {} can change", self.show_ty(element));
            self.error(value_span, message);
            return self.error_expr(f, span);
        }
        if let Some(Shape::Fixed(_, ref fixed_arg)) = shape {
            let len_span = self.ast(f.module)[len].span;
            let solved_arg = self.solved_const(f, fixed_arg);
            let fixed_arg = comptime::normalize_const_arg(self, &solved_arg);
            if let Ok(ConstVal::Int(n, _)) = comptime::eval_ast_const_in_fn(self, f, len) {
                if n < 0 || n > i128::from(u32::MAX) {
                    self.error(len_span, "fixed array length out of range");
                    return self.error_expr(f, span);
                }
                let actual = ConstArg::from_u32(n as u32);
                match fixed_arg {
                    ConstArg::Infer(_) => {
                        self.unify_const(f, &fixed_arg, &actual);
                        let len_node = f.body.expr(ExprKind::Int(n), TyId::INT, len_span);
                        let ty = self.program.types.fixed_array_arg(element, actual);
                        return f.body.expr(ExprKind::ArrayRepeat { value, len: len_node }, ty, span);
                    }
                    ConstArg::Val(ref v) => {
                        let fixed = v.as_u32().unwrap_or(0);
                        if n != i128::from(fixed) {
                            self.error(len_span, format!("the length must be the literal {fixed}"));
                            return self.error_expr(f, span);
                        }
                        let len_node = f.body.expr(ExprKind::Int(i128::from(fixed)), TyId::INT, len_span);
                        let ty = self.program.types.fixed_array_arg(element, fixed_arg);
                        return f.body.expr(ExprKind::ArrayRepeat { value, len: len_node }, ty, span);
                    }
                    other => {
                        let len_node = f.body.expr(ExprKind::Int(n), TyId::INT, len_span);
                        let ty = self.program.types.fixed_array_arg(element, other);
                        return f.body.expr(ExprKind::ArrayRepeat { value, len: len_node }, ty, span);
                    }
                }
            }
            if let Some(fixed) = fixed_arg.as_u32() {
                self.error(len_span, format!("the length must be the literal {fixed}"));
                return self.error_expr(f, span);
            }
            let len_node = self.expr_to(f, len, TyId::INT);
            let ty = self.program.types.fixed_array_arg(element, fixed_arg);
            return f.body.expr(ExprKind::ArrayRepeat { value, len: len_node }, ty, span);
        }
        let len = self.expr_to(f, len, TyId::INT);
        let ty = self.program.types.array(element);
        f.body.expr(ExprKind::ArrayRepeat { value, len }, ty, span)
    }

    /// `{k: v}`: a `Map`.
    pub(crate) fn map_literal(&mut self, f: &mut FnCx, entries: &[ast::MapEntry], expected: Option<TyId>, span: Span) -> Id<Expr> {
        let hint = self.without_optional(expected).and_then(|ty| self.map_args(ty));
        let mut keys = Vec::new();
        let mut values = Vec::new();
        for entry in entries {
            match hint {
                Some((key, value)) => {
                    keys.push(self.expr_to(f, entry.key, key));
                    values.push(self.expr_to(f, entry.value, value));
                }
                None => {
                    let key_hint = keys.first().map(|&first| f.body.exprs[first].ty);
                    let value_hint = values.first().map(|&first| f.body.exprs[first].ty);
                    keys.push(self.expr(f, entry.key, key_hint));
                    values.push(self.expr(f, entry.value, value_hint));
                }
            }
        }
        let (key, value) = match hint {
            Some(pair) => pair,
            None if keys.is_empty() => {
                self.error(span, "an empty map needs its type from context");
                return self.error_expr(f, span);
            }
            None => {
                let (joined_keys, key) = self.join_all(f, &keys, span, "keys");
                let (joined_values, value) = self.join_all(f, &values, span, "values");
                if key == TyId::ERROR || value == TyId::ERROR {
                    return self.error_expr(f, span);
                }
                keys = joined_keys;
                values = joined_values;
                (key, value)
            }
        };
        let ty = self.std_type(self.program.std.map, "Map", vec![key, value], span);
        if ty == TyId::ERROR {
            return self.error_expr(f, span);
        }
        if hint.is_none() {
            self.check_type_bounds(f, ty, span);
        }
        f.body.expr(ExprKind::Map(keys.into_iter().zip(values).collect()), ty, span)
    }

    /// `a..b` and `a..=b`: a `Range` of integers.
    pub(crate) fn range_expr(&mut self, f: &mut FnCx, start: Id<ast::Expr>, end: Id<ast::Expr>, inclusive: bool, expected: Option<TyId>, span: Span) -> Id<Expr> {
        let hint = self.without_optional(expected).and_then(|ty| self.range_arg(ty));
        let (start, end) = self.operands(f, start, end, hint);
        let (st, et) = (f.body.exprs[start].ty, f.body.exprs[end].ty);
        if st == TyId::ERROR || et == TyId::ERROR {
            return self.error_expr(f, span);
        }
        if st != et || !self.is_integer(st) {
            let message = format!("a range needs integers of one type, found {} and {}", self.show_ty(st), self.show_ty(et));
            self.error(span, message);
            return self.error_expr(f, span);
        }
        let ty = self.std_type(self.program.std.range, "Range", vec![st], span);
        if ty == TyId::ERROR {
            return self.error_expr(f, span);
        }
        f.body.expr(ExprKind::Range { start, end, inclusive }, ty, span)
    }

    /// `xs[i]` on a `List`, an array or a `Map`.
    pub(crate) fn index_expr(&mut self, f: &mut FnCx, object: Id<ast::Expr>, index: Id<ast::Expr>, expected: Option<TyId>, span: Span) -> Id<Expr> {
        let object = self.expr(f, object, None);
        self.index_on(f, object, index, expected, span)
    }

    /// Type-checks `object[index]` given an already-checked `object`, falling back to `__getitem__`.
    pub(crate) fn index_on(&mut self, f: &mut FnCx, object: Id<Expr>, index: Id<ast::Expr>, expected: Option<TyId>, span: Span) -> Id<Expr> {
        let ty = f.body.exprs[object].ty;
        let (key, value) = match self.program.types.kind(ty).clone() {
            TyKind::Error => {
                self.expr(f, index, None);
                return self.error_expr(f, span);
            }
            TyKind::Array(element) | TyKind::FixedArray(element, _) => (TyId::INT, element),
            TyKind::Builtin { kind: BuiltinTy::List, args } => (TyId::INT, args[0]),
            TyKind::String => {
                self.error(span, "a String cannot be indexed; use `slice`");
                return self.error_expr(f, span);
            }
            _ => match self.map_args(ty) {
                Some((k, v)) => (k, v),
                None => {
                    if let Some(name) = self.interner.get("__getitem__")
                        && let Some((method, owner_args)) = self.quiet_method(f, ty, name)
                        && self.receiver_of(method) != Receiver::None
                    {
                        let args = [ast::Arg { name: None, value: index }];
                        return self.call_with(f, method, &owner_args, &[], Some(object), Args::Written(&args), expected, span);
                    }
                    let message = format!("{} cannot be indexed", self.show_ty(ty));
                    self.error(span, message);
                    return self.error_expr(f, span);
                }
            },
        };
        let index = self.expr_to(f, index, key);
        f.body.expr(ExprKind::Index { object, index }, value, span)
    }

    /// Validates the target and bounds of a slice expression or slice assignment.
    #[allow(clippy::type_complexity)]
    pub(crate) fn check_slice_parts(
        &mut self,
        f: &mut FnCx,
        object: Id<ast::Expr>,
        start: Option<Id<ast::Expr>>,
        end: Option<Id<ast::Expr>>,
        step: Option<Id<ast::Expr>>,
        span: Span,
    ) -> Option<(Id<Expr>, TyId, Option<Id<Expr>>, Option<Id<Expr>>, Option<Id<Expr>>)> {
        let object = self.expr(f, object, None);
        let ty = f.body.exprs[object].ty;
        let bound = |this: &mut Self, f: &mut FnCx, bound: Option<Id<ast::Expr>>| bound.map(|bound| this.expr_to(f, bound, TyId::INT));
        let (start, end, step) = (bound(self, f, start), bound(self, f, end), bound(self, f, step));
        if ty == TyId::ERROR {
            return None;
        }
        if step.is_some() && ty == TyId::STRING {
            self.error(span, "a String slice takes no step");
            return None;
        }
        if step.is_some() && end.is_none() {
            self.error(span, "a slice with a step needs its end");
            return None;
        }
        Some((object, ty, start, end, step))
    }

    /// `xs[a..b]`, `xs[a..=b]`, `xs[..]`, `xs[a..b..s]`: dispatches to `__slice__` (except `[T; N]` fallback).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn slice_expr(&mut self, f: &mut FnCx, object: Id<ast::Expr>, start: Option<Id<ast::Expr>>, end: Option<Id<ast::Expr>>, inclusive: bool, step: Option<Id<ast::Expr>>, expected: Option<TyId>, span: Span) -> Id<Expr> {
        let Some((object, ty, start, end, step)) = self.check_slice_parts(f, object, start, end, step, span) else {
            return self.error_expr(f, span);
        };
        let array_element = match self.program.types.kind(ty) {
            TyKind::Array(element) | TyKind::FixedArray(element, _) => Some(*element),
            _ => None,
        };
        let is_fixed_array = matches!(self.program.types.kind(ty), TyKind::FixedArray(..));
        if !is_fixed_array
            && let Some(name) = self.interner.get("__slice__")
            && let Some((method, owner_args)) = self.quiet_method(f, ty, name)
            && self.receiver_of(method) != Receiver::None
        {
            let opt_int = self.program.types.optional(TyId::INT);
            let opt_arg = |body: &mut crate::tree::Body, v: Option<Id<Expr>>| match v {
                Some(expr) => {
                    let s = body.exprs[expr].span;
                    body.expr(ExprKind::Convert { conversion: Conversion::ToOptional, expr }, opt_int, s)
                }
                None => body.expr(ExprKind::Null, opt_int, span),
            };
            let start_arg = opt_arg(&mut f.body, start);
            let end_arg = opt_arg(&mut f.body, end);
            let step_arg = opt_arg(&mut f.body, step);
            let inc_arg = f.body.expr(ExprKind::Bool(inclusive), TyId::BOOL, span);
            let checked = [start_arg, end_arg, inc_arg, step_arg];
            return self.call_with(f, method, &owner_args, &[], Some(object), Args::Checked(&checked), expected, span);
        }
        let is_list = array_element.is_some() || matches!(self.program.types.kind(ty), TyKind::Builtin { kind: BuiltinTy::List, .. });
        if !is_list && ty != TyId::STRING {
            let message = format!("{} cannot be sliced", self.show_ty(ty));
            self.error(span, message);
            return self.error_expr(f, span);
        }
        let ty = match array_element {
            Some(element) => self.program.types.array(element),
            None => ty,
        };
        f.body.expr(ExprKind::Slice { object, start, end, inclusive, step }, ty, span)
    }

    /// How a `for` steps through a value, and what each step yields.
    pub(crate) fn iteration(&mut self, f: &mut FnCx, iterable: &mut Id<Expr>, span: Span) -> Option<(ForKind, TyId)> {
        let ty = f.body.exprs[*iterable].ty;
        match self.program.types.kind(ty).clone() {
            TyKind::Error => None,
            TyKind::Array(element) | TyKind::FixedArray(element, _) => Some((ForKind::Indexed, element)),
            TyKind::Builtin { kind: BuiltinTy::List, args } => Some((ForKind::Indexed, args[0])),
            TyKind::Builtin { kind: BuiltinTy::Stream, args } => Some((ForKind::Stream, args[0])),
            TyKind::Builtin { kind: BuiltinTy::Receiver, args } => Some((ForKind::Receiver, args[0])),
            _ => {
                if let Some(elem) = self.range_arg(ty) {
                    return Some((ForKind::Range, elem));
                }
                if let Some(name) = self.interner.get("stream")
                    && let Some((method, owner_args)) = self.quiet_method(f, ty, name)
                    && let Some(function) = self.program.defs[method].function()
                {
                    let receiver_ok = match function.receiver {
                        Receiver::Read => true,
                        Receiver::Write => !Self::is_place(f, *iterable) || self.writable(f, *iterable),
                        Receiver::None => false,
                    };
                    if receiver_ok && function.generics.is_empty() && function.params.iter().all(|p| p.default.is_some()) {
                        let ret_ty = self.program.types.subst_generic(function.ret, &owner_args);
                        if let TyKind::Builtin { kind: BuiltinTy::Stream, args } = self.program.types.kind(ret_ty).clone()
                            && args.len() == 1
                        {
                            *iterable = self.call_with(f, method, &owner_args, &[], Some(*iterable), Args::Checked(&[]), None, span);
                            return Some((ForKind::Stream, args[0]));
                        }
                    }
                }
                let message = format!("{} is not iterable", self.show_ty(ty));
                self.error(span, message);
                None
            }
        }
    }
}
