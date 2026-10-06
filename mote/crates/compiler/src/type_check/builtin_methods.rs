//! Type rules for built-in and class method calls.

use super::*;

impl TypeChecker {
    pub(crate) fn check_index(&mut self, recv: &Type, index: Type, value: Option<Type>, span: Span) -> Type {
        if matches!(recv, Type::Shared(_)) {
            self.errors.push(("a `Shared` cannot be indexed; index a snapshot: `.get()[…]`".into(), span));
            return Type::Any;
        }
        match recv {
            Type::List(_) | Type::Map(..) | Type::Bytes | Type::Any => {
                let (method, args) = match value {
                    Some(v) => ("set", vec![index, v]),
                    None => ("get", vec![index]),
                };
                let first_new = self.errors.len();
                let ty = self.check_builtin_method(recv, method, &args, span);
                for (msg, _) in &mut self.errors[first_new..] {
                    *msg = msg.replace(&format!("`.{method}()`"), "`[]`").replace("cannot call `[]`", "cannot assign through `[]`");
                }
                ty
            }
            Type::String => {
                self.errors.push((
                    "a `String` has no integer indexing; use `.bytes()`, `.find` or `.slice`".into(),
                    span,
                ));
                Type::Any
            }
            other => {
                self.errors.push((
                    format!("`{}` cannot be indexed; `[]` works on `List`, `Map` and `Bytes`", Self::describe(other)),
                    span,
                ));
                Type::Any
            }
        }
    }

