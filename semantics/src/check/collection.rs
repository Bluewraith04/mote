use mote_base::{Id, Span};
use mote_syntax as ast;

use super::FnCx;
use crate::cx::Cx;
use crate::defs::LangItem;
use crate::tree::{Expr, ExprKind, ForKind};
use crate::types::{TyId, TyKind};

/// What an expected type asks of a list or array literal.
#[derive(Clone, Copy, PartialEq)]
enum Shape {
    List(TyId),
    Array(TyId),
    Fixed(TyId, u32),
}

impl Shape {
    fn element(self) -> TyId {
        match self {
            Shape::List(element) | Shape::Array(element) | Shape::Fixed(element, _) => element,
        }
    }
}

impl Cx<'_> {
    /// The built-in generic type a std declaration stands for, or an error when std lacks it.
    pub(crate) fn lang_type(&mut self, item: LangItem, args: Vec<TyId>, span: Span) -> TyId {
        match self.program.lang_item(item) {
            Some(def) => self.program.types.adt(def, args),
            None => {
                self.error(span, format!("std has no `{}` declaration", item.name()));
                TyId::ERROR
            }
        }
    }

    /// The lang item and arguments of a type that is one of std's built-in generics.
    pub(crate) fn lang_adt(&self, ty: TyId) -> Option<(LangItem, Vec<TyId>)> {
        let TyKind::Adt { def, args } = self.program.types.kind(ty) else { return None };
        Some((self.program.defs[*def].lang?, args.clone()))
    }

    fn without_optional(&self, expected: Option<TyId>) -> Option<TyId> {
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
            TyKind::FixedArray(element, len) => Some(Shape::Fixed(*element, *len)),
            _ => match self.lang_adt(ty)? {
                (LangItem::List, args) => Some(Shape::List(args[0])),
                _ => None,
            },
        }
    }

    /// `[a, b]`: a `List`, or an array where the context asks for one.
    pub(crate) fn list_literal(&mut self, f: &mut FnCx, items: &[Id<ast::Expr>], expected: Option<TyId>, span: Span) -> Id<Expr> {
        let shape = self.shape_of(expected);
        let mut nodes: Vec<Id<Expr>> = Vec::new();
        for &item in items {
            let node = match shape {
                Some(shape) => self.expr_to(f, item, shape.element()),
                None => {
                    let hint = nodes.first().map(|&first| f.body.exprs[first].ty);
                    self.expr(f, item, hint)
                }
            };
            nodes.push(node);
        }
        let element = match shape {
            Some(shape) => shape.element(),
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
            Some(Shape::Fixed(_, len)) => {
                if nodes.len() != len as usize {
                    let message = format!("expected {len} elements, found {}", nodes.len());
                    self.error(span, message);
                    return self.error_expr(f, span);
                }
                let ty = self.program.types.fixed_array(element, len);
                f.body.expr(ExprKind::Array(nodes), ty, span)
            }
            _ => {
                let ty = self.list_of(element, span);
                f.body.expr(ExprKind::List(nodes), ty, span)
            }
        }
    }

    /// `[v; n]`: an array of `n` copies; `[T; N]` where the context says so, with `n` the literal `N`.
    pub(crate) fn array_repeat(&mut self, f: &mut FnCx, value: Id<ast::Expr>, len: Id<ast::Expr>, expected: Option<TyId>, span: Span) -> Id<Expr> {
        let shape = self.shape_of(expected).filter(|shape| !matches!(shape, Shape::List(_)));
        let value_span = self.ast(f.module)[value].span;
        let value = match shape {
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
        if let Some(Shape::Fixed(_, fixed)) = shape {
            let len_span = self.ast(f.module)[len].span;
            let written = match &self.ast(f.module)[len].kind {
                ast::ExprKind::Int(literal) => Some(literal.value),
                _ => None,
            };
            if written != Some(u128::from(fixed)) {
                self.error(len_span, format!("the length must be the literal {fixed}"));
                return self.error_expr(f, span);
            }
            let len = f.body.expr(ExprKind::Int(i128::from(fixed)), TyId::INT, len_span);
            let ty = self.program.types.fixed_array(element, fixed);
            return f.body.expr(ExprKind::ArrayRepeat { value, len }, ty, span);
        }
        let len = self.expr_to(f, len, TyId::INT);
        let ty = self.program.types.array(element);
        f.body.expr(ExprKind::ArrayRepeat { value, len }, ty, span)
    }

    /// `{k: v}`: a `Map`.
    pub(crate) fn map_literal(&mut self, f: &mut FnCx, entries: &[ast::MapEntry], expected: Option<TyId>, span: Span) -> Id<Expr> {
        let hint = self.without_optional(expected).and_then(|ty| match self.lang_adt(ty)? {
            (LangItem::Map, args) => Some((args[0], args[1])),
            _ => None,
        });
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
        let ty = self.lang_type(LangItem::Map, vec![key, value], span);
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
        let hint = self.without_optional(expected).and_then(|ty| match self.lang_adt(ty)? {
            (LangItem::Range, args) => Some(args[0]),
            _ => None,
        });
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
        let ty = self.lang_type(LangItem::Range, vec![st], span);
        if ty == TyId::ERROR {
            return self.error_expr(f, span);
        }
        f.body.expr(ExprKind::Range { start, end, inclusive }, ty, span)
    }

    /// `xs[i]` on a `List`, an array or a `Map`.
    pub(crate) fn index_expr(&mut self, f: &mut FnCx, object: Id<ast::Expr>, index: Id<ast::Expr>, span: Span) -> Id<Expr> {
        let object = self.expr(f, object, None);
        let ty = f.body.exprs[object].ty;
        let (key, value) = match self.program.types.kind(ty).clone() {
            TyKind::Error => {
                self.expr(f, index, None);
                return self.error_expr(f, span);
            }
            TyKind::Array(element) | TyKind::FixedArray(element, _) => (TyId::INT, element),
            TyKind::String => {
                self.error(span, "a String cannot be indexed; use `slice`");
                return self.error_expr(f, span);
            }
            _ => match self.lang_adt(ty) {
                Some((LangItem::List, args)) => (TyId::INT, args[0]),
                Some((LangItem::Map, args)) => (args[0], args[1]),
                _ => {
                    let message = format!("{} cannot be indexed", self.show_ty(ty));
                    self.error(span, message);
                    return self.error_expr(f, span);
                }
            },
        };
        let index = self.expr_to(f, index, key);
        f.body.expr(ExprKind::Index { object, index }, value, span)
    }

    /// `xs[a..b]`, `xs[a..=b]`, `xs[..]`, `xs[a..b..s]`: a copy of a `List`, a `String`, or a `[T]` from an array.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn slice_expr(&mut self, f: &mut FnCx, object: Id<ast::Expr>, start: Option<Id<ast::Expr>>, end: Option<Id<ast::Expr>>, inclusive: bool, step: Option<Id<ast::Expr>>, span: Span) -> Id<Expr> {
        let object = self.expr(f, object, None);
        let ty = f.body.exprs[object].ty;
        let array_element = match self.program.types.kind(ty) {
            TyKind::Array(element) | TyKind::FixedArray(element, _) => Some(*element),
            _ => None,
        };
        let is_list = array_element.is_some() || matches!(self.lang_adt(ty), Some((LangItem::List, _)));
        let bound = |this: &mut Self, f: &mut FnCx, bound: Option<Id<ast::Expr>>| bound.map(|bound| this.expr_to(f, bound, TyId::INT));
        let (start, end, step) = (bound(self, f, start), bound(self, f, end), bound(self, f, step));
        if ty == TyId::ERROR {
            return self.error_expr(f, span);
        }
        if !is_list && ty != TyId::STRING {
            let message = format!("{} cannot be sliced", self.show_ty(ty));
            self.error(span, message);
            return self.error_expr(f, span);
        }
        if step.is_some() && !is_list {
            self.error(span, "a String slice takes no step");
            return self.error_expr(f, span);
        }
        if step.is_some() && end.is_none() {
            self.error(span, "a slice with a step needs its end");
            return self.error_expr(f, span);
        }
        let ty = match array_element {
            Some(element) => self.program.types.array(element),
            None => ty,
        };
        f.body.expr(ExprKind::Slice { object, start, end, inclusive, step }, ty, span)
    }

    /// How a `for` steps through a value, and what each step yields.
    pub(crate) fn iteration(&mut self, ty: TyId, span: Span) -> Option<(ForKind, TyId)> {
        match self.program.types.kind(ty).clone() {
            TyKind::Error => None,
            TyKind::Array(element) | TyKind::FixedArray(element, _) => Some((ForKind::Indexed, element)),
            _ => match self.lang_adt(ty) {
                Some((LangItem::Range, args)) => Some((ForKind::Range, args[0])),
                Some((LangItem::List, args)) => Some((ForKind::Indexed, args[0])),
                Some((LangItem::Stream, args)) => Some((ForKind::Stream, args[0])),
                Some((LangItem::Receiver, args)) => Some((ForKind::Receiver, args[0])),
                _ => {
                    let message = format!("{} is not iterable", self.show_ty(ty));
                    self.error(span, message);
                    None
                }
            },
        }
    }
}
