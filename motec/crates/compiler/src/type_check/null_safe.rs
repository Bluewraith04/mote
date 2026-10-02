//! `??`, `??=` and `?.` typing.

use super::*;

impl TypeChecker {
    fn optional_payload(&mut self, op: &str, ty: &Type, span: Span) -> Option<Type> {
        match ty {
            Type::Nullable(t) => Some((**t).clone()),
            Type::Any | Type::Null => Some(Type::Any),
            other => {
                let msg = format!("`{op}` needs an optional on its left, found `{}`", Self::describe(other));
                self.errors.push((msg, span));
                None
            }
        }
    }

    pub(super) fn coalesce_type(&mut self, left: &Expr, right: &Expr) -> Type {
        let lt = self.check_expr(left);
        if lt == Type::Null {
            return self.check_expr(right);
        }
        let Some(payload) = self.optional_payload("??", &lt, left.span()) else {
            self.check_expr(right);
            return Type::Any;
        };
        let rt = self.check_expr_expecting(right, Some(payload.clone()));
        if payload == Type::Any || rt == Type::Any {
            self.note_coercion(right.span(), &rt, &payload);
            return Type::Any;
        }
        if rt.is_assignable_to(&payload) {
            self.note_coercion(right.span(), &rt, &payload);
            return payload;
        }
        let optional = Type::Nullable(Box::new(payload.clone()));
        if rt.is_assignable_to(&optional) {
            self.note_coercion(right.span(), &rt, &optional);
            return optional;
        }
        let msg = format!(
            "the right side of `??` must be `{}` or `{}`, found `{}`",
            Self::describe(&payload),
            Self::describe(&optional),
            Self::describe(&rt)
        );
        self.errors.push((msg, right.span()));
        Type::Any
    }

    pub(super) fn coalesce_assign(&mut self, target: &Expr, target_ty: &Type, value: &Expr) {
        let vt = self.check_expr_expecting(value, Some(target_ty.clone()));
        if self.optional_payload("??=", target_ty, target.span()).is_none() {
            return;
        }
        if !vt.is_assignable_to(target_ty) {
            let msg = format!("`??=` cannot assign `{}` to `{}`", Self::describe(&vt), Self::describe(target_ty));
            self.errors.push((msg, value.span()));
        }
        self.note_coercion(value.span(), &vt, target_ty);
    }

    pub(super) fn optional_chain_type(&mut self, object: &Expr, temp: &str, body: &Expr) -> Type {
        let ot = self.check_expr(object);
        let payload = self.optional_payload("?.", &ot, object.span()).unwrap_or(Type::Any);
        self.push_scope();
        self.insert_symbol(temp, payload.clone(), false);
        self.inherit_read_only(temp, object, &payload);
        let bt = self.check_expr(body);
        self.pop_scope();
        match bt {
            Type::Nullable(_) | Type::Any | Type::Null => bt,
            other => Type::Nullable(Box::new(other)),
        }
    }
}
