//! Built-in method and intrinsic lowering.

use super::*;

impl CodeGenerator {
    pub(crate) fn try_lower_builtin_method(
        &mut self,
        object: &Expr,
        method: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<u8>, (String, Span)> {
        if let Some(r) = self.try_lower_shared_method(object, method, args, span)? {
            return Ok(Some(r));
        }
        match method {
            "is_some" | "is_none" if !matches!(self.fallible_kind(object), Fallible::Tagged(_)) => {
                let r = self.compile_expr(object)?;
                let dest = self.emit_none_test(r, method == "is_none");
                self.reg_alloc.free_temp(r);
                Ok(Some(dest))
            }
            "is_some" => Ok(Some(self.method_variant_test(object, "Some")?)),
            "is_none" => Ok(Some(self.method_variant_test(object, "None")?)),
            "is_ok" => Ok(Some(self.method_variant_test(object, "Ok")?)),
            "is_err" => Ok(Some(self.method_variant_test(object, "Err")?)),
            "unwrap" => {
                let kind = self.fallible_kind(object);
                let r = self.compile_expr(object)?;
                let fail = self.emit_failure_test(r, &kind, span)?;
                let site = self.current_insts.len();
                self.current_insts.push(0);
                self.emit_panic("called `.unwrap()` on an empty Option / an Err value");
                let after = self.current_insts.len() as i32;
                self.current_insts[site] =
                    encode_jc(Opcode::JMPIFNOT, fail, (after - site as i32) as i16);
                self.reg_alloc.free_temp(fail);
                let dest = self.reg_alloc.alloc_temp();
                self.emit_payload(dest, r, &kind);
                self.reg_alloc.free_temp(r);
                Ok(Some(dest))
            }
            "unwrap_or" => {
                let default_expr = args
                    .first()
                    .ok_or_else(|| ("`.unwrap_or(x)` needs one argument".to_string(), span))?;
                let kind = self.fallible_kind(object);
                let r = self.compile_expr(object)?;
                let default = self.compile_expr(default_expr)?;
                let fail = self.emit_failure_test(r, &kind, span)?;
                let dest = self.reg_alloc.alloc_temp();
                self.emit_payload(dest, r, &kind);
                let site = self.current_insts.len();
                self.current_insts.push(0);
                self.current_insts.push(encode_r2(Opcode::MOVE, dest, default));
                let after = self.current_insts.len() as i32;
                self.current_insts[site] =
                    encode_jc(Opcode::JMPIFNOT, fail, (after - site as i32) as i16);
                self.reg_alloc.free_temp(fail);
                self.reg_alloc.free_temp(default);
                self.reg_alloc.free_temp(r);
                Ok(Some(dest))
            }
            "unwrap_or_else" => {
                let f_expr = args
                    .first()
                    .ok_or_else(|| ("`.unwrap_or_else(f)` needs one argument".to_string(), span))?;
                let kind = self.fallible_kind(object);
                let r = self.compile_expr(object)?;
                let fail = self.emit_failure_test(r, &kind, span)?;
                let dest = self.reg_alloc.alloc_temp();
                self.emit_payload(dest, r, &kind);
                let site = self.current_insts.len();
                self.current_insts.push(0);
                let callee = self.compile_value(f_expr)?;
                let called = self.emit_call_value(callee, None);
                self.reg_alloc.free_temp(callee);
                self.current_insts.push(encode_r2(Opcode::MOVE, dest, called));
                self.reg_alloc.free_temp(called);
                let after = self.current_insts.len() as i32;
                self.current_insts[site] =
                    encode_jc(Opcode::JMPIFNOT, fail, (after - site as i32) as i16);
                self.reg_alloc.free_temp(fail);
                self.reg_alloc.free_temp(r);
                Ok(Some(dest))
            }
            "and_then" => {
                let f_expr = args
                    .first()
                    .ok_or_else(|| ("`.and_then(f)` needs one argument".to_string(), span))?;
                let kind = self.fallible_kind(object);
                let r = self.compile_expr(object)?;
                let fail = self.emit_failure_test(r, &kind, span)?;
                let dest = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r2(Opcode::MOVE, dest, r));
                let site = self.current_insts.len();
                self.current_insts.push(0);
                let payload = self.reg_alloc.alloc_temp();
                self.emit_payload(payload, r, &kind);
                let callee = self.compile_value(f_expr)?;
                let called = self.emit_call_value(callee, Some(payload));
                self.reg_alloc.free_temp(payload);
                self.reg_alloc.free_temp(callee);
                self.current_insts.push(encode_r2(Opcode::MOVE, dest, called));
                self.reg_alloc.free_temp(called);
                let after = self.current_insts.len() as i32;
                self.current_insts[site] = encode_jc(Opcode::JMPIF, fail, (after - site as i32) as i16);
                self.reg_alloc.free_temp(fail);
                self.reg_alloc.free_temp(r);
                Ok(Some(dest))
            }
            "map" => {
                let f_expr = args.first().ok_or_else(|| ("`.map(f)` needs one argument".to_string(), span))?;
                let kind = self.fallible_kind(object);
                let succ_idx = match kind {
                    Fallible::Tagged(_) => Some(
                        self.success_variant_type_idx(object)
                            .ok_or_else(|| ("`.map()` needs `Option`/`Result` in scope (the prelude)".to_string(), span))?,
                    ),
                    _ => None,
                };
                let r = self.compile_expr(object)?;
                let fail = self.emit_failure_test(r, &kind, span)?;
                let dest = self.reg_alloc.alloc_temp();
                let site = self.current_insts.len();
                self.current_insts.push(0);
                let payload = self.reg_alloc.alloc_temp();
                self.emit_payload(payload, r, &kind);
                let callee = self.compile_value(f_expr)?;
                let called = self.emit_call_value(callee, Some(payload));
                self.reg_alloc.free_temp(payload);
                self.reg_alloc.free_temp(callee);
                self.emit_rewrap(dest, called, r, &kind, succ_idx);
                self.reg_alloc.free_temp(called);
                let to_end = self.current_insts.len();
                self.current_insts.push(0);
                let fail_at = self.current_insts.len() as i32;
                self.current_insts[site] = encode_jc(Opcode::JMPIF, fail, (fail_at - site as i32) as i16);
                self.current_insts.push(encode_r2(Opcode::MOVE, dest, r));
                let end_at = self.current_insts.len() as i32;
                self.current_insts[to_end] = encode_ju(Opcode::JMP, end_at - to_end as i32);
                self.reg_alloc.free_temp(fail);
                self.reg_alloc.free_temp(r);
                Ok(Some(dest))
            }
            "ok_or" => {
                let e_expr = args.first().ok_or_else(|| ("`.ok_or(e)` needs one argument".to_string(), span))?;
                let ok_idx = self
                    .variant_map
                    .get("Ok")
                    .map(|v| v.type_idx)
                    .ok_or_else(|| ("`.ok_or()` needs `Result` in scope (the prelude)".to_string(), span))?;
                let err_idx = self
                    .variant_map
                    .get("Err")
                    .map(|v| v.type_idx)
                    .ok_or_else(|| ("`.ok_or()` needs `Result` in scope (the prelude)".to_string(), span))?;
                let r = self.compile_expr(object)?;
                let kind = self.fallible_kind(object);
                let fail = self.emit_failure_test(r, &kind, span)?;
                let dest = self.reg_alloc.alloc_temp();
                let site = self.current_insts.len();
                self.current_insts.push(0);
                let payload = self.reg_alloc.alloc_temp();
                self.emit_payload(payload, r, &kind);
                self.current_insts.push(encode_ri(Opcode::NEWOBJ, dest, ok_idx));
                self.current_insts.push(encode_r3(Opcode::SETFIELD, dest, 0, payload));
                self.reg_alloc.free_temp(payload);
                let to_end = self.current_insts.len();
                self.current_insts.push(0);
                let err_at = self.current_insts.len() as i32;
                self.current_insts[site] = encode_jc(Opcode::JMPIF, fail, (err_at - site as i32) as i16);
                let e = self.compile_value(e_expr)?;
                self.current_insts.push(encode_ri(Opcode::NEWOBJ, dest, err_idx));
                self.current_insts.push(encode_r3(Opcode::SETFIELD, dest, 0, e));
                self.reg_alloc.free_temp(e);
                let end_at = self.current_insts.len() as i32;
                self.current_insts[to_end] = encode_ju(Opcode::JMP, end_at - to_end as i32);
                self.reg_alloc.free_temp(fail);
                self.reg_alloc.free_temp(r);
                Ok(Some(dest))
            }
            "or" => {
                let other_expr = args.first().ok_or_else(|| ("`.or(x)` needs one argument".to_string(), span))?;
                let r = self.compile_expr(object)?;
                let kind = self.fallible_kind(object);
                let fail = self.emit_failure_test(r, &kind, span)?;
                let dest = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r2(Opcode::MOVE, dest, r));
                let site = self.current_insts.len();
                self.current_insts.push(0);
                let other = self.compile_value(other_expr)?;
                self.current_insts.push(encode_r2(Opcode::MOVE, dest, other));
                self.reg_alloc.free_temp(other);
                let after = self.current_insts.len() as i32;
                self.current_insts[site] =
                    encode_jc(Opcode::JMPIFNOT, fail, (after - site as i32) as i16);
                self.reg_alloc.free_temp(fail);
                self.reg_alloc.free_temp(r);
                Ok(Some(dest))
            }
            "or_else" => {
                let f_expr = args.first().ok_or_else(|| ("`.or_else(f)` needs one argument".to_string(), span))?;
                let r = self.compile_expr(object)?;
                let kind = self.fallible_kind(object);
                let fail = self.emit_failure_test(r, &kind, span)?;
                let dest = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r2(Opcode::MOVE, dest, r));
                let site = self.current_insts.len();
                self.current_insts.push(0);
                let callee = self.compile_value(f_expr)?;
                let called = self.emit_call_value(callee, None);
                self.reg_alloc.free_temp(callee);
                self.current_insts.push(encode_r2(Opcode::MOVE, dest, called));
                self.reg_alloc.free_temp(called);
                let after = self.current_insts.len() as i32;
                self.current_insts[site] =
                    encode_jc(Opcode::JMPIFNOT, fail, (after - site as i32) as i16);
                self.reg_alloc.free_temp(fail);
                self.reg_alloc.free_temp(r);
                Ok(Some(dest))
            }
            "len" => {
                let r = self.compile_expr(object)?;
                let dest = self.emit_len(r);
                self.reg_alloc.free_temp(r);
                Ok(Some(dest))
            }
            "is_empty" => {
                let r = self.compile_expr(object)?;
                let len = self.emit_len(r);
                self.reg_alloc.free_temp(r);
                let zero = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_ri(Opcode::LOADI, zero, 0));
                let dest = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r3(Opcode::EQ, dest, len, zero));
                self.reg_alloc.free_temp(zero);
                self.reg_alloc.free_temp(len);
                Ok(Some(dest))
            }
            "into_shared" if args.is_empty() => self.emit_shared_new(object, true, span).map(Some),
            "clone" if args.is_empty() => {
                let r = self.compile_expr(object)?;
                let d = if matches!(self.types.of(object), Some(Type::Sender(_))) {
                    self.emit_native_regs(isa::intrinsics::SENDER_CLONE_INTRINSIC, &[r])
                } else {
                    self.emit_native_regs("clone", &[r])
                };
                self.reg_alloc.free_temp(r);
                Ok(Some(d))
            }
            "to_string" => {
                let r = self.compile_expr(object)?;
                let d = self.emit_native_regs("str_from", &[r]);
                self.reg_alloc.free_temp(r);
                Ok(Some(d))
            }
            "debug"
                if matches!(
                    self.types.of(object),
                    Some(Type::Int | Type::Float | Type::Bool | Type::Char | Type::String)
                ) =>
            {
                let r = self.compile_expr(object)?;
                let d = self.emit_native_regs("debug_raw", &[r]);
                self.reg_alloc.free_temp(r);
                Ok(Some(d))
            }
            "eq"
                if matches!(
                    self.types.of(object),
                    Some(Type::Int | Type::Float | Type::Bool | Type::Char | Type::String)
                ) =>
            {
                let l = self.compile_expr(object)?;
                let r = self.compile_expr(&args[0])?;
                let dest = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r3(Opcode::EQ, dest, l, r));
                self.reg_alloc.free_temp(r);
                self.reg_alloc.free_temp(l);
                Ok(Some(dest))
            }
            "compare"
                if matches!(
                    self.types.of(object),
                    Some(Type::Int | Type::Float | Type::Char | Type::String)
                ) =>
            {
                let l = self.compile_expr(object)?;
                let r = self.compile_expr(&args[0])?;
                let dest = self.reg_alloc.alloc_temp();

                let lt = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r3(Opcode::LT, lt, l, r));
                let to_check_gt = self.current_insts.len();
                self.current_insts.push(0);
                let neg_one = self.compile_expr(&Expr::Int(-1, span))?;
                self.current_insts.push(encode_r2(Opcode::MOVE, dest, neg_one));
                self.reg_alloc.free_temp(neg_one);
                let to_end_a = self.current_insts.len();
                self.current_insts.push(0);

                let check_gt_at = self.current_insts.len() as i32;
                self.current_insts[to_check_gt] =
                    encode_jc(Opcode::JMPIFNOT, lt, (check_gt_at - to_check_gt as i32) as i16);
                self.reg_alloc.free_temp(lt);

                let gt = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r3(Opcode::GT, gt, l, r));
                let to_eq = self.current_insts.len();
                self.current_insts.push(0);
                let one = self.compile_expr(&Expr::Int(1, span))?;
                self.current_insts.push(encode_r2(Opcode::MOVE, dest, one));
                self.reg_alloc.free_temp(one);
                let to_end_b = self.current_insts.len();
                self.current_insts.push(0);

                let eq_at = self.current_insts.len() as i32;
                self.current_insts[to_eq] = encode_jc(Opcode::JMPIFNOT, gt, (eq_at - to_eq as i32) as i16);
                self.reg_alloc.free_temp(gt);
                let zero = self.compile_expr(&Expr::Int(0, span))?;
                self.current_insts.push(encode_r2(Opcode::MOVE, dest, zero));
                self.reg_alloc.free_temp(zero);

                let end_at = self.current_insts.len() as i32;
                self.current_insts[to_end_a] = encode_ju(Opcode::JMP, end_at - to_end_a as i32);
                self.current_insts[to_end_b] = encode_ju(Opcode::JMP, end_at - to_end_b as i32);

                self.reg_alloc.free_temp(r);
                self.reg_alloc.free_temp(l);
                Ok(Some(dest))
            }
            "get" => Ok(Some(self.emit_list_method(
                "coll_get",
                object,
                args,
                span,
            )?)),
            "get_or" => Ok(Some(self.emit_list_method(
                "coll_get_or",
                object,
                args,
                span,
            )?)),
            "set" => Ok(Some(self.emit_list_method(
                "coll_set",
                object,
                args,
                span,
            )?)),
            "push" => Ok(Some(self.emit_list_method(
                "coll_push",
                object,
                args,
                span,
            )?)),
            "clear" => Ok(Some(self.emit_list_method(
                "coll_clear",
                object,
                args,
                span,
            )?)),
            "add" => Ok(Some(self.emit_list_method(
                "set_add",
                object,
                args,
                span,
            )?)),
            "contains" | "contains_key" => Ok(Some(self.emit_list_method(
                "coll_contains",
                object,
                args,
                span,
            )?)),
            "remove" => Ok(Some(self.emit_list_method(
                "coll_remove",
                object,
                args,
                span,
            )?)),
            "keys" => Ok(Some(self.emit_list_method(
                "map_keys",
                object,
                args,
                span,
            )?)),
            "values" => Ok(Some(self.emit_list_method(
                "map_values",
                object,
                args,
                span,
            )?)),
            "entries" => Ok(Some(self.emit_list_method(
                "map_entries",
                object,
                args,
                span,
            )?)),
            "items" => Ok(Some(self.emit_list_method(
                "set_items",
                object,
                args,
                span,
            )?)),
            "extend" => Ok(Some(self.emit_list_method(
                "bytes_extend",
                object,
                args,
                span,
            )?)),
            "slice" => Ok(Some(self.emit_list_method(
                "coll_slice",
                object,
                args,
                span,
            )?)),
            "concat" => {
                let l = self.compile_expr(object)?;
                let r = self.compile_expr(args.first().ok_or_else(|| {
                    ("`.concat(s)` needs one argument".to_string(), span)
                })?)?;
                let dest = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r3(Opcode::ADD, dest, l, r));
                self.reg_alloc.free_temp(r);
                self.reg_alloc.free_temp(l);
                Ok(Some(dest))
            }
            "replace" => Ok(Some(self.emit_list_method(
                "str_replace",
                object,
                args,
                span,
            )?)),
            "to_upper" => Ok(Some(self.emit_list_method(
                "str_to_upper",
                object,
                args,
                span,
            )?)),
            "to_lower" => Ok(Some(self.emit_list_method(
                "str_to_lower",
                object,
                args,
                span,
            )?)),
            "trim" => Ok(Some(self.emit_list_method(
                "str_trim",
                object,
                args,
                span,
            )?)),
            "split" => Ok(Some(self.emit_list_method(
                "str_split",
                object,
                args,
                span,
            )?)),
            "bytes" => Ok(Some(self.emit_list_method(
                "str_bytes",
                object,
                args,
                span,
            )?)),
            "repeat" => Ok(Some(self.emit_list_method(
                "str_repeat",
                object,
                args,
                span,
            )?)),
            "starts_with" => Ok(Some(self.emit_list_method(
                "str_starts_with",
                object,
                args,
                span,
            )?)),
            "ends_with" => Ok(Some(self.emit_list_method(
                "str_ends_with",
                object,
                args,
                span,
            )?)),
            "find" => {
                let recv = self.compile_expr(object)?;
                let sub = self.compile_expr(args.first().ok_or_else(|| {
                    ("`.find(sub)` needs one argument".to_string(), span)
                })?)?;
                let off = self.emit_native_regs("str_find", &[recv, sub]);
                let zero = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_ri(Opcode::LOADI, zero, 0));
                let missing = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r3(Opcode::LT, missing, off, zero));
                self.reg_alloc.free_temp(zero);
                self.reg_alloc.free_temp(off);
                let opt = self.emit_option_wrap(missing, span, |s| {
                    Ok(s.emit_native_regs("str_find", &[recv, sub]))
                })?;
                self.reg_alloc.free_temp(recv);
                self.reg_alloc.free_temp(sub);
                Ok(Some(opt))
            }
            "rfind" => {
                let recv = self.compile_expr(object)?;
                let sub = self.compile_expr(args.first().ok_or_else(|| {
                    ("`.rfind(sub)` needs one argument".to_string(), span)
                })?)?;
                let off = self.emit_native_regs("str_rfind", &[recv, sub]);
                let zero = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_ri(Opcode::LOADI, zero, 0));
                let missing = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r3(Opcode::LT, missing, off, zero));
                self.reg_alloc.free_temp(zero);
                self.reg_alloc.free_temp(off);
                let opt = self.emit_option_wrap(missing, span, |s| {
                    Ok(s.emit_native_regs("str_rfind", &[recv, sub]))
                })?;
                self.reg_alloc.free_temp(recv);
                self.reg_alloc.free_temp(sub);
                Ok(Some(opt))
            }
            "decode" => {
                let recv = self.compile_expr(object)?;
                let ok = self.emit_native_regs("bytes_is_utf8", &[recv]);
                let bad = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r2(Opcode::NOT, bad, ok));
                self.reg_alloc.free_temp(ok);
                let opt = self.emit_option_wrap(bad, span, |s| {
                    Ok(s.emit_native_regs("bytes_decode", &[recv]))
                })?;
                self.reg_alloc.free_temp(recv);
                Ok(Some(opt))
            }
            "pop" | "first" | "last" => {
                let recv = self.compile_expr(object)?;
                let empty = self.emit_list_is_empty(recv);
                let m = method.to_string();
                let opt = self.emit_option_wrap(empty, span, |s| match m.as_str() {
                    "pop" => Ok(s.emit_native_regs("list_pop", &[recv])),
                    _ => {
                        let idx = s.reg_alloc.alloc_temp();
                        if m == "last" {
                            let len = s.emit_len(recv);
                            let one = s.reg_alloc.alloc_temp();
                            s.current_insts.push(encode_ri(Opcode::LOADI, one, 1));
                            s.current_insts.push(encode_r3(Opcode::SUB, idx, len, one));
                            s.reg_alloc.free_temp(one);
                            s.reg_alloc.free_temp(len);
                        } else {
                            s.current_insts.push(encode_ri(Opcode::LOADI, idx, 0));
                        }
                        let v = s.emit_native_regs("coll_get", &[recv, idx]);
                        s.reg_alloc.free_temp(idx);
                        Ok(v)
                    }
                })?;
                self.reg_alloc.free_temp(recv);
                Ok(Some(opt))
            }
            "iter" => {
                let concrete_non_list = self
                    .types
                    .of(object)
                    .is_some_and(|ty| !matches!(ty, Type::List(_) | Type::Receiver(_) | Type::Stream(_) | Type::Any | Type::Hole));
                if concrete_non_list {
                    return Ok(None);
                }
                let recv = self.compile_expr(object)?;
                let it = self.emit_native_regs("list_iter", &[recv]);
                self.reg_alloc.free_temp(recv);
                Ok(Some(it))
            }
            "stream" => {
                let call = Expr::Call {
                    callee: Box::new(Expr::Ident(crate::prelude::STREAM_OF_FN.into(), span)),
                    args: vec![object.clone()],
                    names: Vec::new(),
                    span,
                };
                Ok(Some(self.compile_expr(&call)?))
            }
            "next" => {
                let it = self.compile_expr(object)?;
                let (chan, generator) = match self.types.of(object) {
                    Some(Type::Receiver(_)) => (IterFlag::Yes, IterFlag::No),
                    Some(Type::Stream(_)) => (IterFlag::No, IterFlag::Yes),
                    Some(Type::List(_) | Type::Set(_)) => (IterFlag::No, IterFlag::No),
                    _ => (self.emit_type_flag(it, CHANNEL_TYPE_ID), self.emit_type_flag(it, GENERATOR_TYPE_ID)),
                };
                let opt = self.emit_iter_next(it, chan, generator, span)?;
                for flag in [chan, generator] {
                    if let IterFlag::Reg(r) = flag {
                        self.reg_alloc.free_temp(r);
                    }
                }
                self.reg_alloc.free_temp(it);
                Ok(Some(opt))
            }
            "join" => {
                let recv = self.compile_expr(object)?;
                let wait = self.emit_native_regs(
                    isa::intrinsics::TASK_JOIN_INTRINSIC,
                    &[recv],
                );
                self.reg_alloc.free_temp(wait);
                let result = self.emit_task_join_result(recv, span)?;
                self.reg_alloc.free_temp(recv);
                Ok(Some(result))
            }
            "cancel" => {
                let recv = self.compile_expr(object)?;
                let dest = self.emit_native_regs(
                    isa::intrinsics::TASK_CANCEL_INTRINSIC,
                    &[recv],
                );
                self.reg_alloc.free_temp(recv);
                Ok(Some(dest))
            }
            "is_ready" => {
                let recv = self.compile_expr(object)?;
                let dest = self.emit_native_regs(isa::intrinsics::TASK_IS_READY_INTRINSIC, &[recv]);
                self.reg_alloc.free_temp(recv);
                Ok(Some(dest))
            }
            "send" => {
                let recv = self.compile_expr(object)?;
                let value = self.compile_expr(
                    args.first()
                        .ok_or_else(|| ("`.send(v)` needs one argument".to_string(), span))?,
                )?;
                let dest = self.emit_native_regs(
                    isa::intrinsics::CHANNEL_SEND_INTRINSIC,
                    &[recv, value],
                );
                self.reg_alloc.free_temp(value);
                self.reg_alloc.free_temp(recv);
                Ok(Some(dest))
            }
            "recv" => {
                let recv = self.compile_expr(object)?;
                let result = self.emit_recv_option(recv, span)?;
                self.reg_alloc.free_temp(recv);
                Ok(Some(result))
            }
            "close" => {
                let recv = self.compile_expr(object)?;
                let dest = self.emit_native_regs(
                    isa::intrinsics::CHANNEL_CLOSE_INTRINSIC,
                    &[recv],
                );
                self.reg_alloc.free_temp(recv);
                Ok(Some(dest))
            }
            _ => Ok(None),
        }
    }

    pub(crate) fn emit_recv_option(&mut self, recv: u8, span: Span) -> Result<u8, (String, Span)> {
        let arg_base = self.reg_alloc.alloc_block(2);
        if recv != arg_base {
            self.current_insts.push(encode_r2(Opcode::MOVE, arg_base, recv));
        }
        let dest = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(
            Opcode::CALLINTRINSIC,
            dest,
            isa::intrinsics::CHANNEL_RECV_INTRINSIC,
            arg_base,
        ));
        let result = self.emit_channel_recv_result(arg_base + 1, dest, span)?;
        self.reg_alloc.free_block(arg_base, 2);
        Ok(result)
    }

    pub(crate) fn emit_resume_option(&mut self, gen_reg: u8, span: Span) -> Result<u8, (String, Span)> {
        let value = self.reg_alloc.alloc_temp();
        let status = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(Opcode::RESUME, value, gen_reg, status));
        let zero = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::LOADI, zero, 0));
        let done = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(Opcode::EQ, done, status, zero));
        self.reg_alloc.free_temp(zero);
        self.reg_alloc.free_temp(status);
        self.emit_option_wrap(done, span, |_| Ok(value))
    }

    pub(crate) fn emit_list_next(&mut self, it: u8, span: Span) -> Result<u8, (String, Span)> {
        let list = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(Opcode::GETFIELD, list, it, 0));
        let idx = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(Opcode::GETFIELD, idx, it, 1));
        let len = self.emit_len(list);
        let lt = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(Opcode::LT, lt, idx, len));
        let done = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r2(Opcode::NOT, done, lt));
        self.reg_alloc.free_temp(lt);
        self.reg_alloc.free_temp(len);
        let opt = self.emit_option_wrap(done, span, |s| {
            let v = s.emit_native_regs("coll_get", &[list, idx]);
            let one = s.reg_alloc.alloc_temp();
            s.current_insts.push(encode_ri(Opcode::LOADI, one, 1));
            let nidx = s.reg_alloc.alloc_temp();
            s.current_insts.push(encode_r3(Opcode::ADD, nidx, idx, one));
            s.current_insts.push(encode_r3(Opcode::SETFIELD, it, 1, nidx));
            s.reg_alloc.free_temp(one);
            s.reg_alloc.free_temp(nidx);
            Ok(v)
        })?;
        self.reg_alloc.free_temp(list);
        self.reg_alloc.free_temp(idx);
        Ok(opt)
    }

    pub(crate) fn emit_type_flag(&mut self, subject: u8, type_id: u64) -> IterFlag {
        let k = self.add_constant(Value::int(type_id as i64));
        let t = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r2(Opcode::TYPEOF, t, subject));
        let kr = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::LOADK, kr, k));
        let flag = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(Opcode::EQ, flag, t, kr));
        self.reg_alloc.free_temp(kr);
        self.reg_alloc.free_temp(t);
        IterFlag::Reg(flag)
    }

    pub(crate) fn pin_flag(&mut self, type_id: u64, subject: u8, name: &str) -> IterFlag {
        let IterFlag::Reg(t) = self.emit_type_flag(subject, type_id) else { unreachable!() };
        let var = self.reg_alloc.alloc_var(name);
        self.current_insts.push(encode_r2(Opcode::MOVE, var, t));
        self.reg_alloc.free_temp(t);
        IterFlag::Reg(var)
    }

    pub(crate) fn emit_iter_next(
        &mut self,
        it: u8,
        chan: IterFlag,
        generator: IterFlag,
        span: Span,
    ) -> Result<u8, (String, Span)> {
        let out = self.reg_alloc.alloc_temp();
        let mut to_end = Vec::new();
        let mut terminal = false;
        for (flag, is_chan) in [(chan, true), (generator, false)] {
            if terminal || flag == IterFlag::No {
                continue;
            }
            let skip = match flag {
                IterFlag::Reg(r) => {
                    let at = self.current_insts.len();
                    self.current_insts.push(0);
                    Some((at, r))
                }
                _ => None,
            };
            let opt = if is_chan { self.emit_recv_option(it, span)? } else { self.emit_resume_option(it, span)? };
            self.current_insts.push(encode_r2(Opcode::MOVE, out, opt));
            self.reg_alloc.free_temp(opt);
            match skip {
                Some((at, r)) => {
                    to_end.push(self.current_insts.len());
                    self.current_insts.push(0);
                    let next_kind = self.current_insts.len() as i32;
                    self.current_insts[at] = encode_jc(Opcode::JMPIFNOT, r, (next_kind - at as i32) as i16);
                }
                None => terminal = true,
            }
        }
        if !terminal {
            let opt = self.emit_list_next(it, span)?;
            self.current_insts.push(encode_r2(Opcode::MOVE, out, opt));
            self.reg_alloc.free_temp(opt);
        }
        let end = self.current_insts.len() as i32;
        for at in to_end {
            self.current_insts[at] = encode_ju(Opcode::JMP, end - at as i32);
        }
        Ok(out)
    }

    pub(crate) fn emit_channel_recv_result(
        &mut self,
        status_reg: u8,
        payload_reg: u8,
        span: Span,
    ) -> Result<u8, (String, Span)> {
        use isa::intrinsics::RECV_GOT_VALUE;

        let got_const = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::LOADI, got_const, RECV_GOT_VALUE as u16));
        let has_value = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(Opcode::EQ, has_value, status_reg, got_const));
        self.reg_alloc.free_temp(got_const);

        let is_empty = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r2(Opcode::NOT, is_empty, has_value));
        self.reg_alloc.free_temp(has_value);

        self.emit_option_wrap(is_empty, span, |_| Ok(payload_reg))
    }

    pub(crate) fn emit_task_join_result(&mut self, handle: u8, span: Span) -> Result<u8, (String, Span)> {
        use isa::intrinsics::{STATUS_OK, TASK_SLOT_RESULT, TASK_SLOT_STATUS};

        let ok_idx = self
            .variant_map
            .get("Ok")
            .map(|v| v.type_idx)
            .ok_or_else(|| ("`Result` must be in scope (the prelude)".to_string(), span))?;
        let err_idx = self
            .variant_map
            .get("Err")
            .map(|v| v.type_idx)
            .ok_or_else(|| ("`Result` must be in scope (the prelude)".to_string(), span))?;

        let status = self.reg_alloc.alloc_temp();
        self.current_insts
            .push(encode_r3(Opcode::GETFIELD, status, handle, TASK_SLOT_STATUS as u8));
        let ok_const = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::LOADI, ok_const, STATUS_OK as u16));
        let is_ok = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(Opcode::EQ, is_ok, status, ok_const));
        self.reg_alloc.free_temp(ok_const);
        self.reg_alloc.free_temp(status);

        let payload = self.reg_alloc.alloc_temp();
        self.current_insts
            .push(encode_r3(Opcode::GETFIELD, payload, handle, TASK_SLOT_RESULT as u8));

        let res = self.reg_alloc.alloc_temp();
        let to_err = self.current_insts.len();
        self.current_insts.push(0);
        self.current_insts.push(encode_ri(Opcode::NEWOBJ, res, ok_idx));
        self.current_insts.push(encode_r3(Opcode::SETFIELD, res, 0, payload));
        let to_end = self.current_insts.len();
        self.current_insts.push(0);

        let err_at = self.current_insts.len() as i32;
        self.current_insts[to_err] =
            encode_jc(Opcode::JMPIFNOT, is_ok, (err_at - to_err as i32) as i16);
        self.current_insts.push(encode_ri(Opcode::NEWOBJ, res, err_idx));
        self.current_insts.push(encode_r3(Opcode::SETFIELD, res, 0, payload));

        let end_at = self.current_insts.len() as i32;
        self.current_insts[to_end] = encode_ju(Opcode::JMP, end_at - to_end as i32);

        self.reg_alloc.free_temp(payload);
        self.reg_alloc.free_temp(is_ok);
        Ok(res)
    }

    pub(crate) fn emit_err_value(&mut self, payload: u8, span: Span) -> Result<u8, (String, Span)> {
        let err_idx = self
            .variant_map
            .get("Err")
            .map(|v| v.type_idx)
            .ok_or_else(|| ("`Result` must be in scope (the prelude)".to_string(), span))?;
        let res = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::NEWOBJ, res, err_idx));
        self.current_insts.push(encode_r3(Opcode::SETFIELD, res, 0, payload));
        Ok(res)
    }

    /// An `Error` of kind `Other` holding the message in `message`.
    fn emit_error_from_message(&mut self, message: u8, span: Span) -> Result<u8, (String, Span)> {
        let error_fn = self
            .func_map
            .iter()
            .find(|(k, _)| k.ends_with("__native_error"))
            .map(|(_, idx)| *idx)
            .ok_or(("a scope in a function returning `Result<_, Error>` needs `std.error` in the program".to_string(), span))?;
        let (arg_base, error) = self.call_frame(CallTarget::Code(0), 2);
        let error = error.expect("a code call has a destination");
        self.current_insts.push(encode_ri(Opcode::LOADI, arg_base, 8));
        self.current_insts.push(encode_r2(Opcode::MOVE, arg_base + 1, message));
        self.emit_call_op(Self::code_target(error_fn, span)?, error, arg_base);
        let out = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r2(Opcode::MOVE, out, error));
        self.reg_alloc.free_block(error, 3);
        Ok(out)
    }

    pub(crate) fn emit_scope_fault_check(&mut self, outcome: u8, span: Span) -> Result<(), (String, Span)> {
        let null_reg = self.reg_alloc.alloc_temp();
        let null_const = self.add_constant(Value::null());
        self.current_insts.push(encode_ri(Opcode::LOADK, null_reg, null_const));
        let is_clean = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(Opcode::EQ, is_clean, outcome, null_reg));
        self.reg_alloc.free_temp(null_reg);

        let site = self.current_insts.len();
        self.current_insts.push(0);

        match self.current_scope_fault {
            ScopeFault::ErrText => {
                let err = self.emit_err_value(outcome, span)?;
                self.emit_ret(err);
                self.reg_alloc.free_temp(err);
            }
            ScopeFault::ErrError => {
                let error = self.emit_error_from_message(outcome, span)?;
                let err = self.emit_err_value(error, span)?;
                self.emit_ret(err);
                self.reg_alloc.free_temp(err);
                self.reg_alloc.free_temp(error);
            }
            ScopeFault::Panic => {
                self.emit_native_regs("panic", &[outcome]);
            }
        }

        let after = self.current_insts.len() as i32;
        self.current_insts[site] = encode_jc(Opcode::JMPIF, is_clean, (after - site as i32) as i16);
        self.reg_alloc.free_temp(is_clean);
        Ok(())
    }

    pub(crate) fn emit_list_is_empty(&mut self, recv: u8) -> u8 {
        let len = self.emit_len(recv);
        let zero = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::LOADI, zero, 0));
        let empty = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(Opcode::EQ, empty, len, zero));
        self.reg_alloc.free_temp(zero);
        self.reg_alloc.free_temp(len);
        empty
    }

    pub(crate) fn emit_option_wrap<F>(
        &mut self,
        done_reg: u8,
        _span: Span,
        produce: F,
    ) -> Result<u8, (String, Span)>
    where
        F: FnOnce(&mut Self) -> Result<u8, (String, Span)>,
    {
        let opt = self.emit_null();
        let to_end = self.current_insts.len();
        self.current_insts.push(0);
        let v = produce(self)?;
        self.current_insts.push(encode_r2(Opcode::SOME, opt, v));
        self.reg_alloc.free_temp(v);
        let end_at = self.current_insts.len() as i32;
        self.current_insts[to_end] = encode_jc(Opcode::JMPIF, done_reg, (end_at - to_end as i32) as i16);
        self.reg_alloc.free_temp(done_reg);
        Ok(opt)
    }

    pub(crate) fn emit_list_method(
        &mut self,
        native: impl Into<Callee>,
        object: &Expr,
        args: &[Expr],
        span: Span,
    ) -> Result<u8, (String, Span)> {
        let callee = native.into();
        let n = 1 + args.len();
        let (arg_base, dest) = self.native_frame(&callee, n);
        if self.reg_alloc.overflowed() {
            return Err(("list method call needs more than 256 registers".into(), span));
        }
        let recv = self.compile_expr(object)?;
        if recv != arg_base {
            self.current_insts.push(encode_r2(Opcode::MOVE, arg_base, recv));
        }
        self.reg_alloc.free_temp(recv);
        for (i, arg) in args.iter().enumerate() {
            let r = self.compile_value(arg)?;
            let target = arg_base + 1 + i as u8;
            if r != target {
                self.current_insts.push(encode_r2(Opcode::MOVE, target, r));
            }
            self.reg_alloc.free_temp(r);
        }
        self.emit_native_op(&callee, dest, arg_base);
        self.reg_alloc.free_block(arg_base, n);
        Ok(dest)
    }

}
