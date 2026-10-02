//! Expression type inference.

use super::*;

impl TypeChecker {
    pub(crate) fn infer_expr(&mut self, expr: &Expr) -> Type {
        let expected = self.expected.take();
        match expr {
            Expr::Int(_, _) => Type::Int,
            Expr::Float(_, _) => Type::Float,
            Expr::Bool(_, _) => Type::Bool,
            Expr::String(_, _) => Type::String,
            Expr::Char(_, _) => Type::Char,
            Expr::Null(_) => Type::Null,
            Expr::SelfValue(_) => self.current_self_type.clone().unwrap_or(Type::Any),
            Expr::Ident(name, span) => {
                if let Some((info, depth)) = self.lookup_symbol_depth(name) {
                    match info.is_moved {
                        Some("spawn") => self.errors.push((format!("cannot use `{name}` — it moved into a `spawn`; clone it first"), *span)),
                        Some(by) => self.errors.push((
                            format!("cannot use `{name}` — it was consumed by `{by}` and the binding is no longer valid"),
                            *span,
                        )),
                        None => {}
                    }
                    if self.spawn_boundary.is_some_and(|b| depth < b) && info.ty.holds_end() {
                        self.spawn_ends.push((name.clone(), depth, *span));
                    }
                    if let Some(boundary) = self.spawn_boundary
                        && depth < boundary
                        && !self.is_sendable(&info.ty)
                    {
                        let msg = if matches!(info.ty, Type::Function { .. }) {
                            format!(
                                "cannot capture `{name}` (type `{:?}`) in `spawn` — a function is Sendable only when it \
                                 is a named function or a lambda that captures no `var` and nothing un-Sendable",
                                info.ty
                            )
                        } else {
                            format!(
                                "cannot capture `{name}` (type `{:?}`) in `spawn` — it is not Sendable across \
                                 a task boundary; share it with `Shared({name})` (copy) or `{name}.into_shared()` (move)",
                                info.ty
                            )
                        };
                        self.errors.push((msg, *span));
                    }
                    let unsendable = (info.is_mutable && depth > 0) || !self.is_sendable(&info.ty);
                    for frame in self.lambda_frames.iter_mut().filter(|f| depth < f.0) {
                        frame.1 &= !unsendable;
                    }
                    if info.is_mutable {
                        return info.ty.clone();
                    }
                    self.narrowed(name, depth, *span).unwrap_or_else(|| info.ty.clone())
                } else if let Some(sig) = self.fn_sigs.get(name).cloned() {
                    if sig.modes.iter().any(|(_, v)| *v) {
                        self.errors.push((format!("`{name}` has a `var` parameter, so it can only be called directly"), *span));
                        Type::Any
                    } else if sig.variadic.is_some() || sig.required != sig.params.len() {
                        Type::Any
                    } else if sig.type_params.is_empty() {
                        Type::Function { params: sig.params.clone(), ret: Box::new(sig.ret.clone()), sendable: true, modes: Vec::new() }
                    } else {
                        let mut bind = HashMap::new();
                        if let Some(Type::Function { params, ret, .. }) = &expected {
                            for (p, want) in sig.params.iter().zip(params) {
                                Self::unify(p, want, &sig.type_params, &mut bind);
                            }
                            Self::unify(&sig.ret, ret, &sig.type_params, &mut bind);
                        }
                        let preset: Vec<(String, Type)> = bind.into_iter().collect();
                        let (params, ret, bind) = Self::instantiate(&sig.type_params, &preset, &sig.params, &sig.ret, &[]);
                        let names = sig.type_params.clone();
                        let declared = Type::Function { params: sig.params.clone(), ret: Box::new(sig.ret.clone()), sendable: true, modes: Vec::new() };
                        self.note_call(*span, CallTypes { all: names.clone(), names, ret: declared, bind, variadic: None });
                        Type::Function { params, ret: Box::new(ret), sendable: true, modes: Vec::new() }
                    }
                } else if let Some(ctor) = self.option_ctor(expr) {
                    if ctor == "Some" {
                        self.errors.push(("`Some` takes 1 value — write `Some(…)`".into(), *span));
                    }
                    Type::Null
                } else if let Some((enum_name, payload)) = self.resolve_variant(None, name, *span) {
                    if !payload.is_empty() {
                        self.errors.push((
                            format!("Variant '{}' takes {} value(s) — write '{}(…)'", name, payload.len(), name),
                            *span,
                        ));
                    }
                    self.instantiate_type(&enum_name, &[])
                } else {
                    self.errors.push((format!("Undefined identifier '{}'", name), *span));
                    Type::Any
                }
            }
            Expr::Binary { left, op, right, span } => {
                let lt = self.check_expr(left);
                let rt = match op {
                    BinaryOp::And => self.check_assuming(self.facts(left).when_true, right),
                    BinaryOp::Or => self.check_assuming(self.facts(left).when_false, right),
                    _ => self.check_expr(right),
                };
                let ordered = !matches!(op, BinaryOp::Eq | BinaryOp::NotEq | BinaryOp::And | BinaryOp::Or);
                if let Some(u) = [&lt, &rt].into_iter().find(|t| matches!(t, Type::Union(_))).filter(|_| ordered) {
                    self.union_operand(op.symbol(), u, *span);
                    return if matches!(op, BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq) { Type::Bool } else { Type::Any };
                }
                if let Some(o) = [&lt, &rt].into_iter().find(|t| matches!(t, Type::Nullable(_))).filter(|_| ordered) {
                    self.optional_operand(op.symbol(), o, *span);
                    return if matches!(op, BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq) { Type::Bool } else { Type::Any };
                }
                match op {
                    BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod
                    | BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor | BinaryOp::Shl | BinaryOp::Shr => {
                        self.arithmetic_type(*op, &lt, &rt, *span)
                    }
                    BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq => {
                        let orderable = |t: &Type| matches!(t, Type::Int | Type::Float | Type::String);
                        if is_scalar(&lt) && is_scalar(&rt) && !(lt == rt && orderable(&lt)) {
                            self.errors.push((format!("cannot compare `{:?}` with `{:?}` using `{}`", lt, rt, op.symbol()), *span));
                        }
                        Type::Bool
                    }
                    BinaryOp::Eq | BinaryOp::NotEq => Type::Bool,
                    BinaryOp::And | BinaryOp::Or => {
                        if lt != Type::Bool && lt != Type::Any {
                            self.errors.push((format!("Left operand of logical op must be Bool, found '{:?}'", lt), *span));
                        }
                        if rt != Type::Bool && rt != Type::Any {
                            self.errors.push((format!("Right operand of logical op must be Bool, found '{:?}'", rt), *span));
                        }
                        Type::Bool
                    }
                }
            }
            Expr::Unary { op, expr, span } => {
                let ty = self.check_expr(expr);
                let (ok, sym) = match op {
                    UnaryOp::Neg => (matches!(ty, Type::Int | Type::Float), "-"),
                    UnaryOp::Not => (ty == Type::Bool, "!"),
                    UnaryOp::BitNot => (ty == Type::Int, "~"),
                };
                if matches!(ty, Type::Union(_)) {
                    self.union_operand(sym, &ty, *span);
                }
                if matches!(ty, Type::Nullable(_)) {
                    self.optional_operand(sym, &ty, *span);
                }
                if !ok && is_scalar(&ty) {
                    self.errors.push((format!("cannot apply `{sym}` to `{:?}`", ty), *span));
                }
                match op {
                    UnaryOp::Not => Type::Bool,
                    _ => ty,
                }
            }
            Expr::Call { callee, args, span } => {
                let is_lambda = |a: &Expr| matches!(a, Expr::Lambda { .. });
                let early = self.early_expectations(callee, expected.as_ref());
                let mut arg_tys: Vec<Type> = args
                    .iter()
                    .enumerate()
                    .map(|(i, a)| if is_lambda(a) { Type::Null } else { self.check_expr_expecting(a, early.get(i).cloned().flatten()) })
                    .collect();
                let mut recv_pre = None;
                let open = |a: &Expr, t: &Type| Self::takes_context(a) || t.has_hole();
                if args.iter().zip(&arg_tys).any(|(a, t)| is_lambda(a) || open(a, t)) {
                    let expected = self.call_expectations(callee, &arg_tys, expected.as_ref(), &mut recv_pre);
                    for (i, a) in args.iter().enumerate() {
                        let exp = expected.get(i).cloned().flatten();
                        if is_lambda(a) {
                            arg_tys[i] = self.check_expr_expecting(a, exp);
                        } else if let Some(exp) = exp.filter(|_| open(a, &arg_tys[i])) {
                            arg_tys[i] = self.settle(a, &exp, arg_tys[i].clone());
                        }
                    }
                    self.implicit_var_lambda = false;
                }
                let arg_spans: Vec<Span> = args.iter().map(Expr::span).collect();
                self.call_arg_spans = arg_spans.clone();
                match self.option_ctor(callee) {
                    Some("Some") => return self.check_some(&arg_tys, expected.as_ref(), *span),
                    Some(_) => {
                        self.errors.push(("`None` takes no values; write `None`".into(), *span));
                        return Type::Null;
                    }
                    None => {}
                }
                if let Some(ty) = self.check_static_call(callee, &arg_tys, *span) {
                    return ty;
                }
                let named = match (callee.as_ref(), self.written_fn(callee)) {
                    (_, Some((name, written))) => Some((name, written)),
                    (Expr::Ident(name, _), None) => Some((name, &[][..])),
                    _ => None,
                };
                if let Some((name, written)) = named {
                    if self.lookup_symbol(name).is_some()
                        && let Type::Function { params, ret, .. } = self.check_expr(callee) {
                            return self.check_value_call(name, &params, &ret, &arg_tys, *span);
                        }
                    if let Some(sig) = self.fn_sigs.get(name).cloned() {
                        let mut preset = HashMap::new();
                        if let Some(exp) = &expected {
                            Self::unify(&sig.ret, exp, &sig.type_params, &mut preset);
                        }
                        self.preset_written(name, &sig.type_params, written, *span, &mut preset);
                        let preset: Vec<(String, Type)> = preset.into_iter().collect();
                        let (params, ret, bind) = Self::instantiate(&sig.type_params, &preset, &sig.params, &sig.ret, &arg_tys);
                        self.check_bounds(&sig.bounds, &bind, *span);
                        let mut ret = ret;
                        if crate::foreign::is_bind(name) {
                            if let Err(message) = crate::foreign::signature_of(bind.get("F")) {
                                self.errors.push((message, *span));
                            }
                            crate::foreign::make_sendable(&mut ret);
                        }
                        let names = sig.type_params.clone();
                        let call = CallTypes { all: names.clone(), names, ret: sig.ret.clone(), bind: bind.clone(), variadic: sig.variadic.clone() };
                        self.note_call(*span, call);
                        if !sig.bounds.is_empty() {
                            self.request_instance(name, &sig.type_params, &bind, *span);
                        }
                        let arity_bad = match &sig.variadic {
                            Some(_) if arg_tys.len() < sig.required => {
                                Some(format!("`{name}` takes at least {} argument(s), but {} were given", sig.required, arg_tys.len()))
                            }
                            Some(_) => None,
                            None => Self::arity_error(&format!("`{name}`"), sig.required, sig.params.len(), arg_tys.len()),
                        };
                        if let Some(msg) = arity_bad {
                            self.errors.push((msg, *span));
                        } else {
                            for (i, (got, want)) in arg_tys.iter().zip(&params).enumerate() {
                                if !got.is_assignable_to(want) {
                                    self.errors.push((
                                        format!("`{}` argument {} expects '{:?}', found '{:?}'", name, i + 1, want, got),
                                        *span,
                                    ));
                                }
                                self.note_arg_check(i, got, want);
                            }
                            if let Some(elem) = &sig.variadic {
                                for (i, got) in arg_tys.iter().enumerate().skip(params.len()) {
                                    if !got.is_assignable_to(elem) {
                                        self.errors.push((
                                            format!("`{}` argument {} expects '{:?}', found '{:?}'", name, i + 1, elem, got),
                                            *span,
                                        ));
                                    }
                                    self.note_arg_check(i, got, elem);
                                }
                            }
                        }
                        return ret;
                    }
                    if self.enum_variants.contains_key(name) {
                        if let Some((enum_name, payload)) = self.resolve_variant(None, name, *span) {
                            return self.check_variant_payload(&enum_name, name, &payload, &arg_tys, expected.as_ref(), *span);
                        }
                        return Type::Any;
                    }
                }
                if let Some((enum_name, m_span, member)) = self.qualified_variant_ref(callee)
                    && let Some((_, payload)) = self.resolve_variant(Some(&enum_name), &member, m_span) {
                        return self.check_variant_payload(&enum_name, &member, &payload, &arg_tys, expected.as_ref(), *span);
                    }
                if let Some(ty) = self.builtin_from(callee, &arg_tys, *span) {
                    return ty;
                }
                if let Expr::MemberAccess { object, member, span: m_span } = callee.as_ref() {
                    let recv = match recv_pre.take() {
                        Some(t) => t,
                        None => self.check_expr(object),
                    };
                    self.call_arg_spans = arg_spans;
                    if matches!(member.as_str(), "shared" | "into_shared") {
                        return self.check_share(object, &recv, member, &arg_tys, *m_span);
                    }
                    if member == "clone"
                        && let Some(t) = self.check_clone(&recv, &arg_tys, *m_span) {
                            return t;
                        }
                    return self.check_builtin_method(&recv, member, &arg_tys, *m_span);
                }
                if let Expr::Ident(name, _) = callee.as_ref() {
                    if self.lookup_symbol(name).is_some() || self.imported_names.contains(name) {
                        return Type::Any;
                    }
                    let msg = match name.as_str() {
                        "Mutex" | "Atomic" => super::shared::unknown_type(name),
                        _ => format!("no function `{name}`"),
                    };
                    self.errors.push((msg, *span));
                    return Type::Any;
                }
                match self.check_expr(callee) {
                    Type::Function { params, ret, .. } => self.check_value_call("this function", &params, &ret, &arg_tys, *span),
                    _ => Type::Any,
                }
            }
            Expr::StaticAccess { span, .. } | Expr::MemberAccess { span, .. } if self.option_ctor(expr).is_some() => {
                if self.option_ctor(expr) == Some("Some") {
                    self.errors.push(("`Option.Some` takes 1 value — write `Option.Some(…)`".into(), *span));
                }
                Type::Null
            }
            Expr::StaticAccess { target: TypeNode::Generic(name, _, _), member, span } => {
                self.errors.push((format!("`{name}.{member}` is a method; call it"), *span));
                Type::Any
            }
            Expr::MemberAccess { object, member, span } if matches!(object.as_ref(), Expr::Ident(n, _) if self.is_type_value(n)) => {
                let Expr::Ident(name, _) = object.as_ref() else { unreachable!() };
                self.errors.push((format!("`{name}.{member}` is a method; call it"), *span));
                Type::Any
            }
            Expr::StaticAccess { span, .. } => {
                if let Some((enum_name, m_span, member)) = self.qualified_variant_ref(expr) {
                    if let Some((_, payload)) = self.resolve_variant(Some(&enum_name), &member, m_span) {
                        if !payload.is_empty() {
                            self.errors.push((
                                format!("Variant '{}.{}' takes {} value(s)", enum_name, member, payload.len()),
                                *span,
                            ));
                        }
                        return self.instantiate_type(&enum_name, &[]);
                    }
                    self.errors.push((format!("Enum '{}' has no variant '{}'", enum_name, member), *span));
                }
                Type::Any
            }
            Expr::MemberAccess { object, member, span } => {
                if let Some((enum_name, m_span, m)) = self.qualified_variant_ref(expr) {
                    if let Some((_, payload)) = self.resolve_variant(Some(&enum_name), &m, m_span) {
                        if !payload.is_empty() {
                            self.errors.push((
                                format!("Variant '{}.{}' takes {} value(s)", enum_name, m, payload.len()),
                                *span,
                            ));
                        }
                        return self.instantiate_type(&enum_name, &[]);
                    }
                    self.errors.push((format!("Enum '{}' has no variant '{}'", enum_name, member), *span));
                    return Type::Any;
                }
                let obj_ty = self.check_expr(object);
                if matches!(obj_ty, Type::Shared(_)) {
                    self.errors.push((format!("a `Shared` has no field `{member}`; read it from a snapshot: `.get().{member}`"), *span));
                    return Type::Any;
                }
                match &obj_ty {
                    Type::Struct { name, fields, .. } | Type::Class { name, fields, .. } => {
                        if let Some((_, f_ty)) = fields.iter().find(|(n, _)| n == member) {
                            self.check_field_read(name, member, *span);
                            f_ty.clone()
                        } else if let Some(sig) = self.class_method_sigs.get(&(name.clone(), member.clone())) {
                            let params: Vec<String> = (0..sig.params.len()).map(|i| format!("a{}", i + 1)).collect();
                            let lambda = format!("|{}| {object}.{member}({})", params.join(", "), params.join(", "), object = Self::describe_receiver(object));
                            self.errors.push((
                                format!("`{member}` is a method, not a value: write `{lambda}` to pass it around"),
                                *span,
                            ));
                            Type::Any
                        } else {
                            self.errors.push((format!("Field '{}' not found on type '{:?}'", member, obj_ty), *span));
                            Type::Any
                        }
                    }
                    Type::Tuple(elems) => match member.parse::<usize>() {
                        Ok(i) if i < elems.len() => elems[i].clone(),
                        Ok(i) => {
                            self.errors.push((
                                format!("tuple has {} element(s); no field '.{}'", elems.len(), i),
                                *span,
                            ));
                            Type::Any
                        }
                        Err(_) => {
                            self.errors.push((
                                format!("a tuple has no field '{}' — use '.0', '.1', …", member),
                                *span,
                            ));
                            Type::Any
                        }
                    },
                    t if is_scalar(t) => {
                        self.errors.push((format!("`{:?}` has no field '{}'", t, member), *span));
                        Type::Any
                    }
                    Type::Param(p) => {
                        self.errors.push((format!("`{p}` is a type parameter with no bound, so it has no field '{member}'"), *span));
                        Type::Any
                    }
                    Type::Nullable(_) => {
                        self.optional_access(&obj_ty, member, *span);
                        Type::Any
                    }
                    Type::Union(_) => self.union_member(&obj_ty, member, *span, |this, m| this.member_field(m, member, *span)),
                    _ => Type::Any,
                }
            }
            Expr::Index { object, index, span } => {
                let obj_ty = self.check_expr(object);
                let idx_ty = self.check_expr(index);
                self.call_arg_spans = vec![index.span()];
                self.check_index(&obj_ty, idx_ty, None, *span)
            }
            Expr::ListLiteral { elements, span } => self.list_literal_type(elements, *span),
            Expr::MapLiteral { entries, span } => self.map_literal_type(entries, *span),
            Expr::Ternary { cond, then_expr, else_expr, span } => self.ternary_type(cond, then_expr, else_expr, *span),
            Expr::NullCoalesce { left, right, .. } => self.coalesce_type(left, right),
            Expr::OptionalChain { object, temp, body, .. } => self.optional_chain_type(object, temp, body),
            Expr::TypeTest { expr, target_type, span } => {
                self.check_expr(expr);
                let target = self.resolve_type_node(target_type);
                self.type_tests.insert(*span, target);
                Type::Bool
            }
            Expr::TupleLiteral { elements, .. } => {
                Type::Tuple(elements.iter().map(|e| self.check_expr(e)).collect())
            }
            Expr::StructInit { name, fields, span, .. } => {
                if let Some(key) = self.named_field_variant(name) {
                    return self.check_variant_literal(key, fields, *span);
                }
                if let Some(template) = self.types.get(name).cloned()
                    && let Type::Struct { fields: decl_fields, .. } | Type::Class { fields: decl_fields, .. } = &template {
                        self.check_literal_allowed(name, *span);
                        let vals: Vec<(&String, Type)> = fields.iter().map(|(n, v)| (n, self.check_expr(v))).collect();
                        let declared: Vec<String> = decl_fields.iter().map(|(n, _)| n.clone()).collect();
                        self.check_literal_field_names(name, "literal", fields.iter().map(|(n, _)| n.as_str()), &declared, *span);
                        let names = self.generics.get(name).cloned().unwrap_or_default();
                        let mut bind = HashMap::new();
                        for (f_name, val_ty) in &vals {
                            if let Some((_, decl)) = decl_fields.iter().find(|(n, _)| n == *f_name) {
                                Self::unify(decl, val_ty, &names, &mut bind);
                            }
                        }
                        let ty = Self::subst(&template, &names, &bind);
                        let (Type::Struct { fields: inst_fields, .. } | Type::Class { fields: inst_fields, .. }) = &ty else { return ty };
                        for ((f_name, val_ty), (_, val_expr)) in vals.iter().zip(fields) {
                            if let Some((_, exp_ty)) = inst_fields.iter().find(|(n, _)| n == *f_name) {
                                let val_ty = self.settle(val_expr, exp_ty, val_ty.clone());
                                if !val_ty.is_assignable_to(exp_ty) {
                                    self.errors.push((format!("Type mismatch on '{}' field '{}': expected '{:?}', found '{:?}'", name, f_name, exp_ty, val_ty), *span));
                                }
                                self.note_coercion(val_expr.span(), &val_ty, exp_ty);
                            }
                        }
                        return ty;
                    }
                let msg = match self.type_aliases.get(name).map(|a| &a.target) {
                    Some(TypeNode::Named(target, _)) => {
                        format!("`{name}` is a type alias; a literal needs the type's own name `{target}`")
                    }
                    _ => format!("Unknown type '{}'", name),
                };
                self.errors.push((msg, *span));
                Type::Any
            }
            Expr::Try { expr, span } => {
                let inner = self.check_expr(expr);
                if Self::failure_variant(&inner).is_none() && inner != Type::Any {
                    self.errors.push((
                        format!("`?` needs an optional or a `Result`, found '{:?}'", inner),
                        *span,
                    ));
                }
                match &self.current_fn_ret {
                    Some(ret) if ret != &Type::Any && Self::failure_variant(ret).is_none() => {
                        self.errors.push((
                            "`?` needs a function that returns an optional or a `Result`".into(),
                            *span,
                        ));
                    }
                    Some(ret) => match (Self::failure_variant(&inner), Self::failure_variant(ret)) {
                        (Some("None"), Some("Err")) => self.errors.push((
                            "`?` on an optional returns `None`, but this function returns a `Result`; use `.ok_or(e)?`".into(),
                            *span,
                        )),
                        (Some("Err"), Some("None")) => self.errors.push((
                            "`?` on a `Result` returns its `Err`, but this function returns an optional".into(),
                            *span,
                        )),
                        _ => {}
                    },
                    None => {}
                }
                Self::success_payload(&inner).unwrap_or(Type::Any)
            }
            Expr::Unwrap { expr, span } => {
                let inner = self.check_expr(expr);
                if Self::failure_variant(&inner).is_none() && inner != Type::Any {
                    self.errors.push((format!("`!` needs an optional or a `Result`, found '{:?}'", inner), *span));
                }
                Self::success_payload(&inner).unwrap_or(Type::Any)
            }
            Expr::Lambda { params, return_type, body, .. } => {
                let (expected, expected_ret): (Vec<Type>, Option<Type>) = match self.lambda_expect.take() {
                    Some(Type::Function { params: ps, ret, .. }) if ps.len() == params.len() => (ps, Some(*ret)),
                    _ => (Vec::new(), None),
                };
                let implicit_var = std::mem::take(&mut self.implicit_var_lambda);
                let declared_ret = return_type.as_ref().map(|t| self.resolve_type_node(t));
                let prev_ret = std::mem::replace(&mut self.current_fn_ret, declared_ret.clone());
                let prev_returns = self.lambda_returns.replace(Vec::new());
                let prev_gen = self.gen_elem.take();
                let prev_in_fn = std::mem::replace(&mut self.in_function_body, true);
                self.lambda_frames.push((self.scopes.len(), true));
                self.push_scope();
                let mut param_tys = Vec::new();
                for (i, p) in params.iter().enumerate() {
                    let ty = match &p.ty {
                        Some(t) => self.resolve_type_node(t),
                        None => expected.get(i).cloned().unwrap_or(Type::Any),
                    };
                    let is_var = p.is_mut || implicit_var;
                    self.insert_symbol(&p.name, ty.clone(), is_var);
                    if !is_var {
                        self.set_read_only(&p.name, modes::ReadOnly::Derived);
                    }
                    param_tys.push(ty);
                }
                let (lead, tail) = match body.split_last() {
                    Some((Stmt::Expr { expr, .. }, lead)) => (lead, Some(expr)),
                    _ => (&body[..], None),
                };
                for stmt in lead {
                    self.check_stmt(stmt);
                }
                let want_tail = declared_ret.clone().or(expected_ret.clone()).filter(|t| !matches!(t, Type::Null | Type::Any));
                let mut tail_ty = tail.map(|e| self.check_expr_expecting(e, want_tail.clone()));
                if let (Some(e), Some(Type::Any), None, Some(want)) = (tail, &tail_ty, &declared_ret, &expected_ret)
                    && !want.has_hole() && !matches!(want, Type::Any | Type::Null) {
                        self.note_coercion(e.span(), &Type::Any, want);
                        tail_ty = Some(want.clone());
                    }
                self.pop_scope();
                let (_, sendable) = self.lambda_frames.pop().expect("pushed above");
                let returns = std::mem::replace(&mut self.lambda_returns, prev_returns).unwrap_or_default();
                self.current_fn_ret = prev_ret;
                self.gen_elem = prev_gen;
                self.in_function_body = prev_in_fn;
                let ret = declared_ret.or(tail_ty).unwrap_or_else(|| Self::common_return(&returns));
                Type::Function { params: param_tys, ret: Box::new(ret), sendable, modes: modes::lambda_modes(params) }
            }
            Expr::Spawn { body, .. } => {
                let ret = self.check_spawn_body(body);
                Type::Task(Box::new(ret))
            }
            _ => Type::Any,
        }
    }

}

