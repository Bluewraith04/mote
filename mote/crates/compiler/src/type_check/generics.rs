//! Type parameters: scope, bound rejection, inference and substitution at a call.

use super::*;

pub(super) struct CallTypes {
    pub names: Vec<String>,
    pub all: Vec<String>,
    pub ret: Type,
    pub bind: HashMap<String, Type>,
    pub variadic: Option<Type>,
}

impl TypeChecker {
    pub(super) fn push_type_params(&mut self, params: &[GenericParam]) -> usize {
        let mark = self.type_params.len();
        self.type_params.extend(params.iter().map(|p| p.name.clone()));
        for p in params {
            let traits: Vec<String> = p.bounds.iter().filter_map(|b| crate::impls::trait_ref_name(b).map(str::to_string)).collect();
            if !traits.is_empty() {
                self.param_bounds.insert(p.name.clone(), traits);
            }
        }
        mark
    }

    pub(super) fn pop_type_params(&mut self, mark: usize) {
        for name in self.type_params.drain(mark..) {
            self.param_bounds.remove(&name);
        }
    }

    pub(super) fn reject_bounds(&mut self, params: &[GenericParam]) {
        for p in params.iter().filter(|p| !p.bounds.is_empty()) {
            self.errors.push((format!("`{}` has a trait bound on a type's parameter, which is not supported yet", p.name), p.span));
        }
    }

    pub(super) fn collect_bounds(&mut self, params: &[GenericParam]) -> Vec<(String, Vec<String>)> {
        let mut out = Vec::new();
        for p in params.iter().filter(|p| !p.bounds.is_empty()) {
            let mut traits = Vec::new();
            for b in &p.bounds {
                match crate::impls::trait_ref_name(b) {
                    Some(n) if self.traits.contains_key(n) => traits.push(n.to_string()),
                    Some(n) => self.errors.push((format!("`{n}` is not a trait, so it cannot bound `{}`", p.name), p.span)),
                    None => self.errors.push((format!("a bound on `{}` must be a trait name", p.name), p.span)),
                }
            }
            out.push((p.name.clone(), traits));
        }
        out
    }

    pub(super) fn check_bounds(&mut self, bounds: &[(String, Vec<String>)], bind: &HashMap<String, Type>, span: Span) {
        for (param, traits) in bounds {
            let Some(ty) = bind.get(param) else { continue };
            for t in traits {
                if let Some(why) = self.bound_mismatch(ty, t) {
                    self.errors.push((format!("`{}` cannot be used for `{param}`: {why}", Self::describe(ty)), span));
                }
            }
        }
    }

    fn bound_mismatch(&mut self, ty: &Type, trait_name: &str) -> Option<String> {
        match ty {
            Type::Any => None,
            Type::Param(q) if self.param_bounds.get(q).is_some_and(|b| b.iter().any(|x| x == trait_name)) => None,
            Type::Class { name, .. } | Type::Struct { name, .. } | Type::Enum { name, .. } => {
                let decl = self.traits.get(trait_name)?.clone();
                self.trait_mismatch(&decl, name)
            }
            Type::Int | Type::Float | Type::Bool | Type::Char | Type::String => {
                let decl = self.traits.get(trait_name)?.clone();
                decl.members
                    .iter()
                    .find(|m| !Self::primitive_provides(ty, m))
                    .map(|m| format!("missing method `{}`", m.name))
            }
            _ => Some(format!("it does not implement `{trait_name}`")),
        }
    }

    fn primitive_provides(ty: &Type, m: &TraitMember) -> bool {
        let takes_self = matches!(m.params.first().and_then(|p| p.kind.as_ref()), Some(ParameterKind::SelfValue { .. }));
        if !takes_self {
            return false;
        }
        match (m.name.as_str(), m.params.len() - 1) {
            ("to_string" | "debug", 0) => true,
            ("eq", 1) => true,
            ("compare", 1) => !matches!(ty, Type::Bool),
            _ => false,
        }
    }

    pub(super) fn describe(ty: &Type) -> String {
        match ty {
            Type::Int => "Int".into(),
            Type::Float => "Float".into(),
            Type::Bool => "Bool".into(),
            Type::Char => "Char".into(),
            Type::String => "String".into(),
            Type::Param(n) | Type::Class { name: n, .. } | Type::Struct { name: n, .. } | Type::Enum { name: n, .. } => n.clone(),
            Type::List(e) => format!("List<{}>", Self::describe(e)),
            other => format!("{other:?}"),
        }
    }

