//! List and map literal and `?:` typing.

use super::*;

pub(super) struct MixedLiteral {
    pub span: Span,
    pub message: String,
}

impl TypeChecker {
    pub(super) fn list_literal_type(&mut self, elements: &[Expr], span: Span) -> Type {
        let tys: Vec<Type> = elements.iter().map(|e| self.check_expr(e)).collect();
        match Self::join_all(&tys) {
            Ok(t) => Type::List(Box::new(t)),
            Err(mixed) => {
                let (a, b) = *mixed;
                self.defer_mixed(span, "list elements", &a, &b, "List<_>");
                Type::List(Box::new(Type::Any))
            }
        }
    }

    pub(super) fn map_literal_type(&mut self, entries: &[(Expr, Expr)], span: Span) -> Type {
        let mut seen: HashSet<String> = HashSet::new();
        let mut keys = Vec::new();
        let mut vals = Vec::new();
        for (k, v) in entries {
            keys.push(self.check_expr(k));
            vals.push(self.check_expr(v));
            if let Some(repr) = Self::literal_key(k)
                && !seen.insert(repr.clone()) {
                    self.errors.push((format!("key {repr} appears twice in this map literal"), k.span()));
                }
        }
        let k_ty = Self::join_all(&keys).unwrap_or_else(|mixed| {
            let (a, b) = *mixed;
            self.defer_mixed(span, "map keys", &a, &b, "Map<_, …>");
            Type::Any
        });
        let v_ty = Self::join_all(&vals).unwrap_or_else(|mixed| {
            let (a, b) = *mixed;
            self.defer_mixed(span, "map values", &a, &b, "Map<…, _>");
            Type::Any
        });
        self.check_key_type(&k_ty, span);
        Type::Map(Box::new(k_ty), Box::new(v_ty))
    }

    pub(super) fn ternary_type(&mut self, cond: &Expr, then_expr: &Expr, else_expr: &Expr, span: Span) -> Type {
        let cond_ty = self.check_expr(cond);
        if cond_ty != Type::Bool && cond_ty != Type::Any {
            self.errors.push((
                format!("the condition of `?:` must be `Bool`, found `{}`", Self::describe(&cond_ty)),
                cond.span(),
            ));
        }
        let facts = self.facts(cond);
        let a = self.check_assuming(facts.when_true, then_expr);
        let b = self.check_assuming(facts.when_false, else_expr);
        Self::join(&a, &b).unwrap_or_else(|| {
            if !self.mixed_literals.iter().any(|m| m.span == span) {
                let example = Self::union_hint(&a, &b).map(|u| format!(", for example `{u}`")).unwrap_or_default();
                let message = format!(
                    "the branches of `?:` have different types, `{}` and `{}`; annotate the result's type{example}",
                    Self::describe(&a),
                    Self::describe(&b)
                );
                self.mixed_literals.push(MixedLiteral { span, message });
            }
            Type::Any
        })
    }

    pub(super) fn settle(&mut self, expr: &Expr, expected: &Type, ty: Type) -> Type {
        self.resolve_literal(expr, expected);
        let now = self.expr_types.get(&expr.span()).cloned().unwrap_or(ty);
        let fills = !expected.has_hole() && *expected != Type::Any;
        if now.has_hole() && now != Type::Hole && fills && now.is_assignable_to(expected) {
            self.expr_types.insert(expr.span(), expected.clone());
            return expected.clone();
        }
        now
    }

    pub(super) fn reject_hole(&mut self, kw: &str, name: &str, ty: &Type, span: Span) {
        let shown = if *ty == Type::Null { "_?".to_string() } else { format!("{ty:?}") };
        if ty.has_hole() || *ty == Type::Null {
            let msg = format!("the type of `{name}` is not fully known (`{shown}`); annotate it, for example `{kw} {name}: …`");
            self.errors.push((msg, span));
        }
    }

    pub(super) fn takes_context(expr: &Expr) -> bool {
        matches!(expr, Expr::ListLiteral { .. } | Expr::MapLiteral { .. } | Expr::Ternary { .. })
    }

