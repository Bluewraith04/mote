use mote_base::{Id, Interner, Span};

use super::FnCx;
use crate::cx::Cx;
use crate::program::Program;
use crate::tree::{Callee, Conversion, Expr, ExprKind, FnRef};
use crate::types::{BuiltinTy, FnTy, GenericArg, TyId, TyKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewCoercion {
    StrToView,
    ArrayToSpan(TyId),
    ListToSpan(TyId),
    BufferToSpan(TyId),
    MutSpanToSpan(TyId),
    BytesToSpan,
    ArrayToMutSpan(TyId),
    ListToMutSpan(TyId),
    BufferToMutSpan(TyId),
    BytesToMutSpan,
}

/// Identifies zero-copy owner-to-view coercions between `from` and `to`.
pub fn view_coercion_kind(program: &Program, from: TyId, to: TyId) -> Option<ViewCoercion> {
    let std = &program.std;
    let types = &program.types;
    match (types.kind(from), types.kind(to)) {
        (TyKind::String, TyKind::Adt { def, .. }) if std.str_view == Some(*def) => Some(ViewCoercion::StrToView),
        (_, TyKind::Adt { def, args }) if std.span == Some(*def) => {
            let elem = args.first().and_then(GenericArg::as_type)?;
            match types.kind(from) {
                TyKind::Array(a_elem) if *a_elem == elem => Some(ViewCoercion::ArrayToSpan(elem)),
                TyKind::Builtin { kind: BuiltinTy::List, args: l_args } if l_args.first() == Some(&elem) => Some(ViewCoercion::ListToSpan(elem)),
                TyKind::Adt { def: f_def, args: f_args } if std.buffer == Some(*f_def) && f_args.first().and_then(GenericArg::as_type) == Some(elem) => {
                    Some(ViewCoercion::BufferToSpan(elem))
                }
                TyKind::Adt { def: f_def, args: f_args } if std.mut_span == Some(*f_def) && f_args.first().and_then(GenericArg::as_type) == Some(elem) => {
                    Some(ViewCoercion::MutSpanToSpan(elem))
                }
                TyKind::Adt { def: f_def, .. } if std.bytes == Some(*f_def) && elem == TyId::UINT8 => Some(ViewCoercion::BytesToSpan),
                _ => None,
            }
        }
        (_, TyKind::Adt { def, args }) if std.mut_span == Some(*def) => {
            let elem = args.first().and_then(GenericArg::as_type)?;
            match types.kind(from) {
                TyKind::Array(a_elem) if *a_elem == elem => Some(ViewCoercion::ArrayToMutSpan(elem)),
                TyKind::Builtin { kind: BuiltinTy::List, args: l_args } if l_args.first() == Some(&elem) => Some(ViewCoercion::ListToMutSpan(elem)),
                TyKind::Adt { def: f_def, args: f_args } if std.buffer == Some(*f_def) && f_args.first().and_then(GenericArg::as_type) == Some(elem) => {
                    Some(ViewCoercion::BufferToMutSpan(elem))
                }
                TyKind::Adt { def: f_def, .. } if std.bytes == Some(*f_def) && elem == TyId::UINT8 => Some(ViewCoercion::BytesToMutSpan),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Returns whether coercing `from` to `to` produces a mutable `MutSpan` view.
pub fn needs_writable_view(program: &Program, from: TyId, to: TyId) -> bool {
    matches!(
        view_coercion_kind(program, from, to),
        Some(
            ViewCoercion::ArrayToMutSpan(_)
                | ViewCoercion::ListToMutSpan(_)
                | ViewCoercion::BufferToMutSpan(_)
                | ViewCoercion::BytesToMutSpan
        )
    )
}

/// Resolves the stdlib constructor method implementing an owner-to-view coercion.
pub fn view_coercion_fn(program: &Program, interner: &Interner, from: TyId, to: TyId) -> Option<FnRef> {
    let kind = view_coercion_kind(program, from, to)?;
    let std = &program.std;
    let (owner, method_name, args) = match kind {
        ViewCoercion::StrToView => (std.str_view?, "of", Vec::new()),
        ViewCoercion::ArrayToSpan(elem) => (std.span?, "from_array", vec![GenericArg::Type(elem)]),
        ViewCoercion::ListToSpan(elem) => (std.span?, "from_list", vec![GenericArg::Type(elem)]),
        ViewCoercion::BufferToSpan(elem) => (std.span?, "from_buffer", vec![GenericArg::Type(elem)]),
        ViewCoercion::MutSpanToSpan(elem) => (std.mut_span?, "as_span", vec![GenericArg::Type(elem)]),
        ViewCoercion::BytesToSpan => (std.bytes?, "span", Vec::new()),
        ViewCoercion::ArrayToMutSpan(elem) => (std.mut_span?, "from_array", vec![GenericArg::Type(elem)]),
        ViewCoercion::ListToMutSpan(elem) => (std.mut_span?, "from_list", vec![GenericArg::Type(elem)]),
        ViewCoercion::BufferToMutSpan(elem) => (std.mut_span?, "from_buffer", vec![GenericArg::Type(elem)]),
        ViewCoercion::BytesToMutSpan => (std.bytes?, "mut_span", Vec::new()),
    };
    let sym = interner.get(method_name)?;
    let def = *program.methods_named(owner, sym).first()?;
    Some(FnRef { def, args })
}

fn steps_need_writable_view(program: &Program, start: TyId, steps: &[(Conversion, TyId)]) -> bool {
    let mut current = start;
    for &(conversion, next) in steps {
        if conversion == Conversion::ToView && needs_writable_view(program, current, next) {
            return true;
        }
        current = next;
    }
    false
}

/// The conversions, innermost first, that turn a value of `from` into `to`; `None` when it does not fit.
pub fn plan(program: &Program, from: TyId, to: TyId) -> Option<Vec<(Conversion, TyId)>> {
    let types = &program.types;
    if from == to || from == TyId::ERROR || to == TyId::ERROR {
        return Some(Vec::new());
    }
    match (types.kind(from), types.kind(to)) {
        (_, TyKind::Optional(inner)) => {
            if from == TyId::UNIT {
                return Some(vec![(Conversion::ToOptional, to)]);
            }
            let mut steps = plan(program, from, *inner)?;
            steps.push((Conversion::ToOptional, to));
            Some(steps)
        }
        (TyKind::Union(from_members), TyKind::Union(members)) if from_members.iter().all(|member| members.contains(member)) => Some(vec![(Conversion::WidenUnion, to)]),
        (_, TyKind::Union(members)) if members.contains(&from) => Some(vec![(Conversion::ToUnion, to)]),
        (TyKind::Union(_), TyKind::Any) => Some(vec![(Conversion::WidenUnion, to)]),
        (_, TyKind::Any) => Some(vec![(Conversion::ToUnion, to)]),
        (TyKind::Function(source), TyKind::Function(target)) if function_fits(program, source, target) => Some(vec![(Conversion::Adapt, to)]),
        (TyKind::Builtin { kind: source, args: from_args }, TyKind::Builtin { kind: target, args: to_args })
            if source == target && matches!(source, BuiltinTy::Stream | BuiltinTy::Task) =>
        {
            plan(program, from_args[0], to_args[0]).map(|_| vec![(Conversion::Adapt, to)])
        }
        _ if view_coercion_kind(program, from, to).is_some() => Some(vec![(Conversion::ToView, to)]),
        _ => None,
    }
}

/// Whether a function fits a function type: the parameters it takes accept what the type passes, and its result fits.
fn function_fits(program: &Program, source: &FnTy, target: &FnTy) -> bool {
    if source.params.len() != target.params.len() || (target.send && !source.send) {
        return false;
    }
    let params = source.params.iter().zip(&target.params).all(|(have, want)| {
        (!have.is_var || want.is_var)
            && plan(program, want.ty, have.ty).is_some_and(|steps| want.is_var || !steps_need_writable_view(program, want.ty, &steps))
    });
    params && (target.ret == TyId::UNIT || plan(program, source.ret, target.ret).is_some())
}

impl Cx<'_> {
    /// Writes the conversions out as nodes; a `null` literal just takes its context's type.
    pub(crate) fn apply(&mut self, f: &mut FnCx, node: Id<Expr>, steps: Vec<(Conversion, TyId)>) -> Id<Expr> {
        self.apply_checked(f, node, steps, true)
    }

    fn apply_checked(&mut self, f: &mut FnCx, node: Id<Expr>, steps: Vec<(Conversion, TyId)>, check_writable: bool) -> Id<Expr> {
        let mut node = node;
        for (conversion, ty) in steps {
            if conversion == Conversion::ToOptional && f.body.exprs[node].kind == ExprKind::Null {
                f.body.exprs[node].ty = ty;
                continue;
            }
            let span = f.body.exprs[node].span;
            if conversion == Conversion::ToView {
                let from = f.body.exprs[node].ty;
                if check_writable && needs_writable_view(self.program, from, ty) && Self::is_place(f, node) && !self.writable(f, node) {
                    self.error(span, "this place has no `var` path to write through");
                }
                if let Some(fn_ref) = view_coercion_fn(self.program, self.interner, from, ty) {
                    node = f.body.expr(ExprKind::Call { callee: Callee::Function(fn_ref), args: vec![node] }, ty, span);
                    continue;
                }
            }
            node = f.body.expr(ExprKind::Convert { conversion, expr: node }, ty, span);
        }
        node
    }

    /// Makes `node` a value of `to`, or reports the mismatch.
    pub(crate) fn coerce(&mut self, f: &mut FnCx, node: Id<Expr>, to: TyId, span: Span) -> Id<Expr> {
        self.coerce_arg(f, node, to, span, true)
    }

    /// Makes argument `node` fit `to`, skipping duplicate place writability checks when the caller checked `var`.
    pub(crate) fn coerce_arg(&mut self, f: &mut FnCx, node: Id<Expr>, to: TyId, span: Span, check_writable: bool) -> Id<Expr> {
        let from = f.body.exprs[node].ty;
        if self.program.types.has_error(from) {
            return node;
        }
        match plan(self.program, from, to) {
            Some(steps) => self.apply_checked(f, node, steps, check_writable),
            None => {
                let message = format!("expected {}, found {}", self.show_ty(to), self.show_ty(from));
                self.error(span, message);
                f.body.expr(ExprKind::Error, to, span)
            }
        }
    }
}