pub(super) fn is_scalar(t: &Type) -> bool {
    matches!(t, Type::Int | Type::Float | Type::String | Type::Bool)
}

impl TypeChecker {
    pub(super) fn arithmetic_type(&mut self, op: BinaryOp, lt: &Type, rt: &Type, span: Span) -> Type {
        let arithmetic = matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod);
        if *lt == Type::Int && *rt == Type::Int {
            Type::Int
        } else if arithmetic && *lt == Type::Float && *rt == Type::Float {
            Type::Float
        } else if *lt == Type::String && *rt == Type::String && op == BinaryOp::Add {
            Type::String
        } else {
            if is_scalar(lt) && is_scalar(rt) {
                self.errors.push((format!("cannot apply `{}` to `{:?}` and `{:?}`", op.symbol(), lt, rt), span));
            }
            Type::Any
        }
    }

    pub(super) fn note_arg_check(&mut self, i: usize, got: &Type, want: &Type) {
        if let Some(&span) = self.call_arg_spans.get(i) {
            self.note_coercion(span, got, want);
        }
    }

    pub(super) fn note_coercion(&mut self, span: Span, got: &Type, want: &Type) {
        if *got == Type::Any && !matches!(want, Type::Any | Type::Null | Type::Hole) {
            self.any_checks.insert(span, want.clone());
        }
        if matches!(got, Type::Param(_)) {
            let (mut lifts, mut want) = (0u8, want);
            while let Type::Nullable(inner) = want {
                if !got.is_assignable_to(inner) {
                    break;
                }
                lifts += 1;
                want = inner;
            }
            if lifts > 0 {
                self.some_lifts.insert(span, lifts);
            }
        }
    }

    fn describe_receiver(object: &Expr) -> String {
        match object {
            Expr::Ident(name, _) => name.clone(),
            _ => "obj".to_string(),
        }
    }
}

