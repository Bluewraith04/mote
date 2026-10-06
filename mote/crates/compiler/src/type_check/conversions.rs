//! Conversions through `T.from`.

use super::*;

pub(crate) fn builtin_from_target(name: &str) -> Option<(Type, Type)> {
    Some(match name {
        "Int" => (Type::Int, Type::union(vec![Type::Float, Type::Bool, Type::Char])),
        "Float" => (Type::Float, Type::Int),
        "String" => (Type::String, Type::Any),
        _ => return None,
    })
}

impl TypeChecker {
    pub(super) fn builtin_from(&mut self, callee: &Expr, args: &[Type], span: Span) -> Option<Type> {
        let Expr::MemberAccess { object, member, .. } = callee else { return None };
        let Expr::Ident(name, _) = object.as_ref() else { return None };
        if member != "from" || self.lookup_symbol(name).is_some() {
            return None;
        }
        let (target, takes) = builtin_from_target(name)?;
        if args.len() != 1 {
            self.errors.push((format!("`{name}.from` takes 1 argument, but {} were given", args.len()), span));
            return Some(target);
        }
        if !args[0].is_assignable_to(&takes) {
            let hint = match (&target, &args[0]) {
                (Type::Int, Type::String) => "; to read a number from text, use `std.string.parse_int`",
                (Type::Float, Type::String) => "; to read a number from text, use `std.string.parse_float`",
                _ => "",
            };
            self.errors.push((format!("`{name}.from` takes `{takes:?}`, found `{:?}`{hint}", args[0]), span));
        }
        self.note_arg_check(0, &args[0], &takes);
        Some(target)
    }

    pub(super) fn check_from_decl(&mut self, type_name: &str, f: &FunctionDecl) {
        if f.name != "from" {
            return;
        }
        let takes_self = matches!(f.params.first().and_then(|p| p.kind.as_ref()), Some(ParameterKind::SelfValue { .. }));
        let returns_self = match &f.return_type {
            Some(TypeNode::SelfType(_)) => true,
            Some(TypeNode::Named(n, _) | TypeNode::Generic(n, _, _)) => n == type_name,
            _ => false,
        };
        let problem = if takes_self {
            Some("is static: it takes no `self`")
        } else if f.params.len() != 1 {
            Some("takes exactly one parameter")
        } else if !returns_self {
            Some("returns `Self`; a conversion that can fail needs another name, such as `parse`")
        } else {
            None
        };
        if let Some(p) = problem {
            self.errors.push((format!("`{type_name}.from` {p}"), f.span));
        }
    }
}
