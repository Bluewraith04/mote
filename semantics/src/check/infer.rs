use mote_base::{Id, Span, Symbol};
use mote_syntax as ast;

use super::FnCx;
use crate::cx::Cx;
use crate::defs::{Bound, DefId, DefKind, GenericParam, ModuleId};
use crate::types::{BuiltinTy, ConstArg, GenericArg, TyId, TyKind};

impl Cx<'_> {
    /// A new variable for a type argument a call has to find.
    pub(crate) fn fresh(&mut self, f: &mut FnCx) -> TyId {
        f.infer.push(None);
        self.program.types.intern(TyKind::Infer((f.infer.len() - 1) as u32))
    }

    /// A new variable for a `const` generic argument a call has to find.
    pub(crate) fn fresh_const(&mut self, f: &mut FnCx) -> ConstArg {
        f.infer_consts.push(None);
        ConstArg::Infer((f.infer_consts.len() - 1) as u32)
    }

    /// The type with every variable already found put in.
    pub(crate) fn solved(&mut self, f: &FnCx, ty: TyId) -> TyId {
        self.program.types.solve_generic(ty, &f.infer, &f.infer_consts)
    }

    pub(crate) fn solved_const(&mut self, f: &FnCx, arg: &ConstArg) -> ConstArg {
        self.program.types.solve_const(arg, &f.infer, &f.infer_consts)
    }

    pub(crate) fn solved_generic(&mut self, f: &FnCx, arg: &GenericArg) -> GenericArg {
        match arg {
            GenericArg::Type(t) => GenericArg::Type(self.solved(f, *t)),
            GenericArg::Const(c) => GenericArg::Const(self.solved_const(f, c)),
        }
    }

    pub(crate) fn list_of(&mut self, element: TyId, _span: Span) -> TyId {
        self.program.types.builtin(crate::types::BuiltinTy::List, vec![element])
    }

    /// The names of a function's generic parameters in numbering order.
    pub(crate) fn generic_names(&self, def: DefId) -> Vec<Symbol> {
        let mut names = Vec::new();
        if let Some(ext) = self.program.defs[def].extend {
            names.extend(self.program.extends[ext].generics.iter().map(GenericParam::name));
        } else if let Some(owner) = self.program.defs[def].owner {
            match &self.program.defs[owner].kind {
                DefKind::Trait(item) => {
                    names.push(self.self_name);
                    names.extend(item.generics.iter().map(GenericParam::name));
                }
                DefKind::Record(record) => names.extend(record.generics.iter().map(GenericParam::name)),
                DefKind::Enum(en) => names.extend(en.generics.iter().map(GenericParam::name)),
                _ => {}
            }
        }
        if let Some(function) = self.program.defs[def].function() {
            names.extend(function.generics.iter().map(GenericParam::name));
        }
        names
    }

    /// The traits bounding each parameter a function's body sees, by position.
    pub(crate) fn param_bounds(&mut self, def: DefId) -> Vec<Vec<Bound>> {
        let mut bounds = Vec::new();
        if let Some(ext) = self.program.defs[def].extend {
            bounds.extend(self.program.extends[ext].generics.iter().map(|p| p.bounds().to_vec()));
        } else if let Some(owner) = self.program.defs[def].owner {
            match &self.program.defs[owner].kind {
                DefKind::Trait(item) => {
                    let names: Vec<Symbol> = item.generics.iter().map(GenericParam::name).collect();
                    let args = names.iter().enumerate().map(|(index, &name)| self.program.types.param(index as u32 + 1, name)).collect();
                    let DefKind::Trait(item) = &self.program.defs[owner].kind else { unreachable!("a trait") };
                    bounds.push(vec![Bound { def: owner, args }]);
                    bounds.extend(item.generics.iter().map(|param| param.bounds().to_vec()));
                }
                DefKind::Record(record) => bounds.extend(record.generics.iter().map(|param| param.bounds().to_vec())),
                DefKind::Enum(en) => bounds.extend(en.generics.iter().map(|param| param.bounds().to_vec())),
                _ => {}
            }
        }
        if let Some(function) = self.program.defs[def].function() {
            bounds.extend(function.generics.iter().map(|param| param.bounds().to_vec()));
        }
        bounds
    }

    /// A function's own generic arguments: the ones written, or fresh inference variables.
    pub(crate) fn own_generic_args(&mut self, f: &mut FnCx, def: DefId, written: &[GenericArg], span: Span) -> Option<Vec<GenericArg>> {
        let params = self.program.defs[def].function().map(|function| function.generics.clone()).unwrap_or_default();
        if written.is_empty() {
            return Some(
                params
                    .iter()
                    .map(|p| match p {
                        GenericParam::Type { .. } => GenericArg::Type(self.fresh(f)),
                        GenericParam::Const { .. } => GenericArg::Const(self.fresh_const(f)),
                    })
                    .collect(),
            );
        }
        if written.len() != params.len() {
            let name = self.text(self.program.defs[def].name);
            let message = match params.len() {
                0 => format!("`{name}` takes no type arguments"),
                1 => format!("`{name}` takes 1 type argument"),
                count => format!("`{name}` takes {count} type arguments"),
            };
            self.error(span, message);
            return None;
        }
        for (arg, param) in written.iter().zip(&params) {
            match (arg, param) {
                (GenericArg::Type(_), GenericParam::Const { .. }) => {
                    self.error(span, format!("expected a constant expression for `{}`", self.text(param.name())));
                    return None;
                }
                (GenericArg::Const(_), GenericParam::Type { .. }) => {
                    self.error(span, format!("expected a type for `{}`", self.text(param.name())));
                    return None;
                }
                _ => {}
            }
        }
        Some(written.to_vec())
    }

    /// Finds the variables that make `param` and `arg` one type; `false` when no choice does, with nothing changed.
    pub(crate) fn unify(&mut self, f: &mut FnCx, param: TyId, arg: TyId) -> bool {
        let saved_types = f.infer.clone();
        let saved_consts = f.infer_consts.clone();
        if self.unify_in(f, param, arg, false) {
            return true;
        }
        f.infer = saved_types;
        f.infer_consts = saved_consts;
        false
    }

    pub(crate) fn unify_const(&mut self, f: &mut FnCx, param: &ConstArg, arg: &ConstArg) -> bool {
        let solved_p = self.solved_const(f, param);
        let solved_a = self.solved_const(f, arg);
        let p = crate::comptime::normalize_const_arg(self, &solved_p);
        let a = crate::comptime::normalize_const_arg(self, &solved_a);
        if p == a {
            return true;
        }
        match (p, a) {
            (ConstArg::Infer(v), other) | (other, ConstArg::Infer(v)) => {
                if matches!(other, ConstArg::Infer(o) if o == v) {
                    return true;
                }
                f.infer_consts[v as usize] = Some(other);
                true
            }
            (ConstArg::Expr(ce1), ConstArg::Expr(ce2)) => {
                ce1.params == ce2.params
                    && ce1.args.len() == ce2.args.len()
                    && self.ast_expr_eq(ce1.module, ce1.expr, ce2.module, ce2.expr)
                    && self.unify_generic_args(f, &ce1.args, &ce2.args)
            }
            _ => false,
        }
    }

    fn ast_expr_eq(&self, ma: ModuleId, a: Id<ast::Expr>, mb: ModuleId, b: Id<ast::Expr>) -> bool {
        let ka = &self.ast(ma)[a].kind;
        let kb = &self.ast(mb)[b].kind;
        match (ka, kb) {
            (ast::ExprKind::Paren(ia), _) => self.ast_expr_eq(ma, *ia, mb, b),
            (_, ast::ExprKind::Paren(ib)) => self.ast_expr_eq(ma, a, mb, *ib),
            (ast::ExprKind::Int(la), ast::ExprKind::Int(lb)) => la.value == lb.value,
            (ast::ExprKind::Bool(xa), ast::ExprKind::Bool(xb)) => xa == xb,
            (ast::ExprKind::String(xa), ast::ExprKind::String(xb)) => xa == xb,
            (ast::ExprKind::Null, ast::ExprKind::Null) => true,
            (ast::ExprKind::Ident(xa), ast::ExprKind::Ident(xb)) => xa == xb,
            (ast::ExprKind::Unary { op: oa, operand: xa }, ast::ExprKind::Unary { op: ob, operand: xb }) => {
                oa == ob && self.ast_expr_eq(ma, *xa, mb, *xb)
            }
            (ast::ExprKind::Binary { op: oa, left: la, right: ra }, ast::ExprKind::Binary { op: ob, left: lb, right: rb }) => {
                oa == ob && self.ast_expr_eq(ma, *la, mb, *lb) && self.ast_expr_eq(ma, *ra, mb, *rb)
            }
            (ast::ExprKind::Tuple(xs), ast::ExprKind::Tuple(ys)) => {
                xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| self.ast_expr_eq(ma, *x, mb, *y))
            }
            _ => false,
        }
    }

    fn unify_generic_args(&mut self, f: &mut FnCx, params: &[GenericArg], args: &[GenericArg]) -> bool {
        let mut ok = true;
        for (p, a) in params.iter().zip(args) {
            match (p, a) {
                (GenericArg::Type(pt), GenericArg::Type(at)) => ok &= self.unify_in(f, *pt, *at, true),
                (GenericArg::Const(pc), GenericArg::Const(ac)) => ok &= self.unify_const(f, pc, ac),
                _ => return false,
            }
        }
        ok
    }

    fn unify_in(&mut self, f: &mut FnCx, param: TyId, arg: TyId, strict_unit: bool) -> bool {
        let param = self.solved(f, param);
        let arg = self.solved(f, arg);
        if param == arg {
            return true;
        }
        let (p, a) = (self.program.types.kind(param).clone(), self.program.types.kind(arg).clone());
        match (p, a) {
            (TyKind::Error, _) | (_, TyKind::Error) => true,
            (TyKind::Infer(_), TyKind::Unit) if !strict_unit => true,
            (TyKind::Infer(variable), _) => self.assign_variable(f, variable, arg),
            (_, TyKind::Infer(variable)) => self.assign_variable(f, variable, param),
            (TyKind::Optional(p), TyKind::Optional(a)) => self.unify_in(f, p, a, true),
            (TyKind::Optional(p), _) => arg == TyId::UNIT || self.unify_in(f, p, arg, true),
            (TyKind::Builtin { kind: pk, args: pa }, TyKind::Builtin { kind: ak, args: aa }) if pk == ak && pa.len() == aa.len() => self.unify_all(f, &pa, &aa),
            (TyKind::Adt { def: pd, args: pa }, TyKind::Adt { def: ad, args: aa }) if pd == ad && pa.len() == aa.len() => self.unify_generic_args(f, &pa, &aa),
            (TyKind::Adt { def: pd, args: pa }, a_kind)
                if (self.program.std.span == Some(pd) || self.program.std.mut_span == Some(pd)) && pa.len() == 1 =>
            {
                let Some(p_elem) = pa[0].as_type() else { return false };
                let is_span = self.program.std.span == Some(pd);
                match a_kind {
                    TyKind::Array(a_elem) => self.unify_in(f, p_elem, a_elem, true),
                    TyKind::Builtin { kind: BuiltinTy::List, args: aa } if aa.len() == 1 => self.unify_in(f, p_elem, aa[0], true),
                    TyKind::Adt { def: ad, args: aa }
                        if aa.len() == 1 && (self.program.std.buffer == Some(ad) || (is_span && self.program.std.mut_span == Some(ad))) =>
                    {
                        aa[0].as_type().is_some_and(|a_elem| self.unify_in(f, p_elem, a_elem, true))
                    }
                    TyKind::Adt { def: ad, .. } if self.program.std.bytes == Some(ad) => self.unify_in(f, p_elem, TyId::UINT8, true),
                    _ => false,
                }
            }
            (TyKind::Tuple(p), TyKind::Tuple(a)) if p.len() == a.len() => self.unify_all(f, &p, &a),
            (TyKind::Array(p), TyKind::Array(a)) => self.unify_in(f, p, a, true),
            (TyKind::FixedArray(p, n), TyKind::FixedArray(a, m)) => self.unify_in(f, p, a, true) && self.unify_const(f, &n, &m),
            (TyKind::Function(p), TyKind::Function(a)) if p.params.len() == a.params.len() => {
                let params: Vec<(TyId, TyId)> = p.params.iter().zip(&a.params).map(|(p, a)| (p.ty, a.ty)).collect();
                let mut ok = true;
                for (p, a) in params {
                    ok &= self.unify_in(f, p, a, false);
                }
                ok & self.unify_result(f, p.ret, a.ret)
            }
            (TyKind::Union(members), _) => self.unify_union(f, &members, arg, strict_unit),
            _ => false,
        }
    }

    /// Like `unify_in` for the results of two functions, where `Null` is a result and fixes a variable.
    fn unify_result(&mut self, f: &mut FnCx, param: TyId, arg: TyId) -> bool {
        let (param, arg) = (self.solved(f, param), self.solved(f, arg));
        match (self.program.types.kind(param).clone(), arg) {
            (TyKind::Infer(variable), TyId::UNIT) => self.assign_variable(f, variable, arg),
            _ => self.unify_in(f, param, arg, false),
        }
    }

    fn unify_all(&mut self, f: &mut FnCx, params: &[TyId], args: &[TyId]) -> bool {
        let mut ok = true;
        for (&param, &arg) in params.iter().zip(args) {
            ok &= self.unify_in(f, param, arg, true);
        }
        ok
    }

    /// A union holding one member with variables takes what the argument has beyond the other members.
    fn unify_union(&mut self, f: &mut FnCx, members: &[TyId], arg: TyId, strict_unit: bool) -> bool {
        let (open, plain): (Vec<TyId>, Vec<TyId>) = members.iter().copied().partition(|&member| self.program.types.has_infer(member));
        let &[open] = open.as_slice() else { return true };
        let held = match self.program.types.kind(arg) {
            TyKind::Union(held) => held.clone(),
            _ => vec![arg],
        };
        let rest: Vec<TyId> = held.into_iter().filter(|member| !plain.contains(member)).collect();
        if rest.is_empty() {
            return true;
        }
        let rest = self.program.types.union(&rest);
        self.unify_in(f, open, rest, strict_unit)
    }

    fn assign_variable(&mut self, f: &mut FnCx, variable: u32, ty: TyId) -> bool {
        if self.program.types.mentions(ty, variable) {
            return false;
        }
        f.infer[variable as usize] = Some(ty);
        true
    }

    /// Reports a type argument that does not meet the bounds of its parameter.
    pub(crate) fn check_bounds(&mut self, f: &FnCx, ty: TyId, bounds: &[Bound], args: &[GenericArg], span: Span) {
        for bound in bounds {
            let bound = &Bound { def: bound.def, args: bound.args.iter().map(|&arg| self.program.types.subst_generic(arg, args)).collect() };
            if let Some(reason) = self.unmet(f, ty, bound) {
                let message = format!("{} does not satisfy `{}`: {reason}", self.show_ty(ty), self.show_bound(bound));
                self.error(span, message);
            }
        }
    }

    /// A trait as written: its name and type arguments.
    pub(crate) fn show_bound(&self, bound: &Bound) -> String {
        let name = self.text(self.program.defs[bound.def].name);
        if bound.args.is_empty() {
            return name;
        }
        let args: Vec<String> = bound.args.iter().map(|&arg| self.show_ty(arg)).collect();
        format!("{name}<{}>", args.join(", "))
    }

    /// Why the type misses the trait, or `None` when it has every method the trait requires.
    pub(super) fn unmet(&mut self, f: &FnCx, ty: TyId, bound: &Bound) -> Option<String> {
        match self.program.types.kind(ty) {
            TyKind::Error => return None,
            TyKind::Param { index, .. } => {
                let bounded = f.bounds.get(*index as usize).is_some_and(|bounds| bounds.contains(bound));
                return (!bounded).then(|| "it has no such bound".to_string());
            }
            _ => {}
        }
        if self.program.std.key == Some(bound.def) {
            let offender = self.unkeyable(f, ty, &mut Vec::new())?;
            return Some(format!("it contains {}", self.show_ty(offender)));
        }
        let DefKind::Trait(item) = &self.program.defs[bound.def].kind else { return None };
        let required: Vec<DefId> = item.methods.iter().copied().filter(|&method| self.program.defs[method].function().is_some_and(|function| !function.has_body)).collect();
        for method in required {
            let name = self.program.defs[method].name;
            let text = self.text(name);
            let candidates: Vec<_> = self
                .methods_for_type(ty, name)
                .into_iter()
                .filter(|(cand, _)| self.visible(f, *cand, self.program.defs[*cand].is_pub))
                .collect();
            let has_visible = !candidates.is_empty();
            let matched = candidates
                .into_iter()
                .find(|(cand, owner_args)| self.same_signature(method, ty, &bound.args, *cand, owner_args));
            if matched.is_none() {
                return Some(if has_visible { format!("`{text}` does not match the trait's") } else { format!("it has no method `{text}`") });
            }
        }
        None
    }

    /// Whether `found`, a method of the type, has the parameters and result of the trait's `required` with `Self` as the type and the trait's arguments.
    pub(crate) fn same_signature(&mut self, required: DefId, self_ty: TyId, trait_args: &[TyId], found: DefId, found_args: &[GenericArg]) -> bool {
        let (want, have) = (self.program.defs[required].function().expect("a method"), self.program.defs[found].function().expect("a method"));
        if want.receiver != have.receiver || want.params.len() != have.params.len() || want.generics.len() != have.generics.len() {
            return false;
        }
        let (want_params, want_ret, have_params, have_ret) = (want.params.clone(), want.ret, have.params.clone(), have.ret);
        let types = &mut self.program.types;
        let owner_args: Vec<GenericArg> = std::iter::once(self_ty).chain(trait_args.iter().copied()).map(GenericArg::Type).collect();
        let ret = (types.subst_generic(want_ret, &owner_args), types.subst_generic(have_ret, found_args));
        ret.0 == ret.1
            && want_params
                .iter()
                .zip(&have_params)
                .all(|(want, have)| want.is_var == have.is_var && types.subst_generic(want.ty, &owner_args) == types.subst_generic(have.ty, found_args))
    }
}
