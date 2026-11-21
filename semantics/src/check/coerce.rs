use mote_base::{Id, Span};

use super::FnCx;
use crate::cx::Cx;
use crate::defs::{DefId, LangItem};
use crate::program::Program;
use crate::tree::{Conversion, Expr, ExprKind};
use crate::types::{FnTy, TyId, TyKind};

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
        (TyKind::Function(source), TyKind::Function(target)) if function_fits(program, source, target) => Some(vec![(Conversion::Adapt, to)]),
        (TyKind::Adt { def: source, args: from_args }, TyKind::Adt { def: target, args: to_args }) if source == target && covariant(program, *source) => {
            plan(program, from_args[0], to_args[0]).map(|_| vec![(Conversion::Adapt, to)])
        }
        _ => None,
    }
}

/// `Stream` and `Task` are the built-in types that keep a fitting element fitting.
fn covariant(program: &Program, def: DefId) -> bool {
    matches!(program.defs[def].lang, Some(LangItem::Stream | LangItem::Task))
}

/// Whether a function fits a function type: the parameters it takes accept what the type passes, and its result fits.
fn function_fits(program: &Program, source: &FnTy, target: &FnTy) -> bool {
    if source.params.len() != target.params.len() || (target.send && !source.send) {
        return false;
    }
    let params = source.params.iter().zip(&target.params).all(|(have, want)| (!have.is_var || want.is_var) && plan(program, want.ty, have.ty).is_some());
    params && (target.ret == TyId::UNIT || plan(program, source.ret, target.ret).is_some())
}

impl Cx<'_> {
    /// Writes the conversions out as nodes; a `null` literal just takes its context's type.
    pub(crate) fn apply(&mut self, f: &mut FnCx, node: Id<Expr>, steps: Vec<(Conversion, TyId)>) -> Id<Expr> {
        let mut node = node;
        for (conversion, ty) in steps {
            if conversion == Conversion::ToOptional && f.body.exprs[node].kind == ExprKind::Null {
                f.body.exprs[node].ty = ty;
                continue;
            }
            let span = f.body.exprs[node].span;
            node = f.body.expr(ExprKind::Convert { conversion, expr: node }, ty, span);
        }
        node
    }

    /// Makes `node` a value of `to`, or reports the mismatch.
    pub(crate) fn coerce(&mut self, f: &mut FnCx, node: Id<Expr>, to: TyId, span: Span) -> Id<Expr> {
        let from = f.body.exprs[node].ty;
        if self.program.types.has_error(from) {
            return node;
        }
        match plan(self.program, from, to) {
            Some(steps) => self.apply(f, node, steps),
            None => {
                let message = format!("expected {}, found {}", self.show_ty(to), self.show_ty(from));
                self.error(span, message);
                f.body.expr(ExprKind::Error, to, span)
            }
        }
    }
}
