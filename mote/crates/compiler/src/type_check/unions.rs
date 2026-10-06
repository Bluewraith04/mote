//! Union types: member rules, common members, narrowing remainders.

use super::*;
use crate::types::Members;

impl TypeChecker {
    pub(super) fn resolve_union(&self, nodes: &[TypeNode], span: Span) -> Type {
        let members: Vec<Type> = nodes.iter().map(|n| self.resolve_type_node(n)).collect();
        let flat: Vec<Type> = members.iter().flat_map(Type::members).collect();
        let err = |msg: String| self.type_errors.borrow_mut().push((msg, span));
        if flat.iter().any(|t| matches!(t, Type::Nullable(_) | Type::Null)) {
            let fixed = Type::union(members.clone());
            err(format!("a union member can't be optional; write `{fixed:?}`"));
        }
        for bad in flat.iter().filter(|t| matches!(t, Type::Any | Type::Null)) {
            err(format!("`{bad:?}` can't be a union member"));
        }
        let clash = |a: &Type, b: &Type| match (a, b) {
            (Type::Function { .. }, Type::Function { .. }) => Some("two function types"),
            (Type::Tuple(x), Type::Tuple(y)) if x.len() == y.len() => Some("two tuple types of the same length"),
            _ => None,
        };
        for (i, a) in flat.iter().enumerate() {
            if let Some((b, what)) = flat[i + 1..].iter().find_map(|b| clash(a, b).filter(|_| !a.same_as(b)).map(|w| (b, w))) {
                err(format!("a union can't hold {what}, `{a:?}` and `{b:?}`: a value can't be told apart at run time"));
            }
        }
        Type::union(members)
    }

    pub(super) fn union_hit(ty: &Type, target: &Type) -> Option<Type> {
        let Type::Union(Members(ms)) = ty else { return None };
        let hit: Vec<Type> = ms.iter().filter(|m| m.is_assignable_to(target)).cloned().collect();
        (!hit.is_empty()).then(|| Type::union(hit))
    }

    pub(super) fn union_rest(ty: &Type, target: &Type) -> Option<Type> {
        let Type::Union(Members(ms)) = ty else { return None };
        let rest: Vec<Type> = ms.iter().filter(|m| !m.is_assignable_to(target)).cloned().collect();
        (!rest.is_empty() && rest.len() < ms.len()).then(|| Type::union(rest))
    }

    fn join_or_union(a: &Type, b: &Type) -> Type {
        Self::join(a, b).unwrap_or_else(|| Type::union(vec![a.clone(), b.clone()]))
    }

    pub(super) fn union_member(&mut self, ty: &Type, member: &str, span: Span, each: impl Fn(&mut Self, &Type) -> Type) -> Type {
        let mut result: Option<Type> = None;
        for m in ty.members() {
            let before = self.errors.len();
            let got = each(self, &m);
            if self.errors.len() > before {
                self.errors.truncate(before);
                let msg = format!("`.{member}` is not on every member of `{ty:?}`: `{m:?}` has none; narrow it with `is` or `match` first");
                self.errors.push((msg, span));
                return Type::Any;
            }
            result = Some(match result {
                None => got,
                Some(acc) => Self::join_or_union(&acc, &got),
            });
        }
        result.unwrap_or(Type::Any)
    }

    pub(super) fn member_field(&mut self, ty: &Type, member: &str, span: Span) -> Type {
        let found = match self.full_type(ty.clone()) {
            Type::Struct { name, fields, .. } | Type::Class { name, fields, .. } => {
                let hit = fields.iter().find(|(n, _)| n == member).map(|(_, t)| t.clone());
                if hit.is_some() {
                    self.check_field_read(&name, member, span);
                }
                hit
            }
            Type::Tuple(elems) => member.parse::<usize>().ok().and_then(|i| elems.get(i).cloned()),
            _ => None,
        };
        found.unwrap_or_else(|| {
            self.errors.push((format!("`{ty:?}` has no field `{member}`"), span));
            Type::Any
        })
    }

    pub(super) fn check_type_pattern(&mut self, node: &TypeNode, subj: &Type, span: Span) -> Type {
        let target = self.resolve_type_node(node);
        self.flush_type_errors();
        self.type_tests.insert(span, target.clone());
        let payload = match subj {
            Type::Nullable(inner) => inner,
            t => t,
        };
        let bound = if *subj == Type::Any || target.is_assignable_to(subj) {
            Some(target.clone())
        } else if payload.is_assignable_to(&target) && *payload != Type::Any {
            Some(payload.clone())
        } else {
            Self::union_hit(payload, &target)
        };
        bound.unwrap_or_else(|| {
            self.errors.push((format!("this pattern can never match: a value of type `{subj:?}` is never a `{target:?}`"), span));
            Type::Any
        })
    }

    pub(super) fn type_pattern_targets(&self, pat: &Pattern) -> Vec<Type> {
        match pat {
            Pattern::Type { span, .. } => self.type_tests.get(span).cloned().into_iter().collect(),
            Pattern::Or(alts, _) => alts.iter().flat_map(|a| self.type_pattern_targets(a)).collect(),
            _ => Vec::new(),
        }
    }

    pub(super) fn uncovered_members(&self, subj: &Type, covered: &[Type], none_covered: bool) -> Vec<Type> {
        let (payload, optional) = match subj {
            Type::Nullable(inner) => (&**inner, true),
            t => (t, false),
        };
        let mut missing: Vec<Type> = payload.members().into_iter().filter(|m| !covered.iter().any(|c| m.is_assignable_to(c))).collect();
        if optional && !none_covered && !covered.iter().any(|c| Type::Null.is_assignable_to(c)) {
            missing.push(Type::Null);
        }
        missing
    }

    pub(super) fn union_hint(a: &Type, b: &Type) -> Option<String> {
        let plain = |t: &Type| !matches!(t, Type::Any | Type::Null | Type::Hole | Type::Function { .. } | Type::Tuple(_)) && !t.has_hole();
        (plain(a) && plain(b)).then(|| format!("{:?}", Type::union(vec![a.clone(), b.clone()])))
    }

    pub(super) fn union_operand(&mut self, sym: &str, ty: &Type, span: Span) {
        let msg = format!("`{sym}` needs a narrowed operand, but this one is `{ty:?}`; test it with `is` or `match` first");
        self.errors.push((msg, span));
    }
}