    pub(crate) fn check_builtin_method(&mut self, recv: &Type, method: &str, args: &[Type], span: Span) -> Type {
        let arity = |this: &mut Self, n: usize| {
            if args.len() != n {
                this.errors.push((
                    format!("`.{method}()` takes {n} argument(s), but {} were given", args.len()),
                    span,
                ));
            }
        };
        let want_self = |this: &mut Self, i: usize| {
            if let Some(got) = args.get(i) {
                if !got.is_assignable_to(recv) {
                    this.errors.push((
                        format!("`.{method}()` expects '{}', found '{got:?}'", TypeChecker::describe(recv)),
                        span,
                    ));
                }
                this.note_arg_check(i, got, recv);
            }
        };
        if method == "to_string" {
            arity(self, 0);
            return Type::String;
        }
        if matches!(recv, Type::Hole) {
            self.errors.push((format!("the type of the value before `.{method}()` is not known; write it, for example `|v: Type|`"), span));
            return Type::Any;
        }
        if let Type::Enum { name, args: targs, .. } = recv
            && self.class_method_sigs.contains_key(&(name.clone(), method.to_string())) {
                return self.check_class_method_call(&name.clone(), &targs.clone(), method, args, false, span);
            }
        if let Type::Union(_) = recv {
            return self.union_member(recv, &format!("{method}()"), span, |this, m| this.check_builtin_method(m, method, args, span));
        }
        if let Type::Param(p) = recv {
            if let Some(ret) = self.bound_method_call(p, method, args, span) {
                return ret;
            }
            self.errors.push((format!("`{p}` is a type parameter with no bound, so it has no method `.{method}()`"), span));
            return Type::Any;
        }
        match recv {
            Type::Shared(inner) => {
                let inner = (**inner).clone();
                self.check_shared_method(&inner, method, args, span)
            }
            Type::Nullable(inner) if matches!(method, "is_some" | "is_none" | "unwrap" | "unwrap_or" | "unwrap_or_else" | "and_then" | "map" | "ok_or") => {
                let inner = (**inner).clone();
                let callback_ret = match args.first() {
                    Some(Type::Function { ret, .. }) => Some((**ret).clone()),
                    _ => None,
                };
                match method {
                    "is_some" | "is_none" => {
                        arity(self, 0);
                        Type::Bool
                    }
                    "unwrap" => {
                        arity(self, 0);
                        inner
                    }
                    "unwrap_or" => {
                        arity(self, 1);
                        if let Some(got) = args.first() {
                            if !got.is_assignable_to(&inner) {
                                self.errors.push((format!("`.unwrap_or()` expects '{inner:?}', found '{got:?}'"), span));
                            }
                            self.note_arg_check(0, got, &inner);
                        }
                        inner
                    }
                    "unwrap_or_else" => {
                        arity(self, 1);
                        inner
                    }
                    "and_then" => {
                        arity(self, 1);
                        callback_ret.unwrap_or(Type::Any)
                    }
                    "map" => {
                        arity(self, 1);
                        Type::Nullable(Box::new(callback_ret.unwrap_or(Type::Any)))
                    }
                    _ => {
                        arity(self, 1);
                        let e = args.first().cloned().unwrap_or(Type::Any);
                        self.result_of(inner, e)
                    }
                }
            }
            Type::Nullable(_) => {
                self.optional_access(recv, &format!("{method}()"), span);
                Type::Any
            }
            Type::Enum { name, variants, args: enum_args } => {
                let has = |v: &str| variants.iter().any(|(n, _)| n == v);
                let is_option_like = has("Some") || has("None") || has("Ok") || has("Err");
                let is_option = has("Some") || has("None");
                let is_result = has("Ok") || has("Err");
                let callback_ret = match args.first() {
                    Some(Type::Function { ret, .. }) => Some((**ret).clone()),
                    _ => None,
                };
                match method {
                    "is_some" | "is_none" if has("Some") || has("None") => {
                        arity(self, 0);
                        Type::Bool
                    }
                    "is_ok" | "is_err" if has("Ok") || has("Err") => {
                        arity(self, 0);
                        Type::Bool
                    }
                    "unwrap" if is_option_like => {
                        arity(self, 0);
                        Self::success_payload(recv).unwrap_or(Type::Any)
                    }
                    "unwrap_or" | "unwrap_or_else" if is_option_like => {
                        arity(self, 1);
                        Self::success_payload(recv).unwrap_or(Type::Any)
                    }
                    "and_then" if is_option_like => {
                        arity(self, 1);
                        callback_ret.unwrap_or(Type::Any)
                    }
                    "map" if is_option_like => {
                        arity(self, 1);
                        let payload = callback_ret.unwrap_or(Type::Any);
                        match variants.iter().find(|(v, _)| v != "None" && v != "Err") {
                            Some((succ, _)) => Type::Enum {
                                name: name.clone(),
                                args: enum_args
                                    .iter()
                                    .enumerate()
                                    .map(|(i, a)| if i == 0 { payload.clone() } else { a.clone() })
                                    .collect(),
                                variants: variants
                                    .iter()
                                    .map(|(v, p)| {
                                        if v == succ { (v.clone(), vec![payload.clone()]) } else { (v.clone(), p.clone()) }
                                    })
                                    .collect(),
                            },
                            None => Type::Any,
                        }
                    }
                    "ok_or" if is_option => {
                        arity(self, 1);
                        let t = Self::success_payload(recv).unwrap_or(Type::Any);
                        let e = args.first().cloned().unwrap_or(Type::Any);
                        self.instantiate_by_shape(&[("Ok", 1), ("Err", 1)], &[t, e])
                    }
                    "or" if is_result => {
                        arity(self, 1);
                        recv.clone()
                    }
                    "or_else" if is_result => {
                        arity(self, 1);
                        callback_ret.unwrap_or(Type::Any)
                    }
                    _ => {
                        if is_option_like || !matches!(method, "eq" | "compare" | "debug" | "hash") {
                            self.errors.push((format!("no method `.{method}()` on `{name}`"), span));
                        }
                        Type::Any
                    }
                }
            }
            Type::String => {
                let want_str = |this: &mut Self, i: usize| {
                    if let Some(got) = args.get(i) {
                        if !got.is_assignable_to(&Type::String) {
                            this.errors.push((
                                format!("`.{method}()` expects a 'String' argument, found '{got:?}'"),
                                span,
                            ));
                        }
                        this.note_arg_check(i, got, &Type::String);
                    }
                };
                match method {
                    "len" => {
                        arity(self, 0);
                        Type::Int
                    }
                    "eq" => {
                        arity(self, 1);
                        want_str(self, 0);
                        Type::Bool
                    }
                    "compare" => {
                        arity(self, 1);
                        want_str(self, 0);
                        Type::Int
                    }
                    "debug" => {
                        arity(self, 0);
                        Type::String
                    }
                    "is_empty" => {
                        arity(self, 0);
                        Type::Bool
                    }
                    "concat" => {
                        arity(self, 1);
                        want_str(self, 0);
                        Type::String
                    }
                    "replace" => {
                        arity(self, 2);
                        want_str(self, 0);
                        want_str(self, 1);
                        Type::String
                    }
                    "slice" => {
                        arity(self, 2);
                        Type::String
                    }
                    "to_upper" | "to_lower" | "trim" => {
                        arity(self, 0);
                        Type::String
                    }
                    "repeat" => {
                        arity(self, 1);
                        Type::String
                    }
                    "contains" | "starts_with" | "ends_with" => {
                        arity(self, 1);
                        want_str(self, 0);
                        Type::Bool
                    }
                    "split" => {
                        arity(self, 1);
                        want_str(self, 0);
                        Type::List(Box::new(Type::String))
                    }
                    "bytes" => {
                        arity(self, 0);
                        Type::Bytes
                    }
                    "find" | "rfind" => {
                        arity(self, 1);
                        want_str(self, 0);
                        self.option_of(Type::Int)
                    }
                    _ => {
                        self.errors.push((format!("no method `.{method}()` on `String`"), span));
                        Type::Any
                    }
                }
            }
            Type::List(elem) => {
                let elem = (**elem).clone();
                let want = |this: &mut Self, i: usize, ty: &Type, what: &str| {
                    if let Some(got) = args.get(i) {
                        if !got.is_assignable_to(ty) {
                            this.errors.push((
                                format!("`.{method}()` {what} expects '{ty:?}', found '{got:?}'"),
                                span,
                            ));
                        }
                        this.note_arg_check(i, got, ty);
                    }
                };
                match method {
                    "len" => {
                        arity(self, 0);
                        Type::Int
                    }
                    "is_empty" => {
                        arity(self, 0);
                        Type::Bool
                    }
                    "push" => {
                        arity(self, 1);
                        want(self, 0, &elem, "argument");
                        Type::Null
                    }
                    "get" => {
                        arity(self, 1);
                        want(self, 0, &Type::Int, "index");
                        elem
                    }
                    "get_or" => {
                        arity(self, 2);
                        want(self, 0, &Type::Int, "index");
                        elem
                    }
                    "set" => {
                        arity(self, 2);
                        want(self, 0, &Type::Int, "index");
                        want(self, 1, &elem, "value");
                        Type::Null
                    }
                    "contains" => {
                        arity(self, 1);
                        Type::Bool
                    }
                    "pop" | "first" | "last" => {
                        arity(self, 0);
                        self.option_of(elem.clone())
                    }
                    "clear" => {
                        arity(self, 0);
                        Type::Null
                    }
                    "iter" => {
                        arity(self, 0);
                        Type::Any
                    }
                    "stream" => {
                        arity(self, 0);
                        Type::Stream(Box::new(elem))
                    }
                    _ => {
                        self.errors
                            .push((format!("no method `.{method}()` on `List`"), span));
                        Type::Any
                    }
                }
            }
            Type::Map(key, val) => {
                let key = (**key).clone();
                let val = (**val).clone();
                let want = |this: &mut Self, i: usize, ty: &Type, what: &str| {
                    if let Some(got) = args.get(i) {
                        if !got.is_assignable_to(ty) {
                            this.errors.push((
                                format!("`.{method}()` {what} expects '{ty:?}', found '{got:?}'"),
                                span,
                            ));
                        }
                        this.note_arg_check(i, got, ty);
                    }
                };
                match method {
                    "len" => { arity(self, 0); Type::Int }
                    "is_empty" => { arity(self, 0); Type::Bool }
                    "get" => { arity(self, 1); want(self, 0, &key, "key"); val }
                    "get_or" => { arity(self, 2); want(self, 0, &key, "key"); val }
                    "set" => {
                        arity(self, 2);
                        want(self, 0, &key, "key");
                        want(self, 1, &val, "value");
                        Type::Null
                    }
                    "contains_key" => { arity(self, 1); want(self, 0, &key, "key"); Type::Bool }
                    "remove" => { arity(self, 1); want(self, 0, &key, "key"); Type::Bool }
                    "clear" => { arity(self, 0); Type::Null }
                    "keys" => { arity(self, 0); Type::List(Box::new(key)) }
                    "values" => { arity(self, 0); Type::List(Box::new(val)) }
                    "entries" => { arity(self, 0); Type::List(Box::new(Type::Any)) }
                    _ => {
                        self.errors
                            .push((format!("no method `.{method}()` on `Map`"), span));
                        Type::Any
                    }
                }
            }
            Type::Set(elem) => {
                let elem = (**elem).clone();
                let want = |this: &mut Self, i: usize, ty: &Type| {
                    if let Some(got) = args.get(i) {
                        if !got.is_assignable_to(ty) {
                            this.errors.push((
                                format!("`.{method}()` expects '{ty:?}', found '{got:?}'"),
                                span,
                            ));
                        }
                        this.note_arg_check(i, got, ty);
                    }
                };
                match method {
                    "len" => { arity(self, 0); Type::Int }
                    "is_empty" => { arity(self, 0); Type::Bool }
                    "add" => { arity(self, 1); want(self, 0, &elem); Type::Bool }
                    "contains" => { arity(self, 1); want(self, 0, &elem); Type::Bool }
                    "remove" => { arity(self, 1); want(self, 0, &elem); Type::Bool }
                    "clear" => { arity(self, 0); Type::Null }
                    "items" => { arity(self, 0); Type::List(Box::new(elem)) }
                    _ => {
                        self.errors
                            .push((format!("no method `.{method}()` on `Set`"), span));
                        Type::Any
                    }
                }
            }
            Type::Bytes => match method {
                "len" => { arity(self, 0); Type::Int }
                "is_empty" => { arity(self, 0); Type::Bool }
                "get" => { arity(self, 1); Type::Int }
                "get_or" => { arity(self, 2); Type::Int }
                "set" => { arity(self, 2); Type::Null }
                "push" => { arity(self, 1); Type::Null }
                "contains" => { arity(self, 1); Type::Bool }
                "clear" => { arity(self, 0); Type::Null }
                "extend" => {
                    arity(self, 1);
                    if let Some(a) = args.first() {
                        if !a.is_assignable_to(&Type::Bytes) {
                            self.errors.push((
                                format!("`.extend()` expects 'Bytes', found '{a:?}'"),
                                span,
                            ));
                        }
                        self.note_arg_check(0, a, &Type::Bytes);
                    }
                    Type::Null
                }
                "slice" => { arity(self, 2); Type::Bytes }
                "decode" => { arity(self, 0); self.option_of(Type::String) }
                _ => {
                    self.errors
                        .push((format!("no method `.{method}()` on `Bytes`"), span));
                    Type::Any
                }
            },
            Type::Task(t) => match method {
                "join" => { arity(self, 0); self.result_of((**t).clone(), Type::String) }
                "cancel" => { arity(self, 0); Type::Null }
                "is_ready" => { arity(self, 0); Type::Bool }
                _ => {
                    self.errors
                        .push((format!("no method `.{method}()` on `Task`"), span));
                    Type::Any
                }
            },
            Type::Sender(elem) => {
                let elem = (**elem).clone();
                match method {
                    "send" => {
                        arity(self, 1);
                        if let Some(got) = args.first() {
                            if !got.is_assignable_to(&elem) {
                                self.errors.push((format!("`.send()` expects '{elem:?}', found '{got:?}'"), span));
                            }
                            self.note_arg_check(0, got, &elem);
                        }
                        Type::Null
                    }
                    "close" => { arity(self, 0); Type::Null }
                    _ => {
                        self.errors.push((format!("no method `.{method}()` on `Sender`"), span));
                        Type::Any
                    }
                }
            }
            Type::Receiver(elem) => {
                let elem = (**elem).clone();
                match method {
                    "recv" => { arity(self, 0); self.option_of(elem) }
                    "iter" => { arity(self, 0); Type::Any }
                    "stream" => { arity(self, 0); Type::Stream(Box::new(elem)) }
                    "close" => {
                        self.errors.push(("a `Receiver` can't close a channel; close its `Sender`s".into(), span));
                        Type::Null
                    }
                    _ => {
                        self.errors.push((format!("no method `.{method}()` on `Receiver`"), span));
                        Type::Any
                    }
                }
            }
            Type::Stream(elem) => {
                let elem = (**elem).clone();
                match method {
                    "next" => { arity(self, 0); self.option_of(elem.clone()) }
                    "iter" => { arity(self, 0); Type::Stream(Box::new(elem)) }
                    _ => {
                        self.errors
                            .push((format!("no method `.{method}()` on `Stream`"), span));
                        Type::Any
                    }
                }
            }
            Type::Class { name, args: targs, .. } | Type::Struct { name, args: targs, .. } => {
                self.check_class_method_call(&name.clone(), &targs.clone(), method, args, false, span)
            }
            Type::Int | Type::Float | Type::Char => match method {
                "eq" => {
                    arity(self, 1);
                    want_self(self, 0);
                    Type::Bool
                }
                "compare" => {
                    arity(self, 1);
                    want_self(self, 0);
                    Type::Int
                }
                "debug" => {
                    arity(self, 0);
                    Type::String
                }
                _ => {
                    self.errors.push((format!("no method `.{method}()` on `{}`", TypeChecker::describe(recv)), span));
                    Type::Any
                }
            },
            Type::Bool => match method {
                "eq" => {
                    arity(self, 1);
                    want_self(self, 0);
                    Type::Bool
                }
                "debug" => {
                    arity(self, 0);
                    Type::String
                }
                _ => {
                    self.errors.push((format!("no method `.{method}()` on `Bool`"), span));
                    Type::Any
                }
            },
            _ => Type::Any,
        }
    }