    pub(super) fn instantiate(names: &[String], preset: &[(String, Type)], params: &[Type], ret: &Type, args: &[Type]) -> (Vec<Type>, Type, HashMap<String, Type>) {
        if names.is_empty() {
            return (params.to_vec(), ret.clone(), HashMap::new());
        }
        let mut bind: HashMap<String, Type> = preset.iter().cloned().collect();
        for (p, a) in params.iter().zip(args) {
            Self::unify(p, a, names, &mut bind);
        }
        (params.iter().map(|p| Self::subst(p, names, &bind)).collect(), Self::subst(ret, names, &bind), bind)
    }

    pub(super) fn unify(param: &Type, arg: &Type, names: &[String], bind: &mut HashMap<String, Type>) {
        match (param, arg) {
            (Type::Class { name: a, args: pa, .. }, Type::Class { name: b, args: ba, .. })
            | (Type::Struct { name: a, args: pa, .. }, Type::Struct { name: b, args: ba, .. })
            | (Type::Enum { name: a, args: pa, .. }, Type::Enum { name: b, args: ba, .. })
                if a == b =>
            {
                for (p, x) in pa.iter().zip(ba) {
                    Self::unify(p, x, names, bind);
                }
            }
            (Type::Param(n), _) if names.contains(n) && *arg != Type::Hole => {
                bind.entry(n.clone()).or_insert_with(|| arg.clone());
            }
            (_, Type::Any) => Self::bind_any(param, names, bind),
            (Type::Union(crate::types::Members(ps)), _) => {
                let (open, fixed): (Vec<&Type>, Vec<&Type>) = ps.iter().partition(|p| Self::has_param(p));
                let left: Vec<Type> = arg.members().into_iter().filter(|a| !fixed.iter().any(|f| a.is_assignable_to(f))).collect();
                if let ([p], false) = (open.as_slice(), left.is_empty()) {
                    Self::unify(p, &Type::union(left), names, bind);
                }
            }
            (Type::List(a), Type::List(b))
            | (Type::Set(a), Type::Set(b))
            | (Type::Receiver(a), Type::Receiver(b))
            | (Type::Sender(a), Type::Sender(b))
            | (Type::Stream(a), Type::Stream(b))
            | (Type::Task(a), Type::Task(b))
            | (Type::Shared(a), Type::Shared(b))
            | (Type::Nullable(a), Type::Nullable(b)) => Self::unify(a, b, names, bind),
            (Type::Nullable(_), Type::Null) => {}
            (Type::Nullable(a), other) => Self::unify(a, other, names, bind),
            (Type::Map(ak, av), Type::Map(bk, bv)) | (Type::Result(ak, av), Type::Result(bk, bv)) => {
                Self::unify(ak, bk, names, bind);
                Self::unify(av, bv, names, bind);
            }
            (Type::Tuple(pa), Type::Tuple(ba)) => {
                for (p, x) in pa.iter().zip(ba) {
                    Self::unify(p, x, names, bind);
                }
            }
            (Type::Function { params: pp, ret: pr, .. }, Type::Function { params: ap, ret: ar, .. }) => {
                for (p, x) in pp.iter().zip(ap) {
                    Self::unify(p, x, names, bind);
                }
                Self::unify(pr, ar, names, bind);
            }
            _ => {}
        }
    }

    fn bind_any(param: &Type, names: &[String], bind: &mut HashMap<String, Type>) {
        match param {
            Type::Param(n) if names.contains(n) => {
                bind.entry(n.clone()).or_insert(Type::Any);
            }
            Type::List(a) | Type::Set(a) | Type::Receiver(a) | Type::Sender(a) | Type::Stream(a) | Type::Task(a) | Type::Shared(a) | Type::Nullable(a) => {
                Self::bind_any(a, names, bind)
            }
            Type::Map(k, v) | Type::Result(k, v) => {
                Self::bind_any(k, names, bind);
                Self::bind_any(v, names, bind);
            }
            Type::Class { args, .. } | Type::Struct { args, .. } | Type::Enum { args, .. } | Type::Tuple(args) | Type::Union(crate::types::Members(args)) => {
                for a in args {
                    Self::bind_any(a, names, bind);
                }
            }
            Type::Function { params, ret, .. } => {
                for a in params {
                    Self::bind_any(a, names, bind);
                }
                Self::bind_any(ret, names, bind);
            }
            _ => {}
        }
    }

    fn subst_all(tys: &[Type], names: &[String], bind: &HashMap<String, Type>) -> Vec<Type> {
        tys.iter().map(|t| Self::subst(t, names, bind)).collect()
    }

    fn subst_fields(fields: &[(String, Type)], names: &[String], bind: &HashMap<String, Type>) -> Vec<(String, Type)> {
        fields.iter().map(|(n, t)| (n.clone(), Self::subst(t, names, bind))).collect()
    }

