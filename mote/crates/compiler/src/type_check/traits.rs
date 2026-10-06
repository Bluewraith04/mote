//! Trait declarations and the structural satisfaction test.

use super::*;

impl TypeChecker {
    pub(super) fn register_trait(&mut self, t: &TraitDecl) {
        if !t.generic_params.is_empty() {
            self.errors.push((format!("trait `{}` has type parameters, which are not supported yet", t.name), t.span));
        }
        self.traits.insert(t.name.clone(), t.clone());
    }

    pub(super) fn check_declared_traits(&mut self, type_name: &str, listed: &[TypeNode]) {
        for node in listed {
            let span = node.span();
            let Some(trait_name) = crate::impls::trait_ref_name(node) else {
                self.errors.push(("a listed trait must be a trait name".into(), span));
                continue;
            };
            let Some(t) = self.traits.get(trait_name).cloned() else {
                self.errors.push((format!("`{trait_name}` is not a trait"), span));
                continue;
            };
            if let Some(problem) = self.trait_mismatch(&t, type_name) {
                self.errors.push((format!("`{type_name}` does not implement `{}`: {problem}", t.name), span));
            }
        }
    }

    pub(super) fn trait_mismatch(&mut self, t: &TraitDecl, type_name: &str) -> Option<String> {
        let self_ty = self.types.get(type_name).cloned()?;
        let prev = self.current_self_type.replace(self_ty);
        let problem = t.members.iter().find_map(|m| self.member_mismatch(&t.name, type_name, m));
        self.current_self_type = prev;
        problem
    }

    fn member_mismatch(&mut self, trait_name: &str, type_name: &str, m: &TraitMember) -> Option<String> {
        let takes_self = matches!(m.params.first().and_then(|p| p.kind.as_ref()), Some(ParameterKind::SelfValue { .. }));
        let Some(sig) = self.class_method_sigs.get(&(type_name.to_string(), m.name.clone())).cloned() else {
            return Some(format!("missing method `{}`", m.name));
        };
        if sig.is_static == takes_self {
            let want = if takes_self { "take" } else { "not take" };
            return Some(format!("`{}` must {want} a `self` receiver, as in `{trait_name}`", m.name));
        }
        let mark = self.push_type_params(&m.generic_params);
        let params: Vec<Type> = m
            .params
            .iter()
            .filter(|p| !matches!(p.kind, Some(ParameterKind::SelfValue { .. })))
            .map(|p| p.ty.as_ref().map(|t| self.resolve_type_node(t)).unwrap_or(Type::Any))
            .collect();
        let ret = m.return_type.as_ref().map(|t| self.resolve_type_node(t)).unwrap_or(Type::Null);
        self.pop_type_params(mark);
        if params.len() != sig.params.len() {
            return Some(format!("`{}` takes {} argument(s), but `{trait_name}` wants {}", m.name, sig.params.len(), params.len()));
        }
        if !m.generic_params.is_empty() {
            return None;
        }
        let same = |a: &Type, b: &Type| a.is_assignable_to(b) && b.is_assignable_to(a);
        if let Some((have, want)) = sig.params.iter().zip(&params).find(|(a, b)| !same(a, b)) {
            return Some(format!("`{}` takes '{have:?}' where `{trait_name}` wants '{want:?}'", m.name));
        }
        if !same(&sig.ret, &ret) {
            return Some(format!("`{}` returns '{:?}' where `{trait_name}` wants '{ret:?}'", m.name, sig.ret));
        }
        None
    }
}

impl TypeChecker {
    pub(super) fn bound_method_call(&mut self, p: &str, method: &str, args: &[Type], span: Span) -> Option<Type> {
        let traits = self.param_bounds.get(p)?.clone();
        let (t, m) = traits.iter().filter_map(|n| self.traits.get(n)).find_map(|t| {
            let m = t.members.iter().find(|m| m.name == method && matches!(m.params.first().and_then(|q| q.kind.as_ref()), Some(ParameterKind::SelfValue { .. })))?;
            Some((t.clone(), m.clone()))
        })?;
        let prev = self.current_self_type.replace(Type::Param(p.to_string()));
        let mark = self.push_type_params(&m.generic_params);
        let params: Vec<Type> = m
            .params
            .iter()
            .filter(|q| !matches!(q.kind, Some(ParameterKind::SelfValue { .. })))
            .map(|q| q.ty.as_ref().map(|n| self.resolve_type_node(n)).unwrap_or(Type::Any))
            .collect();
        let ret = m.return_type.as_ref().map(|n| self.resolve_type_node(n)).unwrap_or(Type::Null);
        self.pop_type_params(mark);
        self.current_self_type = prev;
        if params.len() != args.len() {
            self.errors.push((format!("`.{method}()` from `{}` takes {} argument(s), but {} were given", t.name, params.len(), args.len()), span));
        } else {
            for (i, (got, want)) in args.iter().zip(&params).enumerate() {
                if !got.is_assignable_to(want) {
                    self.errors.push((format!("`.{method}()` argument {} expects '{want:?}', found '{got:?}'", i + 1), span));
                }
                self.note_arg_check(i, got, want);
            }
        }
        Some(ret)
    }
}
