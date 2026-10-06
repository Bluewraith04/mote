//! Static calls: `T(args)` calls `T.new(args)`, and `Name<Args>` in an expression.

use super::*;

pub(crate) fn builtin_ctor_arity(name: &str) -> Option<usize> {
    match name {
        "Map" => Some(2),
        "List" | "Set" | "Channel" | "Shared" => Some(1),
        "Bytes" => Some(0),
        _ => None,
    }
}

impl TypeChecker {
    pub(super) fn is_type_value(&self, name: &str) -> bool {
        self.lookup_symbol(name).is_none()
            && !self.fn_sigs.contains_key(name)
            && (matches!(self.types.get(name), Some(Type::Class { .. } | Type::Struct { .. })) || builtin_ctor_arity(name).is_some())
    }

    fn is_enum_with_method(&self, name: &str, member: &str) -> bool {
        self.lookup_symbol(name).is_none() && matches!(self.types.get(name), Some(Type::Enum { .. })) && self.class_method_sigs.contains_key(&(name.to_string(), member.to_string()))
    }

    pub(super) fn written_fn<'a>(&self, callee: &'a Expr) -> Option<(&'a String, &'a [TypeNode])> {
        match callee {
            Expr::StaticAccess { target: TypeNode::Generic(name, args, _), member, .. }
                if member == "new" && self.lookup_symbol(name).is_none() && self.fn_sigs.contains_key(name) =>
            {
                Some((name, args.as_slice()))
            }
            _ => None,
        }
    }

    pub(super) fn preset_written(&mut self, name: &str, type_params: &[String], written: &[TypeNode], span: Span, preset: &mut HashMap<String, Type>) {
        if written.is_empty() {
            return;
        }
        if written.len() != type_params.len() {
            self.errors.push((format!("`{name}` takes {} type argument(s), but {} were given", type_params.len(), written.len()), span));
            return;
        }
        for (p, t) in type_params.iter().zip(written) {
            preset.insert(p.clone(), self.resolve_type_node(t));
        }
    }

    pub(super) fn static_target<'a>(&self, callee: &'a Expr) -> Option<(String, &'a [TypeNode], String)> {
        match callee {
            Expr::Ident(name, _) if self.is_type_value(name) => Some((name.clone(), &[], "new".into())),
            Expr::MemberAccess { object, member, .. } => match object.as_ref() {
                Expr::Ident(name, _) if self.is_type_value(name) || self.is_enum_with_method(name, member) => Some((name.clone(), &[], member.clone())),
                _ => None,
            },
            Expr::StaticAccess { target: TypeNode::Generic(name, args, _), member, .. } if self.written_fn(callee).is_none() => {
                Some((name.clone(), args.as_slice(), member.clone()))
            }
            Expr::StaticAccess { target: TypeNode::Named(name, _), member, .. } if self.is_type_value(name) || self.is_enum_with_method(name, member) => {
                Some((name.clone(), &[], member.clone()))
            }
            _ => None,
        }
    }

    pub(super) fn check_static_call(&mut self, callee: &Expr, args: &[Type], span: Span) -> Option<Type> {
        let (name, nodes, member) = self.static_target(callee)?;
        let written = !nodes.is_empty();
        let targs: Vec<Type> = nodes.iter().map(|a| self.resolve_type_node(a)).collect();
        if let Some(arity) = builtin_ctor_arity(&name) {
            if member != "new" {
                return None;
            }
            return Some(self.check_builtin_new(&name, arity, written.then_some(targs), args, span));
        }
        if written {
            let want = self.generics.get(&name).map_or(0, Vec::len);
            if targs.len() != want {
                self.errors.push((format!("`{name}` takes {want} type argument(s), but {} were given", targs.len()), span));
            }
        }
        if member == "new" && !self.class_method_sigs.contains_key(&(name.clone(), member.clone())) {
            self.errors.push((format!("`{name}` has no `new`"), span));
            return Some(self.instantiate_type(&name, &targs));
        }
        Some(self.check_class_method_call(&name, &targs, &member, args, true, span))
    }

    fn check_builtin_new(&mut self, name: &str, arity: usize, targs: Option<Vec<Type>>, args: &[Type], span: Span) -> Type {
        if let Some(t) = &targs
            && t.len() != arity {
                self.errors.push((format!("`{name}` takes {arity} type argument(s), but {} were given", t.len()), span));
            }
        let targ = |i: usize| targs.as_ref().and_then(|t| t.get(i).cloned()).unwrap_or(Type::Hole);
        match name {
            "Map" | "Set" | "List" | "Bytes" => {
                if name == "Bytes" && !args.is_empty() {
                    self.one_int_arg("Bytes(n)", args, span);
                } else if !args.is_empty() {
                    self.errors.push((format!("`{name}()` takes no arguments"), span));
                }
                match name {
                    "Map" => {
                        let key = targ(0);
                        if targs.is_some() {
                            self.check_key_type(&key, span);
                        }
                        Type::Map(Box::new(key), Box::new(targ(1)))
                    }
                    "Set" => {
                        let key = targ(0);
                        if targs.is_some() {
                            self.check_key_type(&key, span);
                        }
                        Type::Set(Box::new(key))
                    }
                    "List" => Type::List(Box::new(targ(0))),
                    _ => Type::Bytes,
                }
            }
            "Channel" => {
                self.one_int_arg("Channel(capacity)", args, span);
                let elem = targ(0);
                if !elem.has_hole() && !self.is_sendable(&elem) {
                    self.errors.push((format!("`Channel<{elem:?}>`: a value sent over a channel must be Sendable, and `{elem:?}` is not"), span));
                }
                Type::Tuple(vec![Type::Sender(Box::new(elem.clone())), Type::Receiver(Box::new(elem))])
            }
            _ => {
                if args.len() != 1 {
                    self.errors.push((format!("`{name}(initial)` takes one argument"), span));
                }
                let elem = match (&targs, args.first()) {
                    (Some(_), got) => {
                        let want = targ(0);
                        if let Some(got) = got {
                            if !got.is_assignable_to(&want) {
                                self.errors.push((format!("`{name}<{want:?}>` expects '{want:?}', found '{got:?}'"), span));
                            }
                            self.note_arg_check(0, got, &want);
                        }
                        want
                    }
                    (None, Some(Type::Null) | None) => Type::Hole,
                    (None, Some(t)) => t.clone(),
                };
                self.check_shareable(&elem, span);
                Type::Shared(Box::new(elem))
            }
        }
    }

    fn one_int_arg(&mut self, what: &str, args: &[Type], span: Span) {
        if args.len() != 1 || !args[0].is_assignable_to(&Type::Int) {
            self.errors.push((format!("`{what}` takes one `Int` argument"), span));
        } else {
            self.note_arg_check(0, &args[0], &Type::Int);
        }
    }
}