    pub(super) fn subst(ty: &Type, names: &[String], bind: &HashMap<String, Type>) -> Type {
        let go = |t: &Type| Box::new(Self::subst(t, names, bind));
        match ty {
            Type::Param(n) if names.contains(n) => bind.get(n).cloned().unwrap_or(Type::Hole),
            Type::List(a) => Type::List(go(a)),
            Type::Set(a) => Type::Set(go(a)),
            Type::Receiver(a) => Type::Receiver(go(a)),
            Type::Sender(a) => Type::Sender(go(a)),
            Type::Stream(a) => Type::Stream(go(a)),
            Type::Task(a) => Type::Task(go(a)),
            Type::Shared(a) => Type::Shared(go(a)),
            Type::Nullable(a) => Type::Nullable(go(a)),
            Type::Map(k, v) => Type::Map(go(k), go(v)),
            Type::Class { name, args, fields } => Type::Class { name: name.clone(), args: Self::subst_all(args, names, bind), fields: Self::subst_fields(fields, names, bind) },
            Type::Struct { name, args, fields } => Type::Struct { name: name.clone(), args: Self::subst_all(args, names, bind), fields: Self::subst_fields(fields, names, bind) },
            Type::Enum { name, args, variants } => Type::Enum {
                name: name.clone(),
                args: Self::subst_all(args, names, bind),
                variants: variants.iter().map(|(v, p)| (v.clone(), Self::subst_all(p, names, bind))).collect(),
            },
            Type::Tuple(elems) => Type::Tuple(Self::subst_all(elems, names, bind)),
            Type::Union(crate::types::Members(ms)) => Type::union(Self::subst_all(ms, names, bind)),
            Type::Function { params, ret, sendable, modes } => {
                Type::Function { params: Self::subst_all(params, names, bind), ret: go(ret), sendable: *sendable, modes: modes.clone() }
            }
            other => other.clone(),
        }
    }
}

impl TypeChecker {
    pub(super) fn declare_generics(&mut self, name: &str, params: &[GenericParam]) -> Vec<Type> {
        if !params.is_empty() {
            self.generics.insert(name.to_string(), params.iter().map(|p| p.name.clone()).collect());
        }
        params.iter().map(|p| Type::Param(p.name.clone())).collect()
    }

    pub(super) fn instantiate_type(&self, name: &str, args: &[Type]) -> Type {
        let (Some(template), Some(names)) = (self.types.get(name), self.generics.get(name)) else {
            return self.types.get(name).cloned().unwrap_or(Type::Any);
        };
        let bind = names.iter().enumerate().map(|(i, n)| (n.clone(), args.get(i).cloned().unwrap_or(Type::Hole))).collect();
        Self::subst(template, names, &bind)
    }

    pub(super) fn instantiate_by_shape(&self, wanted: &[(&str, usize)], args: &[Type]) -> Type {
        let found = self.types.iter().find_map(|(key, ty)| {
            let Type::Enum { variants, .. } = ty else { return None };
            let shape_matches = variants.len() == wanted.len()
                && wanted.iter().all(|(name, arity)| variants.iter().any(|(v, p)| v == name && p.len() == *arity));
            shape_matches.then_some((ty, key.as_str()))
        });
        let Some((template, key)) = found else { return Type::Any };
        let Some(names) = self.generics.get(key) else { return Type::Any };
        let bind = names.iter().enumerate().map(|(i, n)| (n.clone(), args.get(i).cloned().unwrap_or(Type::Hole))).collect();
        Self::subst(template, names, &bind)
    }

    pub(super) fn flush_type_errors(&mut self) {
        for e in self.type_errors.borrow_mut().drain(..) {
            if !self.errors.contains(&e) {
                self.errors.push(e);
            }
        }
    }

    pub(super) fn push_type_param_names(&mut self, names: &[String]) -> usize {
        let mark = self.type_params.len();
        self.type_params.extend(names.iter().cloned());
        mark
    }
}

impl TypeChecker {
    fn has_param(ty: &Type) -> bool {
        match ty {
            Type::Param(_) => true,
            Type::List(a) | Type::Set(a) | Type::Receiver(a) | Type::Sender(a) | Type::Stream(a) | Type::Task(a) | Type::Shared(a) | Type::Nullable(a) => Self::has_param(a),
            Type::Map(k, v) => Self::has_param(k) || Self::has_param(v),
            Type::Class { args, .. } | Type::Struct { args, .. } | Type::Enum { args, .. } | Type::Tuple(args) | Type::Union(crate::types::Members(args)) => args.iter().any(Self::has_param),
            Type::Function { params, ret, .. } => params.iter().any(Self::has_param) || Self::has_param(ret),
            _ => false,
        }
    }

