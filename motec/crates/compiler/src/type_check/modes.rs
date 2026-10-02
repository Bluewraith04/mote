//! Parameter modes: read-only parameters and receivers, `var` parameters and the call-site `var` mark.

use super::*;

/// Why a binding is read-only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReadOnly {
    No,
    /// A plain parameter or receiver.
    Param,
    /// A binding made from a read-only value.
    Derived,
    /// A `Shared` snapshot, or a binding made from one.
    Snapshot,
}

pub(super) fn param_modes(params: &[Param]) -> Vec<(String, bool)> {
    params
        .iter()
        .filter(|p| matches!(p.kind, Some(ParameterKind::Regular { .. })))
        .map(|p| (p.name.clone(), p.is_mut))
        .collect()
}

pub(super) fn lambda_modes(params: &[Param]) -> Vec<bool> {
    let modes: Vec<bool> = params.iter().map(|p| p.is_mut).collect();
    if modes.contains(&true) { modes } else { Vec::new() }
}

pub(super) fn writes_self(params: &[Param]) -> bool {
    matches!(params.first().and_then(|p| p.kind.as_ref()), Some(ParameterKind::SelfValue { is_var: true }))
}

impl TypeChecker {
    pub(super) fn set_read_only(&mut self, name: &str, why: ReadOnly) {
        if let Some(info) = self.scopes.last_mut().and_then(|s| s.get_mut(name)) {
            info.read_only = why;
        }
    }

    pub(super) fn immutable_reason(&self, place: &Expr) -> Option<String> {
        let root = |name: &str| self.lookup_symbol(name).filter(|i| i.read_only != ReadOnly::No).map(|i| Self::read_only_msg(name, i.read_only));
        match place {
            Expr::Ident(n, _) => root(n).or_else(|| {
                let info = self.lookup_symbol(n)?;
                (!info.is_mutable && matches!(info.ty, Type::Struct { .. })).then(|| format!("`{n}` is a `let` struct; declare it `var {n}`"))
            }),
            Expr::SelfValue(_) => root("self"),
            Expr::MemberAccess { object, member, .. } => {
                let owner = match self.expr_types.get(&object.span()) {
                    Some(Type::Struct { name, .. } | Type::Class { name, .. }) => Some(name.clone()),
                    Some(Type::Nullable(inner)) => match inner.as_ref() {
                        Type::Struct { name, .. } | Type::Class { name, .. } => Some(name.clone()),
                        _ => None,
                    },
                    _ => None,
                };
                match owner {
                    Some(t) if self.field_write_reason(&t, member).is_some() => self.field_write_reason(&t, member),
                    Some(t) if !self.var_fields.contains(&(t.clone(), member.clone())) => Some(format!("field `{member}` is not `var`; declare it `var {member}`")),
                    _ => self.immutable_reason(object),
                }
            }
            Expr::Index { object, .. } => self.immutable_reason(object),
            Expr::Unwrap { expr, .. } => self.immutable_reason(expr),
            Expr::Call { .. } if self.is_snapshot(place) => Some("a snapshot is read-only; `.clone()` it to change it".into()),
            _ => None,
        }
    }

    pub(super) fn check_writable(&mut self, place: &Expr, span: Span) {
        if let Some(msg) = self.immutable_reason(place) {
            self.errors.push((msg, span));
        }
    }

    pub(super) fn assign_msg(name: &str, info: &SymbolInfo) -> String {
        match info.read_only {
            ReadOnly::Param => Self::read_only_msg(name, info.read_only),
            _ => format!("Mutability Violation: cannot assign to immutable constant '{name}'"),
        }
    }

    fn read_only_msg(name: &str, why: ReadOnly) -> String {
        match why {
            _ if name == "self" => "`self` is read-only; declare the method with `var self`".into(),
            ReadOnly::Param => format!("`{name}` is a read-only parameter; declare it `var {name}`"),
            ReadOnly::Snapshot => format!("`{name}` is a snapshot; `.clone()` it to change it"),
            _ => format!("`{name}` is read-only"),
        }
    }

    pub(super) fn inherit_read_only(&mut self, name: &str, init: &Expr, ty: &Type) {
        if Self::is_copy(ty) {
            return;
        }
        if self.is_snapshot(init) {
            self.set_read_only(name, ReadOnly::Snapshot);
        } else if self.immutable_reason(init).is_some() {
            self.set_read_only(name, ReadOnly::Derived);
        }
    }

    pub(super) fn bind_read_only(&mut self, name: &str, ty: Type) {
        self.insert_symbol(name, ty, false);
        self.set_read_only(name, ReadOnly::Derived);
    }

    fn is_copy(ty: &Type) -> bool {
        infer::is_scalar(ty) || matches!(ty, Type::Char | Type::Null | Type::Struct { .. })
    }

    fn call_modes(&self, callee: &Expr) -> Option<(Vec<(String, bool)>, bool)> {
        if let Some((name, _)) = self.written_fn(callee) {
            return self.fn_sigs.get(name).map(|s| (s.modes.clone(), false));
        }
        if let Some((name, _, member)) = self.static_target(callee) {
            return self.class_method_sigs.get(&(name, member)).map(|s| (s.modes.clone(), false));
        }
        if let Some(Type::Function { params, modes, .. }) = self.expr_types.get(&callee.span()) {
            let slots = (0..params.len()).map(|i| (String::new(), modes.get(i) == Some(&true))).collect();
            return Some((slots, false));
        }
        match callee {
            Expr::Ident(name, _) if self.lookup_symbol(name).is_none() => self.fn_sigs.get(name).map(|s| (s.modes.clone(), false)),
            Expr::MemberAccess { object, member, .. } => {
                let recv = self.expr_types.get(&object.span())?;
                match recv {
                    Type::Class { name, .. } | Type::Struct { name, .. } | Type::Enum { name, .. } => {
                        self.class_method_sigs.get(&(name.clone(), member.clone())).map(|s| (s.modes.clone(), s.writes_self))
                    }
                    Type::List(_) | Type::Map(..) | Type::Set(_) | Type::Bytes => Some((Vec::new(), Self::is_mutating_method(member))),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub(super) fn check_call_modes(&mut self, callee: &Expr, args: &[Expr]) {
        if let Expr::MemberAccess { object, member, span } = callee
            && matches!(member.as_str(), "update" | "set") && matches!(self.expr_types.get(&object.span()), Some(Type::Shared(_))) {
                self.check_cell_write(object, *span);
                return;
            }
        let modes = self.call_modes(callee);
        if let (Some((_, true)), Expr::MemberAccess { object, span, .. }) = (&modes, callee) {
            self.check_writable(object, *span);
        }
        let params = modes.map(|(m, _)| m).unwrap_or_default();
        for (a, _) in args.iter().zip(&params).filter(|(_, (_, is_var))| *is_var) {
            self.check_var_arg(a);
        }
    }

    fn check_var_arg(&mut self, arg: &Expr) {
        let span = arg.span();
        match arg {
            Expr::Ident(n, _) => match self.lookup_symbol(n) {
                Some(info) if info.read_only != ReadOnly::No => self.errors.push((Self::read_only_msg(n, info.read_only), span)),
                Some(info) if !info.is_mutable => self.errors.push((format!("`{n}` is a `let`, and this parameter is `var`; declare it `var {n}`"), span)),
                _ => {}
            },
            Expr::SelfValue(_) | Expr::MemberAccess { .. } | Expr::Index { .. } => self.check_writable(arg, span),
            _ => {}
        }
    }
}
