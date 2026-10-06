//! `Shared<T>`: making a cell, snapshots, `update`, `set`, `wait_until` and `.clone()`.

use super::*;

pub(super) fn unknown_type(name: &str) -> String {
    match name {
        "Mutex" => "there is no `Mutex`; use `Shared(x)` and `.update()`".into(),
        "Atomic" => "there is no `Atomic`; use `Shared(0)` and `.update()`".into(),
        "Channel" => "`Channel` is not a type; write `Sender<T>` or `Receiver<T>`".into(),
        "Any" =>"`Any` is not in scope; `import { Any } from std.experimental.types`".into(),
        _ => format!("unknown type `{name}`"),
    }
}

impl TypeChecker {
    pub(super) fn check_shareable(&mut self, ty: &Type, span: Span) {
        if let Some(what) = self.unshareable_part(ty, &mut HashSet::new()) {
            self.errors.push((format!("a `Shared` cannot hold {what}"), span));
        }
    }

    fn unshareable_part(&self, ty: &Type, seen: &mut HashSet<String>) -> Option<&'static str> {
        match self.full_type(ty.clone()) {
            Type::Function { .. } => Some("a function"),
            Type::Stream(_) => Some("a Stream"),
            Type::List(t) | Type::Set(t) | Type::Nullable(t) => self.unshareable_part(&t, seen),
            Type::Map(k, v) => self.unshareable_part(&k, seen).or_else(|| self.unshareable_part(&v, seen)),
            Type::Tuple(ts) | Type::Union(crate::types::Members(ts)) => ts.iter().find_map(|t| self.unshareable_part(t, seen)),
            Type::Struct { name, fields, .. } | Type::Class { name, fields, .. } if seen.insert(name.clone()) => {
                fields.iter().find_map(|(_, t)| self.unshareable_part(t, seen))
            }
            _ => None,
        }
    }

    pub(crate) fn check_share(&mut self, object: &Expr, recv: &Type, method: &str, args: &[Type], span: Span) -> Type {
        if method == "shared" {
            self.errors.push(("no method `.shared()`; write `Shared(x)`".into(), span));
            return Type::Any;
        }
        if !args.is_empty() {
            self.errors.push((format!("`.into_shared()` takes 0 argument(s), but {} were given", args.len()), span));
        }
        self.check_shareable(recv, span);
        if let Expr::Ident(name, _) = object {
            self.set_moved(name, Some("into_shared"));
        }
        Type::Shared(Box::new(recv.clone()))
    }

    pub(super) fn check_shared_method(&mut self, elem: &Type, method: &str, args: &[Type], span: Span) -> Type {
        let arity = |c: &mut Self, n: usize| {
            if args.len() != n {
                c.errors.push((format!("`.{method}()` takes {n} argument(s), but {} were given", args.len()), span));
            }
        };
        match method {
            "get" => {
                arity(self, 0);
                elem.clone()
            }
            "update" => {
                arity(self, 1);
                match args.first() {
                    Some(Type::Function { params, ret, .. }) if params.len() == 1 => {
                        if !elem.is_assignable_to(&params[0]) {
                            self.errors.push((format!("`.update()` expects a function of '{elem:?}', found one of '{:?}'", params[0]), span));
                        }
                        if !matches!(**ret, Type::Null | Type::Any) && ret.is_assignable_to(elem) {
                            self.errors.push(("`.update()`'s function returns nothing; change its parameter instead".into(), span));
                        }
                    }
                    Some(Type::Any) | None => {}
                    Some(got) => self.errors.push((format!("`.update()` expects a function of '{elem:?}', found '{got:?}'"), span)),
                }
                Type::Null
            }
            "wait_until" => {
                arity(self, 1);
                match args.first() {
                    Some(Type::Function { params, ret, .. }) if params.len() == 1 => {
                        if !elem.is_assignable_to(&params[0]) {
                            self.errors.push((format!("`.wait_until()` expects a function of '{elem:?}', found one of '{:?}'", params[0]), span));
                        }
                        if !matches!(**ret, Type::Bool | Type::Any) {
                            self.errors.push((format!("`.wait_until()`'s function must return 'Bool', found '{ret:?}'"), span));
                        }
                    }
                    Some(Type::Any) | None => {}
                    Some(got) => self.errors.push((format!("`.wait_until()` expects a function of '{elem:?}', found '{got:?}'"), span)),
                }
                elem.clone()
            }
            "set" => {
                arity(self, 1);
                if let Some(got) = args.first() {
                    if !got.is_assignable_to(elem) {
                        self.errors.push((format!("`.set()` expects '{elem:?}', found '{got:?}'"), span));
                    }
                    self.note_arg_check(0, got, elem);
                }
                Type::Null
            }
            _ => {
                self.errors.push((format!("a `Shared` has no method `.{method}()`; call it on a snapshot: `.get().{method}()`"), span));
                Type::Any
            }
        }
    }

    pub(super) fn check_clone(&mut self, recv: &Type, args: &[Type], span: Span) -> Option<Type> {
        if let Type::Class { name, .. } | Type::Struct { name, .. } = recv
            && self.class_method_sigs.contains_key(&(name.clone(), "clone".to_string())) {
                return None;
            }
        if !args.is_empty() {
            self.errors.push((format!("`.clone()` takes 0 argument(s), but {} were given", args.len()), span));
        }
        if matches!(recv, Type::Stream(_)) {
            self.errors.push(("cannot clone a Stream".into(), span));
        }
        Some(recv.clone())
    }

    pub(super) fn is_snapshot(&self, e: &Expr) -> bool {
        match e {
            Expr::Call { callee, args, .. } if args.len() <= 1 => match callee.as_ref() {
                Expr::MemberAccess { object, member, .. } => {
                    matches!(member.as_str(), "get" | "wait_until") && matches!(self.expr_types.get(&object.span()), Some(Type::Shared(_)))
                }
                _ => false,
            },
            Expr::Ident(n, _) => self.lookup_symbol(n).is_some_and(|i| i.read_only == modes::ReadOnly::Snapshot),
            Expr::MemberAccess { object, .. } | Expr::Index { object, .. } => self.is_snapshot(object),
            Expr::Unwrap { expr, .. } => self.is_snapshot(expr),
            _ => false,
        }
    }

    pub(super) fn check_cell_write(&mut self, object: &Expr, span: Span) {
        let Expr::Ident(n, _) = object else { return };
        match self.lookup_symbol(n) {
            Some(info) if info.read_only != modes::ReadOnly::No => self.check_writable(object, span),
            Some(info) if !info.is_mutable => self.errors.push((format!("`{n}` is a `let`; only a `var` can write a `Shared`"), span)),
            _ => {}
        }
    }
}