    pub(super) fn resolve_literal(&mut self, expr: &Expr, expected: &Type) {
        let (entries, span): (Vec<(&Expr, &Type)>, Span) = match (expr, expected) {
            (Expr::ListLiteral { elements, span }, Type::List(e)) => (elements.iter().map(|x| (x, &**e)).collect(), *span),
            (Expr::MapLiteral { entries, span }, Type::Map(k, v)) => {
                (entries.iter().flat_map(|(ke, ve)| [(ke, &**k), (ve, &**v)]).collect(), *span)
            }
            (Expr::Ternary { then_expr, else_expr, span, .. }, t) => (vec![(&**then_expr, t), (&**else_expr, t)], *span),
            _ => return,
        };
        for (e, t) in &entries {
            self.resolve_literal(e, t);
        }
        let pending = self.mixed_literals.iter().position(|m| m.span == span);
        let fits = entries
            .iter()
            .all(|(e, t)| self.expr_types.get(&e.span()).is_none_or(|got| got.is_assignable_to(t)));
        if fits {
            if let Some(pos) = pending {
                self.mixed_literals.remove(pos);
            }
            self.expr_types.insert(span, expected.clone());
        } else if let (Some(pos), Expr::Ternary { .. }) = (pending, expr) {
            let (e, got) = entries
                .iter()
                .find_map(|(e, t)| self.expr_types.get(&e.span()).filter(|g| !g.is_assignable_to(t)).map(|g| (*e, g.clone())))
                .expect("a branch that does not fit");
            self.mixed_literals.remove(pos);
            let message = format!("this `?:` branch is `{}`, but `{}` is expected", Self::describe(&got), Self::describe(expected));
            self.errors.push((message, e.span()));
        }
    }

    pub(super) fn report_mixed_literals(&mut self) {
        for m in std::mem::take(&mut self.mixed_literals) {
            self.errors.push((m.message, m.span));
        }
    }

    fn defer_mixed(&mut self, span: Span, what: &str, a: &Type, b: &Type, hint: &str) {
        if self.mixed_literals.iter().any(|m| m.span == span) {
            return;
        }
        let part = Self::union_hint(a, b).unwrap_or_else(|| "Any".into());
        let message = format!(
            "{what} have different types, `{}` and `{}`; annotate the literal's type (for example `{}`)",
            Self::describe(a),
            Self::describe(b),
            hint.replace('_', &part)
        );
        self.mixed_literals.push(MixedLiteral { span, message });
    }

    fn join_all(tys: &[Type]) -> Result<Type, Box<(Type, Type)>> {
        let Some((first, rest)) = tys.split_first() else { return Ok(Type::Hole) };
        rest.iter().try_fold(first.clone(), |acc, t| Self::join(&acc, t).ok_or_else(|| Box::new((acc.clone(), t.clone()))))
    }

    pub(super) fn join(a: &Type, b: &Type) -> Option<Type> {
        if a == b {
            return Some(a.clone());
        }
        if *a == Type::Any || *b == Type::Any {
            return Some(Type::Any);
        }
        match (a, b) {
            (Type::Null, Type::Nullable(_)) => return Some(b.clone()),
            (Type::Nullable(_), Type::Null) => return Some(a.clone()),
            (Type::Null, t) | (t, Type::Null) => return Some(Type::Nullable(Box::new(t.clone()))),
            _ => {}
        }
        match (b.is_assignable_to(a), a.is_assignable_to(b)) {
            (true, true) => Some(if a.has_hole() { b.clone() } else { a.clone() }),
            (true, false) => Some(a.clone()),
            (false, true) => Some(b.clone()),
            (false, false) => None,
        }
    }

    fn literal_key(k: &Expr) -> Option<String> {
        match k {
            Expr::String(s, _) => Some(format!("{s:?}")),
            Expr::Int(n, _) => Some(n.to_string()),
            Expr::Char(c, _) => Some(format!("{c:?}")),
            Expr::Bool(b, _) => Some(b.to_string()),
            _ => None,
        }
    }
}
