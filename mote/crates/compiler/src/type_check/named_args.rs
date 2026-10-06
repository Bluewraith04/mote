//! Named arguments: a call written `f(a, y = 2)` is reordered to positional form, calling the hidden default function for each skipped default.

use super::*;

/// Hidden calls the checker adds get spans from here up, so they never match a written node.
const HIDDEN_BASE: usize = 1 << 50;

/// What a callee declares about its parameters.
struct Params {
    /// The callee's name as the code generator knows it.
    callee: String,
    names: Vec<String>,
    required: usize,
    variadic: bool,
    generic: bool,
}

impl TypeChecker {
    fn callee_params(&mut self, callee: &Expr) -> Option<Params> {
        if let Some((name, _)) = self.written_fn(callee) {
            return self.fn_params(name);
        }
        if let Some((type_name, _, member)) = self.static_target(callee) {
            return self.method_params_of(&type_name, &member);
        }
        match callee {
            Expr::Ident(name, _) if self.lookup_symbol(name).is_none() => self.fn_params(name),
            Expr::MemberAccess { object, member, .. } => {
                let before = self.errors.len();
                let recv = self.check_expr(object);
                self.errors.truncate(before);
                let (Type::Class { name, .. } | Type::Struct { name, .. } | Type::Enum { name, .. }) = recv else { return None };
                self.method_params_of(&name, member)
            }
            _ => None,
        }
    }

    fn fn_params(&self, name: &str) -> Option<Params> {
        let sig = self.fn_sigs.get(name)?;
        let names = sig.modes.iter().map(|(n, _)| n.clone()).collect();
        let generic = !sig.type_params.is_empty() || self.bounded_fns.contains_key(name);
        Some(Params { callee: name.to_string(), names, required: sig.required, variadic: sig.variadic.is_some(), generic })
    }

    fn method_params_of(&self, type_name: &str, member: &str) -> Option<Params> {
        let sig = self.class_method_sigs.get(&(type_name.to_string(), member.to_string()))?;
        let names = sig.modes.iter().map(|(n, _)| n.clone()).collect();
        let generic = !sig.type_params.is_empty() || self.generics.get(type_name).is_some_and(|g| !g.is_empty());
        Some(Params { callee: format!("{type_name}::{member}"), names, required: sig.required, variadic: sig.variadic.is_some(), generic })
    }

    /// Records a zero-argument function answering `params[i]` for each default parameter, the call a skipped default becomes.
    pub(super) fn register_default_sigs(&mut self, callee: &str, params: &[Type], required: usize) {
        for (i, ty) in params.iter().enumerate().skip(required) {
            let sig = FnSig { type_params: Vec::new(), bounds: Vec::new(), params: Vec::new(), required: 0, ret: ty.clone(), variadic: None, modes: Vec::new() };
            self.fn_sigs.insert(format!("{callee}::default_{i}"), sig);
        }
    }

    /// The call with its arguments in parameter order, or `None` after reporting why it cannot be.
    pub(super) fn reorder_named_call(&mut self, callee: &Expr, args: &[Expr], names: &[Option<String>], span: Span) -> Option<Expr> {
        let Some(p) = self.callee_params(callee) else {
            self.errors.push(("named arguments need a function or method that names its parameters".into(), span));
            return None;
        };
        let what = p.callee.replace("::", ".");
        let first = names.iter().position(Option::is_some)?;
        if let Some(i) = names[first..].iter().position(Option::is_none) {
            self.errors.push(("a positional argument cannot follow a named one".into(), args[first + i].span()));
            return None;
        }
        if p.variadic {
            self.errors.push((format!("`{what}` takes a variable number of arguments, so none can be named"), span));
            return None;
        }
        let total = p.names.len();
        if first > total {
            self.errors.push((format!("`{what}` takes {total} argument(s), but {} were given", args.len()), span));
            return None;
        }
        let mut slots: Vec<Option<usize>> = (0..total).map(|i| (i < first).then_some(i)).collect();
        for (i, name) in names.iter().enumerate().skip(first) {
            let name = name.as_ref()?;
            let Some(slot) = p.names.iter().position(|n| n == name) else {
                self.errors.push((format!("`{what}` has no parameter `{name}`"), args[i].span()));
                return None;
            };
            if slots[slot].is_some() {
                self.errors.push((format!("`{name}` is given twice"), args[i].span()));
                return None;
            }
            slots[slot] = Some(i);
        }
        if let Some(missing) = slots.iter().enumerate().find(|(i, s)| s.is_none() && *i < p.required) {
            self.errors.push((format!("`{what}` needs an argument for `{}`", p.names[missing.0]), span));
            return None;
        }
        let last = slots.iter().rposition(Option::is_some)?;
        if p.generic && slots[..last].iter().any(Option::is_none) {
            self.errors.push((format!("`{what}` is generic, so a default cannot be skipped; give every argument up to the last one named"), span));
            return None;
        }
        let hole = |i: usize| {
            let at = |extra: usize| Span { start: HIDDEN_BASE + (span.start << 8) + 2 * i + extra, end: HIDDEN_BASE + (span.start << 8) + 2 * i + extra, ..span };
            Expr::Call { callee: Box::new(Expr::Ident(format!("{}::default_{i}", p.callee), at(1))), args: Vec::new(), names: Vec::new(), span: at(0) }
        };
        let new_args = slots[..=last].iter().enumerate().map(|(i, s)| s.map_or_else(|| hole(i), |a| args[a].clone())).collect();
        Some(Expr::Call { callee: Box::new(callee.clone()), args: new_args, names: Vec::new(), span })
    }
}