impl TypeChecker {
    pub(super) fn option_ctor(&self, e: &Expr) -> Option<&'static str> {
        let ctor = |name: &str| match name {
            "Some" => Some("Some"),
            "None" => Some("None"),
            _ => None,
        };
        match e {
            Expr::Ident(name, _) if self.lookup_symbol(name).is_none() && !self.fn_sigs.contains_key(name) => ctor(name),
            Expr::MemberAccess { object, member, .. } => match object.as_ref() {
                Expr::Ident(head, _) if head == "Option" && self.lookup_symbol(head).is_none() => ctor(member),
                _ => None,
            },
            Expr::StaticAccess { target: TypeNode::Named(head, _), member, .. } if head == "Option" => ctor(member),
            _ => None,
        }
    }

    fn check_some(&mut self, arg_tys: &[Type], expected: Option<&Type>, span: Span) -> Type {
        let [got] = arg_tys else {
            self.errors.push((format!("`Some` takes 1 value, but {} were given", arg_tys.len()), span));
            return Type::Nullable(Box::new(Type::Any));
        };
        if let Some(Type::Nullable(inner)) = expected
            && got.is_assignable_to(inner) {
                self.note_arg_check(0, got, inner);
                return Type::Nullable(inner.clone());
            }
        Type::Nullable(Box::new(got.clone()))
    }

    fn early_expectations(&self, callee: &Expr, expected: Option<&Type>) -> Vec<Option<Type>> {
        if self.option_ctor(callee) == Some("Some") {
            return match expected {
                Some(Type::Nullable(inner)) => vec![Some((**inner).clone())],
                _ => Vec::new(),
            };
        }
        let from = |names: &[String], ret: &Type, params: &[Type]| {
            let mut bind = HashMap::new();
            if let Some(exp) = expected {
                Self::unify(ret, exp, names, &mut bind);
            }
            params.iter().map(|p| Some(Self::subst(p, names, &bind)).filter(|t| !t.has_hole())).collect()
        };
        let variant = |enum_name: &str, variant: &str| -> Option<Vec<Option<Type>>> {
            let template = self.types.get(enum_name)?;
            let Type::Enum { variants, .. } = template else { return None };
            let payload = &variants.iter().find(|(v, _)| v == variant)?.1;
            Some(from(&self.generics.get(enum_name).cloned().unwrap_or_default(), template, payload))
        };
        if let Some((enum_name, _, member)) = self.qualified_variant_ref(callee) {
            return variant(&enum_name, &member).unwrap_or_default();
        }
        let Expr::Ident(name, _) = callee else { return Vec::new() };
        if self.lookup_symbol(name).is_some() {
            return Vec::new();
        }
        if let Some(sig) = self.fn_sigs.get(name) {
            return from(&sig.type_params, &sig.ret, &sig.params);
        }
        match self.enum_variants.get(name).map(Vec::as_slice) {
            Some([(enum_name, _)]) => variant(enum_name, name).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    pub(super) fn call_expectations(&mut self, callee: &Expr, arg_tys: &[Type], expected: Option<&Type>, recv_pre: &mut Option<Type>) -> Vec<Option<Type>> {
        if let Some((name, nodes, member)) = self.static_target(callee) {
            if constructors::builtin_ctor_arity(&name).is_some() {
                return Vec::new();
            }
            let targs: Vec<Type> = nodes.iter().map(|a| self.resolve_type_node(a)).collect();
            return self.method_params(&name, &targs, &member, arg_tys).into_iter().map(Some).collect();
        }
        if let Some((name, written)) = self.written_fn(callee) {
            let sig = self.fn_sigs[name].clone();
            let mut preset = HashMap::new();
            if written.len() == sig.type_params.len() {
                for (p, t) in sig.type_params.iter().zip(written) {
                    preset.insert(p.clone(), self.resolve_type_node(t));
                }
            }
            let preset: Vec<(String, Type)> = preset.into_iter().collect();
            return Self::instantiate(&sig.type_params, &preset, &sig.params, &sig.ret, arg_tys).0.into_iter().map(Some).collect();
        }
        let params = match callee {
            Expr::Ident(name, _) => match self.lookup_symbol(name) {
                Some(info) => match info.ty {
                    Type::Function { params, .. } => params,
                    _ => Vec::new(),
                },
                None => match self.fn_sigs.get(name).cloned() {
                    Some(sig) => {
                        let mut bind = HashMap::new();
                        if let Some(exp) = expected {
                            Self::unify(&sig.ret, exp, &sig.type_params, &mut bind);
                        }
                        let preset: Vec<(String, Type)> = bind.into_iter().filter(|(_, t)| !t.has_hole()).collect();
                        Self::instantiate(&sig.type_params, &preset, &sig.params, &sig.ret, arg_tys).0
                    }
                    None => Vec::new(),
                },
            },
            Expr::MemberAccess { object, member, .. } => {
                if self.qualified_variant_ref(callee).is_some() {
                    return Vec::new();
                }
                if let Expr::Ident(name, _) = object.as_ref() {
                    if let Some((_, takes)) = conversions::builtin_from_target(name).filter(|_| member == "from" && self.lookup_symbol(name).is_none()) {
                        return vec![Some(takes).filter(|t| *t != Type::Any)];
                    }
                    if self.lookup_symbol(name).is_none() && matches!(self.types.get(name), Some(Type::Class { .. } | Type::Struct { .. })) {
                        return self.method_params(name, &[], member, arg_tys).into_iter().map(Some).collect();
                    }
                }
                let recv = self.check_expr(object);
                *recv_pre = Some(recv.clone());
                self.method_expectations(&recv, member, arg_tys)
            }
            _ => Vec::new(),
        };
        params.into_iter().map(Some).collect()
    }

    fn method_expectations(&mut self, recv: &Type, method: &str, arg_tys: &[Type]) -> Vec<Type> {
        let callback = |params: Vec<Type>| vec![Type::Function { params, ret: Box::new(Type::Any), sendable: false, modes: Vec::new() }];
        match recv {
            Type::Shared(inner) if method == "update" => {
                self.implicit_var_lambda = true;
                vec![Type::Function { params: vec![(**inner).clone()], ret: Box::new(Type::Null), sendable: false, modes: vec![true] }]
            }
            Type::Shared(inner) if method == "wait_until" => callback(vec![(**inner).clone()]),
            Type::Enum { name, args, .. } if self.class_method_sigs.contains_key(&(name.clone(), method.to_string())) => self.method_params(name, args, method, arg_tys),
            Type::Enum { .. } | Type::Nullable(_) => match (method, Self::success_payload(recv)) {
                ("map" | "and_then", Some(payload)) => callback(vec![payload]),
                ("unwrap_or_else" | "or_else", _) => callback(Vec::new()),
                _ => Vec::new(),
            },
            Type::Class { name, args, .. } | Type::Struct { name, args, .. } => self.method_params(name, args, method, arg_tys),
            _ => Vec::new(),
        }
    }

    fn method_params(&self, class_name: &str, recv_args: &[Type], method: &str, arg_tys: &[Type]) -> Vec<Type> {
        let Some(sig) = self.class_method_sigs.get(&(class_name.to_string(), method.to_string())) else {
            return Vec::new();
        };
        let class_params = self.generics.get(class_name).cloned().unwrap_or_default();
        let preset: Vec<(String, Type)> = class_params.iter().cloned().zip(recv_args.iter().cloned()).collect();
        let names: Vec<String> = class_params.iter().chain(&sig.type_params).cloned().collect();
        Self::instantiate(&names, &preset, &sig.params, &sig.ret, arg_tys).0
    }

    pub(super) fn check_value_call(&mut self, name: &str, params: &[Type], ret: &Type, arg_tys: &[Type], span: Span) -> Type {
        if params.len() != arg_tys.len() {
            self.errors.push((format!("`{name}` takes {} argument(s), but {} were given", params.len(), arg_tys.len()), span));
            return ret.clone();
        }
        for (i, (got, want)) in arg_tys.iter().zip(params).enumerate() {
            if !got.is_assignable_to(want) {
                self.errors.push((format!("`{}` argument {} expects '{:?}', found '{:?}'", name, i + 1, want, got), span));
            }
            self.note_arg_check(i, got, want);
        }
        ret.clone()
    }
}
