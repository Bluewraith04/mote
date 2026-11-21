use mote_base::{Span, Symbol};

use super::FnCx;
use crate::cx::Cx;
use crate::defs::{Bound, DefId, DefKind, LangItem};
use crate::types::{TyId, TyKind};

impl Cx<'_> {
    /// A new variable for a type argument a call has to find.
    pub(crate) fn fresh(&mut self, f: &mut FnCx) -> TyId {
        f.infer.push(None);
        self.program.types.intern(TyKind::Infer((f.infer.len() - 1) as u32))
    }

    /// The type with every variable already found put in.
    pub(crate) fn solved(&mut self, f: &FnCx, ty: TyId) -> TyId {
        self.program.types.solve(ty, &f.infer)
    }

    pub(crate) fn list_of(&mut self, element: TyId, span: Span) -> TyId {
        self.lang_type(LangItem::List, vec![element], span)
    }

    /// How many type parameters the owner of a method contributes; a trait counts `Self`.
    pub(crate) fn owner_generics(&self, def: DefId) -> usize {
        let Some(owner) = self.program.defs[def].owner else { return 0 };
        match &self.program.defs[owner].kind {
            DefKind::Trait(item) => 1 + item.generics.len(),
            DefKind::Record(record) => record.generics.len(),
            DefKind::Enum(en) => en.generics.len(),
            _ => 0,
        }
    }

    /// The names of a function's type parameters in numbering order.
    pub(crate) fn generic_names(&self, def: DefId) -> Vec<Symbol> {
        let mut names = Vec::new();
        if let Some(owner) = self.program.defs[def].owner {
            match &self.program.defs[owner].kind {
                DefKind::Trait(item) => {
                    names.push(self.self_name);
                    names.extend(item.generics.iter().map(|param| param.name));
                }
                DefKind::Record(record) => names.extend(record.generics.iter().map(|param| param.name)),
                DefKind::Enum(en) => names.extend(en.generics.iter().map(|param| param.name)),
                _ => {}
            }
        }
        if let Some(function) = self.program.defs[def].function() {
            names.extend(function.generics.iter().map(|param| param.name));
        }
        names
    }

    /// The traits bounding each type parameter a function's body sees, by position.
    pub(crate) fn param_bounds(&mut self, def: DefId) -> Vec<Vec<Bound>> {
        let mut bounds = Vec::new();
        if let Some(owner) = self.program.defs[def].owner {
            match &self.program.defs[owner].kind {
                DefKind::Trait(item) => {
                    let names: Vec<Symbol> = item.generics.iter().map(|param| param.name).collect();
                    let args = names.iter().enumerate().map(|(index, &name)| self.program.types.param(index as u32 + 1, name)).collect();
                    let DefKind::Trait(item) = &self.program.defs[owner].kind else { unreachable!("a trait") };
                    bounds.push(vec![Bound { def: owner, args }]);
                    bounds.extend(item.generics.iter().map(|param| param.bounds.clone()));
                }
                DefKind::Record(record) => bounds.extend(record.generics.iter().map(|param| param.bounds.clone())),
                DefKind::Enum(en) => bounds.extend(en.generics.iter().map(|param| param.bounds.clone())),
                _ => {}
            }
        }
        if let Some(function) = self.program.defs[def].function() {
            bounds.extend(function.generics.iter().map(|param| param.bounds.clone()));
        }
        bounds
    }

    /// A function's own type arguments: the ones written, or a variable for each.
    pub(crate) fn own_type_args(&mut self, f: &mut FnCx, def: DefId, written: &[TyId], span: Span) -> Option<Vec<TyId>> {
        let count = self.program.defs[def].function().map_or(0, |function| function.generics.len());
        if written.is_empty() {
            return Some((0..count).map(|_| self.fresh(f)).collect());
        }
        if written.len() != count {
            let name = self.text(self.program.defs[def].name);
            let message = match count {
                0 => format!("`{name}` takes no type arguments"),
                1 => format!("`{name}` takes 1 type argument"),
                _ => format!("`{name}` takes {count} type arguments"),
            };
            self.error(span, message);
            return None;
        }
        Some(written.to_vec())
    }

    /// Finds the variables that make `param` and `arg` one type; `false` when no choice does, with nothing changed.
    pub(crate) fn unify(&mut self, f: &mut FnCx, param: TyId, arg: TyId) -> bool {
        let saved = f.infer.clone();
        if self.unify_in(f, param, arg) {
            return true;
        }
        f.infer = saved;
        false
    }

    fn unify_in(&mut self, f: &mut FnCx, param: TyId, arg: TyId) -> bool {
        let param = self.solved(f, param);
        let arg = self.solved(f, arg);
        if param == arg {
            return true;
        }
        let (p, a) = (self.program.types.kind(param).clone(), self.program.types.kind(arg).clone());
        match (p, a) {
            (TyKind::Error, _) | (_, TyKind::Error) | (TyKind::Infer(_), TyKind::Unit) => true,
            (TyKind::Infer(variable), _) => self.assign_variable(f, variable, arg),
            (_, TyKind::Infer(variable)) => self.assign_variable(f, variable, param),
            (TyKind::Optional(p), TyKind::Optional(a)) => self.unify_in(f, p, a),
            (TyKind::Optional(p), _) => arg == TyId::UNIT || self.unify_in(f, p, arg),
            (TyKind::Adt { def: pd, args: pa }, TyKind::Adt { def: ad, args: aa }) if pd == ad && pa.len() == aa.len() => self.unify_all(f, &pa, &aa),
            (TyKind::Tuple(p), TyKind::Tuple(a)) if p.len() == a.len() => self.unify_all(f, &p, &a),
            (TyKind::Array(p), TyKind::Array(a)) => self.unify_in(f, p, a),
            (TyKind::FixedArray(p, n), TyKind::FixedArray(a, m)) if n == m => self.unify_in(f, p, a),
            (TyKind::Function(p), TyKind::Function(a)) if p.params.len() == a.params.len() => {
                let params: Vec<(TyId, TyId)> = p.params.iter().zip(&a.params).map(|(p, a)| (p.ty, a.ty)).collect();
                let mut ok = true;
                for (p, a) in params {
                    ok &= self.unify_in(f, p, a);
                }
                ok & self.unify_result(f, p.ret, a.ret)
            }
            (TyKind::Union(members), _) => self.unify_union(f, &members, arg),
            _ => false,
        }
    }

    /// Like `unify_in` for the results of two functions, where `Null` is a result and fixes a variable.
    fn unify_result(&mut self, f: &mut FnCx, param: TyId, arg: TyId) -> bool {
        let (param, arg) = (self.solved(f, param), self.solved(f, arg));
        match (self.program.types.kind(param).clone(), arg) {
            (TyKind::Infer(variable), TyId::UNIT) => self.assign_variable(f, variable, arg),
            _ => self.unify_in(f, param, arg),
        }
    }

    fn unify_all(&mut self, f: &mut FnCx, params: &[TyId], args: &[TyId]) -> bool {
        let mut ok = true;
        for (&param, &arg) in params.iter().zip(args) {
            ok &= self.unify_in(f, param, arg);
        }
        ok
    }

    /// A union holding one member with variables takes what the argument has beyond the other members.
    fn unify_union(&mut self, f: &mut FnCx, members: &[TyId], arg: TyId) -> bool {
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
        self.unify_in(f, open, rest)
    }

    fn assign_variable(&mut self, f: &mut FnCx, variable: u32, ty: TyId) -> bool {
        if self.program.types.mentions(ty, variable) {
            return false;
        }
        f.infer[variable as usize] = Some(ty);
        true
    }

    /// Reports a type argument that does not meet the bounds of its parameter.
    pub(crate) fn check_bounds(&mut self, f: &FnCx, ty: TyId, bounds: &[Bound], args: &[TyId], span: Span) {
        for bound in bounds {
            let bound = &Bound { def: bound.def, args: bound.args.iter().map(|&arg| self.program.types.subst(arg, args)).collect() };
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
    fn unmet(&mut self, f: &FnCx, ty: TyId, bound: &Bound) -> Option<String> {
        match self.program.types.kind(ty) {
            TyKind::Error => return None,
            TyKind::Param { index, .. } => {
                let bounded = f.bounds.get(*index as usize).is_some_and(|bounds| bounds.contains(bound));
                return (!bounded).then(|| "it has no such bound".to_string());
            }
            _ => {}
        }
        if self.program.defs[bound.def].lang == Some(LangItem::Key) {
            let offender = self.unkeyable(f, ty, &mut Vec::new())?;
            return Some(format!("it contains {}", self.show_ty(offender)));
        }
        let DefKind::Trait(item) = &self.program.defs[bound.def].kind else { return None };
        let required: Vec<DefId> = item.methods.iter().copied().filter(|&method| self.program.defs[method].function().is_some_and(|function| !function.has_body)).collect();
        for method in required {
            let name = self.program.defs[method].name;
            let text = self.text(name);
            let Some((owner, owner_args)) = self.method_owner(ty) else { return Some(format!("it has no method `{text}`")) };
            let found = self.methods_named(owner, name).into_iter().find(|&candidate| self.visible(f, candidate, self.program.defs[candidate].is_pub));
            let Some(found) = found else { return Some(format!("it has no method `{text}`")) };
            if !self.same_signature(method, ty, &bound.args, found, &owner_args) {
                return Some(format!("`{text}` does not match the trait's"));
            }
        }
        None
    }

    /// Whether `found`, a method of the type, has the parameters and result of the trait's `required` with `Self` as the type and the trait's arguments.
    pub(crate) fn same_signature(&mut self, required: DefId, self_ty: TyId, trait_args: &[TyId], found: DefId, found_args: &[TyId]) -> bool {
        let (want, have) = (self.program.defs[required].function().expect("a method"), self.program.defs[found].function().expect("a method"));
        if want.receiver != have.receiver || want.params.len() != have.params.len() || want.generics.len() != have.generics.len() {
            return false;
        }
        let (want_params, want_ret, have_params, have_ret, generic) = (want.params.clone(), want.ret, have.params.clone(), have.ret, !want.generics.is_empty());
        if generic {
            return true;
        }
        let types = &mut self.program.types;
        let owner_args: Vec<TyId> = std::iter::once(self_ty).chain(trait_args.iter().copied()).collect();
        let ret = (types.subst(want_ret, &owner_args), types.subst(have_ret, found_args));
        ret.0 == ret.1
            && want_params.iter().zip(&have_params).all(|(want, have)| want.is_var == have.is_var && types.subst(want.ty, &owner_args) == types.subst(have.ty, found_args))
    }
}