    pub(crate) fn check_class_method_call(
        &mut self,
        class_name: &str,
        recv_args: &[Type],
        method: &str,
        args: &[Type],
        is_static: bool,
        span: Span,
    ) -> Type {
        let Some(sig) = self
            .class_method_sigs
            .get(&(class_name.to_string(), method.to_string()))
            .cloned()
        else {
            self.errors.push((format!("no method `.{method}()` on `{class_name}`"), span));
            return Type::Any;
        };
        self.check_method_visible(class_name, method, sig.is_pub, span);
        let class_params = self.generics.get(class_name).cloned().unwrap_or_default();
        let preset: Vec<(String, Type)> = class_params.iter().cloned().zip(recv_args.iter().cloned()).collect();
        let names: Vec<String> = class_params.iter().chain(&sig.type_params).cloned().collect();
        let (params, ret, bind) = Self::instantiate(&names, &preset, &sig.params, &sig.ret, args);
        self.check_bounds(&sig.bounds, &bind, span);
        let call = CallTypes { names: if sig.is_static { names.clone() } else { sig.type_params.clone() }, all: names.clone(), ret: sig.ret.clone(), bind: bind.clone(), variadic: sig.variadic.clone() };
        self.note_call(span, call);
        if !sig.bounds.is_empty() {
            self.errors.push((format!("`{class_name}.{method}()` has a trait bound, which is not supported on a method yet"), span));
        }
        if sig.is_static != is_static {
            self.errors.push((
                if is_static {
                    format!("`{class_name}.{method}()` is an instance method — call it on a value, not on the class")
                } else {
                    format!("`{class_name}.{method}()` is a static method — call it as `{class_name}.{method}(...)`, not on a value")
                },
                span,
            ));
        }
        let arity_bad = match &sig.variadic {
            Some(_) if args.len() < sig.required => {
                Some(format!("`.{method}()` takes at least {} argument(s), but {} were given", sig.required, args.len()))
            }
            Some(_) => None,
            None => Self::arity_error(&format!("`.{method}()`"), sig.required, sig.params.len(), args.len()),
        };
        if let Some(msg) = arity_bad {
            self.errors.push((msg, span));
        } else {
            for (i, (got, want)) in args.iter().zip(&params).enumerate() {
                if !got.is_assignable_to(want) {
                    self.errors.push((
                        format!("`.{method}()` argument {} expects '{:?}', found '{:?}'", i + 1, want, got),
                        span,
                    ));
                }
                self.note_arg_check(i, got, want);
            }
            if let Some(elem) = &sig.variadic {
                for (i, got) in args.iter().enumerate().skip(params.len()) {
                    if !got.is_assignable_to(elem) {
                        self.errors.push((
                            format!("`.{method}()` argument {} expects '{:?}', found '{:?}'", i + 1, elem, got),
                            span,
                        ));
                    }
                    self.note_arg_check(i, got, elem);
                }
            }
        }
        ret
    }
}