    fn type_key(ty: &Type) -> String {
        match ty {
            Type::Class { name, args, .. } | Type::Struct { name, args, .. } | Type::Enum { name, args, .. } if !args.is_empty() => {
                format!("{name}<{}>", args.iter().map(Self::type_key).collect::<Vec<_>>().join(","))
            }
            Type::List(e) => format!("List<{}>", Self::type_key(e)),
            Type::Set(e) => format!("Set<{}>", Self::type_key(e)),
            Type::Map(k, v) => format!("Map<{},{}>", Self::type_key(k), Self::type_key(v)),
            Type::Nullable(e) => format!("{}?", Self::type_key(e)),
            Type::Any => "Any".into(),
            other => Self::describe(other),
        }
    }

    pub(super) fn request_instance(&mut self, name: &str, type_params: &[String], bind: &HashMap<String, Type>, span: Span) {
        let Some(template) = self.bounded_fns.get(name).cloned() else { return };
        let binding: Vec<(String, Type)> = type_params.iter().map(|p| (p.clone(), bind.get(p).cloned().unwrap_or(Type::Any))).collect();
        if binding.iter().any(|(_, t)| Self::has_param(t)) {
            return;
        }
        let key = binding.iter().map(|(_, t)| Self::type_key(t)).collect::<Vec<_>>().join(",");
        let instance = format!("{name}<{key}>");
        self.instance_calls.insert(span, instance.clone());
        if self.instance_seen.insert(instance.clone()) {
            self.pending_instances.push((instance, template, binding));
        }
    }

    pub(super) fn note_call(&mut self, span: Span, call: CallTypes) {
        if !call.names.is_empty() || call.variadic.is_some() {
            self.calls.insert(span, call);
        }
    }

    pub(super) fn resolve_calls(&mut self) -> (HashMap<Span, Vec<Type>>, HashMap<Span, Type>) {
        let mut args = HashMap::new();
        let mut lists = HashMap::new();
        for (span, call) in std::mem::take(&mut self.calls) {
            let mut bind = call.bind;
            if let Some(settled) = self.expr_types.get(&span).filter(|t| **t != Type::Any) {
                let mut late = HashMap::new();
                Self::unify(&call.ret, settled, &call.all, &mut late);
                for (name, ty) in late {
                    if bind.get(&name).is_none_or(Type::has_hole) {
                        bind.insert(name, ty);
                    }
                }
            }
            if !call.names.is_empty() {
                args.insert(span, call.names.iter().map(|n| bind.get(n).cloned().unwrap_or(Type::Hole)).collect());
            }
            if let Some(elem) = &call.variadic {
                lists.insert(span, Type::List(Box::new(Self::subst(elem, &call.all, &bind))));
            }
        }
        (args, lists)
    }

    pub(super) fn check_pending_instances(&mut self) {
        while let Some((name, template, binding)) = self.pending_instances.pop() {
            if self.instance_seen.len() > 512 {
                self.errors.push((format!("`{}` is instantiated too many times", template.name), template.span));
                return;
            }
            let outer_types = std::mem::take(&mut self.expr_types);
            let outer_calls = std::mem::take(&mut self.instance_calls);
            let outer_generic_calls = std::mem::take(&mut self.calls);
            let outer_checks = std::mem::take(&mut self.any_checks);
            let outer_tests = std::mem::take(&mut self.type_tests);
            let outer_lifts = std::mem::take(&mut self.some_lifts);
            let outer_reads = std::mem::take(&mut self.narrowed_reads);
            let outer_narrowings = std::mem::take(&mut self.narrowings);
            let outer_subst = std::mem::replace(&mut self.param_subst, binding.into_iter().collect());
            let mut decl = template;
            decl.name = name;
            decl.generic_params.clear();
            self.check_function(&decl);
            self.param_subst = outer_subst;
            let (type_args, lists) = self.resolve_calls();
            self.calls = outer_generic_calls;
            let table = crate::stage::TypeTable::with_instance_calls(
                std::mem::replace(&mut self.expr_types, outer_types),
                std::mem::replace(&mut self.instance_calls, outer_calls),
            )
            .with_calls(type_args, lists)
            .with_any_checks(std::mem::replace(&mut self.any_checks, outer_checks))
            .with_type_tests(std::mem::replace(&mut self.type_tests, outer_tests))
            .with_rewraps(self.take_rewraps());
            self.some_lifts = outer_lifts;
            self.narrowed_reads = outer_reads;
            self.narrowings = outer_narrowings;
            self.instance_done.push((decl, table));
        }
    }
}
