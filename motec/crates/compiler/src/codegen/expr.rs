//! Expression lowering.

use super::*;

impl CodeGenerator {
    pub(crate) fn compile_expr(&mut self, expr: &Expr) -> Result<u8, (String, Span)> {
        let start = self.current_insts.len();
        let result = self.compile_expr_node(expr);
        if let (Ok(reg), Expr::Call { span, .. }) = (&result, expr) {
            self.stamp_call_result(*reg, *span);
        }
        if let (Ok(reg), Some(want)) = (&result, self.types.any_check(expr.span()).cloned()) {
            self.emit_any_check(*reg, &want);
        }
        let result = result.map(|mut reg| {
            let n = self.types.rewrap(expr.span());
            for _ in 0..n.unsigned_abs() {
                let next = if n > 0 {
                    self.emit_some(reg)
                } else {
                    let d = self.reg_alloc.alloc_temp();
                    self.current_insts.push(encode_r2(Opcode::UNSOME, d, reg));
                    d
                };
                self.reg_alloc.free_temp(reg);
                reg = next;
            }
            reg
        });
        self.mark_span(expr.span(), start);
        result
    }

    pub(crate) fn mark_span(&mut self, span: Span, start_pc: usize) {
        let end = self.current_insts.len();
        if crate::span::is_release() || end <= start_pc || span.source == 0 {
            return;
        }
        self.current_spans.push(isa::code::SourceSpan {
            start_pc: start_pc as u32,
            end_pc: end as u32,
            source: span.source,
            offset: span.start as u32,
            len: (span.end - span.start) as u32,
            line: span.line as u32,
            col: span.col as u32,
        });
    }

    pub(crate) fn is_untyped(&self, expr: &Expr) -> bool {
        !matches!(self.types.of(expr), Some(Type::Int | Type::Float | Type::String | Type::Bool))
    }

    pub(crate) fn emit_operand_guard(&mut self, native: &'static str, symbol: &str, operands: &[u8]) {
        let sym_idx = self.add_string(symbol.to_string());
        let sym_reg = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::NEWSTR, sym_reg, sym_idx));
        let verbose_idx = self.add_constant(if crate::span::is_release() { Value::false_() } else { Value::true_() });
        let verbose_reg = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::LOADK, verbose_reg, verbose_idx));
        let mut args = vec![sym_reg];
        args.extend_from_slice(operands);
        args.push(verbose_reg);
        let dest = self.emit_native_regs(native, &args);
        self.reg_alloc.free_temp(dest);
        self.reg_alloc.free_temp(verbose_reg);
        self.reg_alloc.free_temp(sym_reg);
    }

    fn compile_expr_node(&mut self, expr: &Expr) -> Result<u8, (String, Span)> {
        match expr {
            Expr::Int(val, _) => {
                let dest = self.reg_alloc.alloc_temp();
                if *val >= 0 && *val <= 0xFFFF {
                    self.current_insts.push(encode_ri(Opcode::LOADI, dest, *val as u16));
                } else {
                    let const_idx = self.add_constant(Value::int(*val));
                    self.current_insts.push(encode_ri(Opcode::LOADK, dest, const_idx));
                }
                Ok(dest)
            }
            Expr::Float(val, _) => {
                let dest = self.reg_alloc.alloc_temp();
                let const_idx = self.add_constant(Value::float(*val));
                self.current_insts.push(encode_ri(Opcode::LOADK, dest, const_idx));
                Ok(dest)
            }
            Expr::Bool(val, _) => {
                let dest = self.reg_alloc.alloc_temp();
                let const_idx = self.add_constant(if *val { Value::true_() } else { Value::false_() });
                self.current_insts.push(encode_ri(Opcode::LOADK, dest, const_idx));
                Ok(dest)
            }
            Expr::Char(c, _) => {
                let dest = self.reg_alloc.alloc_temp();
                let const_idx = self.add_constant(Value::char(*c));
                self.current_insts.push(encode_ri(Opcode::LOADK, dest, const_idx));
                Ok(dest)
            }
            Expr::String(s, _) => {
                let dest = self.reg_alloc.alloc_temp();
                let str_idx = self.add_string(s.clone());
                self.current_insts.push(encode_ri(Opcode::NEWSTR, dest, str_idx));
                Ok(dest)
            }
            Expr::Null(_) => {
                let dest = self.reg_alloc.alloc_temp();
                let const_idx = self.add_constant(Value::null());
                self.current_insts.push(encode_ri(Opcode::LOADK, dest, const_idx));
                Ok(dest)
            }
            Expr::Ident(name, span) => {
                if let Some(var_reg) = self.reg_alloc.get_var(name) {
                    if self.region_vars.contains(name) && !self.safe_uses.contains(span) {
                        return Err((format!("internal error: `{name}` lives in a region but is used where the region may end first"), *span));
                    }
                    let dest = self.reg_alloc.alloc_temp();
                    if self.reg_alloc.is_cell(name) {
                        self.current_insts.push(encode_r3(Opcode::GETFIELD, dest, var_reg, 0));
                    } else {
                        self.current_insts.push(encode_r2(Opcode::MOVE, dest, var_reg));
                    }
                    return Ok(dest);
                }
                if let Some(&gidx) = self.global_map.get(name) {
                    let dest = self.reg_alloc.alloc_temp();
                    self.current_insts.push(encode_ri(Opcode::GETGLOBAL, dest, gidx));
                    return Ok(dest);
                }
                if let Some(&func_idx) = self.func_map.get(name) {
                    if self.hidden_params.contains_key(name) {
                        return self.generic_fn_value(name, func_idx, *span);
                    }
                    return self.emit_function_value(func_idx, &[], *span);
                }
                if let Some(info) = self.variant_map.get(name).cloned() {
                    if info.arity != 0 {
                        return Err((
                            format!("Variant '{}' takes {} value(s) — write '{}(…)'", name, info.arity, name),
                            *span,
                        ));
                    }
                    return self.emit_unit_variant(&info, *span);
                }
                Err((format!("Undefined identifier '{}'", name), *span))
            }
            Expr::SelfValue(span) => {
                let var_reg = self
                    .reg_alloc
                    .get_var("self")
                    .ok_or_else(|| ("`self` used outside a method body".to_string(), *span))?;
                let dest = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r2(Opcode::MOVE, dest, var_reg));
                Ok(dest)
            }
            Expr::Binary { left, op: op @ (BinaryOp::And | BinaryOp::Or), right, .. } => {
                let dest = self.reg_alloc.alloc_temp();
                let l_reg = self.compile_expr(left)?;
                self.current_insts.push(encode_r2(Opcode::MOVE, dest, l_reg));
                self.reg_alloc.free_temp(l_reg);
                let site = self.current_insts.len();
                self.current_insts.push(0);
                let r_reg = self.compile_expr(right)?;
                self.current_insts.push(encode_r2(Opcode::MOVE, dest, r_reg));
                self.reg_alloc.free_temp(r_reg);
                let skip = if *op == BinaryOp::And { Opcode::JMPIFNOT } else { Opcode::JMPIF };
                let offset = self.current_insts.len() as i32 - site as i32;
                self.current_insts[site] = encode_jc(skip, dest, offset as i16);
                Ok(dest)
            }
            Expr::TypeTest { expr, span, .. } => {
                let v = self.compile_expr(expr)?;
                let target = self.types.type_test(*span).cloned();
                let t = self.reg_alloc.alloc_temp();
                self.load_type_into(t, target.as_ref());
                let dest = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r3(Opcode::ISTYPE, dest, v, t));
                self.reg_alloc.free_temp(t);
                self.reg_alloc.free_temp(v);
                Ok(dest)
            }
            Expr::Ternary { cond, then_expr, else_expr, .. } => {
                let dest = self.reg_alloc.alloc_temp();
                let c_reg = self.compile_expr(cond)?;
                let to_else = self.current_insts.len();
                self.current_insts.push(0);
                self.reg_alloc.free_temp(c_reg);
                let t_reg = self.compile_value(then_expr)?;
                self.current_insts.push(encode_r2(Opcode::MOVE, dest, t_reg));
                self.reg_alloc.free_temp(t_reg);
                let to_end = self.current_insts.len();
                self.current_insts.push(0);
                let else_at = self.current_insts.len() as i32;
                self.current_insts[to_else] = encode_jc(Opcode::JMPIFNOT, c_reg, (else_at - to_else as i32) as i16);
                let e_reg = self.compile_value(else_expr)?;
                self.current_insts.push(encode_r2(Opcode::MOVE, dest, e_reg));
                self.reg_alloc.free_temp(e_reg);
                let end_at = self.current_insts.len() as i32;
                self.current_insts[to_end] = encode_ju(Opcode::JMP, end_at - to_end as i32);
                Ok(dest)
            }
            Expr::NullCoalesce { left, right, .. } => self.compile_coalesce(expr, left, right),
            Expr::OptionalChain { object, temp, body, .. } => self.compile_optional_chain(object, temp, body),
            Expr::Binary { left, op, right, .. } => {
                let opcode = binop_opcode(*op);
                let l_reg = self.compile_expr(left)?;
                let r_reg = self.compile_expr(right)?;
                if !matches!(op, BinaryOp::Eq | BinaryOp::NotEq) && (self.is_untyped(left) || self.is_untyped(right)) {
                    self.emit_operand_guard("__operand_check", op.symbol(), &[l_reg, r_reg]);
                }
                let dest = self.reg_alloc.alloc_temp();

                self.current_insts.push(encode_r3(opcode, dest, l_reg, r_reg));
                self.reg_alloc.free_temp(r_reg);
                self.reg_alloc.free_temp(l_reg);
                Ok(dest)
            }
            Expr::Unary { op, expr, .. } => {
                let src_reg = self.compile_expr(expr)?;
                if self.is_untyped(expr) {
                    let sym = match op {
                        UnaryOp::Neg => "-",
                        UnaryOp::Not => "!",
                        UnaryOp::BitNot => "~",
                    };
                    self.emit_operand_guard("__unary_check", sym, &[src_reg]);
                }
                let dest = self.reg_alloc.alloc_temp();
                match op {
                    UnaryOp::Neg =>self.current_insts.push(encode_r2(Opcode::NEG, dest, src_reg)),
                    UnaryOp::Not => self.current_insts.push(encode_r2(Opcode::NOT, dest, src_reg)),
                    UnaryOp::BitNot => self.current_insts.push(encode_r2(Opcode::BNOT, dest, src_reg)),
                }
                self.reg_alloc.free_temp(src_reg);
                Ok(dest)
            }
            Expr::Call { callee, args, span } => {
                if let Some(r) = self.try_call_with_embedded_places(expr)? {
                    return Ok(r);
                }
                if let Some(r) = self.try_lower_static_call(callee, args, *span)? {
                    return Ok(r);
                }
                if let Expr::Ident(func_name, _) = callee.as_ref() {
                    if func_name == "print" || func_name == "println" || func_name == "assert" {
                        let target = Callee::Builtin(match func_name.as_str() {
                            "print" => "print",
                            "println" => "println",
                            _ => "assert",
                        });
                        let (arg_base, dest) = self.native_frame(&target, 1);
                        if self.reg_alloc.overflowed() {
                            return Err(("call expression needs more than 256 registers".into(), *span));
                        }
                        if let Some(arg) = args.first() {
                            let r = self.compile_expr(arg)?;
                            if r != arg_base {
                                self.current_insts.push(encode_r2(Opcode::MOVE, arg_base, r));
                            }
                            self.reg_alloc.free_temp(r);
                        }
                        self.emit_native_op(&target, dest, arg_base);
                        self.reg_alloc.free_block(arg_base, 1);
                        return Ok(dest);
                    }

                    if func_name == "assert_eq" {
                        return self.emit_assert_eq(args, *span);
                    }

                    if func_name == "__task_any" {
                        let arg = self.compile_expr(
                            args.first()
                                .ok_or_else(|| ("`__task_any` needs one argument".to_string(), *span))?,
                        )?;
                        let winner = self.emit_native_regs(isa::intrinsics::TASK_ANY_INTRINSIC, &[arg]);
                        self.reg_alloc.free_temp(arg);
                        return Ok(winner);
                    }

                    if func_name == "__task_pin" {
                        let arg = self.compile_expr(
                            args.first()
                                .ok_or_else(|| ("`__task_pin` needs one argument".to_string(), *span))?,
                        )?;
                        let pinned = self.emit_native_regs(isa::intrinsics::TASK_PIN_INTRINSIC, &[arg]);
                        self.reg_alloc.free_temp(arg);
                        return Ok(pinned);
                    }

                    if func_name.starts_with("__")
                        && let Some(name) = crate::builtin_natives::lookup(func_name) {
                            if crate::builtin_natives::is_fallible(func_name) {
                                return self.emit_fallible_native_call(name, args, *span);
                            }
                            return self.emit_native_call(Callee::Builtin(name), args, *span);
                        }

                    if func_name == "typeof"
                        && args.len() == 1
                        && self.reg_alloc.get_var(func_name).is_none()
                    {
                        let arg = self.compile_expr(&args[0])?;
                        let dest = self.emit_native_regs("typeof", &[arg]);
                        self.reg_alloc.free_temp(arg);
                        return Ok(dest);
                    }

                    if let Some(var_reg) = self.reg_alloc.get_var(func_name) {
                        self.value_call_vars = self.types.of(callee).map(Type::var_modes).unwrap_or_default();
                        if self.reg_alloc.is_cell(func_name) {
                            let f = self.reg_alloc.alloc_temp();
                            self.current_insts.push(encode_r3(Opcode::GETFIELD, f, var_reg, 0));
                            let r = self.emit_call(CallTarget::Value(f), args, None, *span);
                            self.reg_alloc.free_temp(f);
                            return r;
                        }
                        return self.emit_call(CallTarget::Value(var_reg), args, None, *span);
                    }

                    if let Some(&gidx) = self.global_map.get(func_name) {
                        self.value_call_vars = self.types.of(callee).map(Type::var_modes).unwrap_or_default();
                        let f = self.reg_alloc.alloc_temp();
                        self.current_insts.push(encode_ri(Opcode::GETGLOBAL, f, gidx));
                        let r = self.emit_call(CallTarget::Value(f), args, None, *span);
                        self.reg_alloc.free_temp(f);
                        return r;
                    }

                    let func_name = &self.types.instance_call(*span).map(str::to_string).unwrap_or_else(|| func_name.clone());
                    if let Some(func_idx) = self.func_map.get(func_name).copied() {
                        return self.emit_call(Self::code_target(func_idx, *span)?, args, Some(func_name), *span);
                    }

                    if let Some(info) = self.variant_map.get(func_name).cloned() {
                        return self.emit_variant_call(&info, args, *span);
                    }
                }

                if let Some(info) = self.qualified_variant(callee) {
                    return self.emit_variant_call(&info, args, *span);
                }

                if let (Expr::MemberAccess { object, member, .. }, [arg]) = (callee.as_ref(), args.as_slice())
                    && let Expr::Ident(name, _) = object.as_ref() {
                        let native = match name.as_str() {
                            "Int" => Some("int_from"),
                            "Float" => Some("num_to_float"),
                            "String" => Some("str_from"),
                            _ => None,
                        };
                        if let Some(native) = native.filter(|_| member == "from" && self.reg_alloc.get_var(name).is_none()) {
                            let r = self.compile_value(arg)?;
                            let dest = self.emit_native_regs(native, &[r]);
                            self.reg_alloc.free_temp(r);
                            return Ok(dest);
                        }
                    }
                if let Expr::MemberAccess { object, member, span: m_span } = callee.as_ref() {
                    if let Some(r) = self.try_lower_union_method_call(object, member, args, *m_span)? {
                        self.recording_calls.insert(*span);
                        return Ok(r);
                    }
                    if let Some(r) = self.try_lower_class_method_call(object, member, args, *m_span)? {
                        self.recording_calls.insert(*span);
                        return Ok(r);
                    }
                }

                if let Expr::MemberAccess { object, member, span: m_span } = callee.as_ref()
                    && let Some(r) = self.try_lower_builtin_method(object, member, args, *m_span)? {
                        return Ok(r);
                    }

                if matches!(callee.as_ref(), Expr::Call { .. }) {
                    let callee_reg = self.compile_expr(callee)?;
                    let r = self.emit_call(CallTarget::Value(callee_reg), args, None, *span);
                    self.reg_alloc.free_temp(callee_reg);
                    return r;
                }
                Err((format!("Call target not found: {:?}", callee), *span))
            }
            Expr::MemberAccess { object, member, span } => {
                if let Some(info) = self.qualified_variant(expr) {
                    return self.emit_unit_variant(&info, *span);
                }
                if let Some(dest) = self.try_window_read(object, member) {
                    return Ok(dest);
                }
                let target = self.field_target(object, member)?;
                let dest = self.emit_read_target(&target, member, *span);
                self.reg_alloc.free_temp(target.root);
                Ok(dest)
            }
            Expr::StructInit { name, fields, span, .. } => {
                let type_idx = self.struct_init_target(name)
                    .ok_or_else(|| (format!("Unknown struct type '{}'", name), *span))?;
                if self.report_memory && !self.reported_inits.contains(span) {
                    let site = crate::regions::MemorySite { span: *span, type_name: name.clone(), reason: Some(crate::regions::HeapReason::NotBound), in_registers: false };
                    self.memory_sites.push(site);
                }
                let dest = self.reg_alloc.alloc_temp();
                self.emit_new_object(dest, type_idx, *span);
                self.emit_literal_fields(dest, 0, type_idx, fields)?;
                Ok(dest)
            }
            Expr::StaticAccess { span, .. } if self.qualified_variant(expr).is_some() => {
                let info = self.qualified_variant(expr).unwrap();
                self.emit_unit_variant(&info, *span)
            }
            Expr::Try { expr: inner, span } => {
                let r = self.compile_expr(inner)?;
                let kind = self.fallible_kind(inner);
                let fail = self.emit_failure_test(r, &kind, *span)?;
                let site = self.current_insts.len();
                self.current_insts.push(0);
                self.exit_regions_above(0);
                self.exit_withs_above(0);
                self.emit_ret(r);
                let after = self.current_insts.len() as i32;
                self.current_insts[site] =
                    encode_jc(Opcode::JMPIFNOT, fail, (after - site as i32) as i16);
                self.reg_alloc.free_temp(fail);
                let dest = self.reg_alloc.alloc_temp();
                self.emit_payload(dest, r, &kind);
                self.reg_alloc.free_temp(r);
                Ok(dest)
            }
            Expr::Unwrap { expr: inner, span } => {
                let r = self.compile_expr(inner)?;
                let kind = self.fallible_kind(inner);
                let fail = self.emit_failure_test(r, &kind, *span)?;
                let site = self.current_insts.len();
                self.current_insts.push(0);
                self.emit_panic("called `!` on an empty Option / an Err value");
                let after = self.current_insts.len() as i32;
                self.current_insts[site] =
                    encode_jc(Opcode::JMPIFNOT, fail, (after - site as i32) as i16);
                self.reg_alloc.free_temp(fail);
                let dest = self.reg_alloc.alloc_temp();
                self.emit_payload(dest, r, &kind);
                self.reg_alloc.free_temp(r);
                Ok(dest)
            }
            Expr::Lambda { params, return_type, body, span, .. } => {
                let param_names: Vec<String> = params.iter().map(|p| p.name.clone()).collect();

                let mut bound: HashSet<String> = param_names.iter().cloned().collect();
                let mut captures: Vec<String> = Vec::new();
                self.lambda_free_vars(body, &mut bound, &mut captures);
                captures.retain(|n| self.reg_alloc.get_var(n).is_some());
                let mut capture_regs: Vec<u8> = captures
                    .iter()
                    .map(|n| self.reg_alloc.get_var(n).expect("filtered to bound names"))
                    .collect();
                let mut cell_captures: Vec<bool> = captures.iter().map(|n| self.reg_alloc.is_cell(n)).collect();
                let type_captures = self.type_scope_captures();
                for (name, reg) in &type_captures {
                    captures.push(name.clone());
                    capture_regs.push(*reg);
                    cell_captures.push(false);
                }

                let code_idx = self.next_code_idx;
                self.next_code_idx += 1;
                self.pending_lambdas.push_back(PendingLambda {
                    code_idx,
                    params: param_names,
                    cell_captures,
                    captures,
                    return_type: return_type.clone(),
                    body: body.clone(),
                    span: *span,
                    returns_param: std::mem::take(&mut self.next_lambda_returns_param),
                    prebuilt: None,
                    module: self.module.clone(),
                });
                let value = self.emit_function_value(code_idx, &capture_regs, *span);
                for (_, reg) in type_captures {
                    self.reg_alloc.free_temp(reg);
                }
                value
            }
            Expr::ListLiteral { elements, span } => {
                let cap = self.reg_alloc.alloc_temp();
                self.current_insts
                    .push(encode_ri(Opcode::LOADI, cap, elements.len() as u16));
                let list = self.emit_native_regs("list_new", &[cap]);
                self.reg_alloc.free_temp(cap);
                self.stamp_at(list, *span);
                for el in elements {
                    let er = self.compile_value(el)?;
                    let d = self
                        .emit_native_regs("coll_push", &[list, er]);
                    self.reg_alloc.free_temp(d);
                    self.reg_alloc.free_temp(er);
                }
                Ok(list)
            }
            Expr::MapLiteral { entries, span } => {
                let cap = self.reg_alloc.alloc_temp();
                self.current_insts
                    .push(encode_ri(Opcode::LOADI, cap, entries.len() as u16));
                let map = self.emit_native_regs("map_new", &[cap]);
                self.reg_alloc.free_temp(cap);
                self.stamp_at(map, *span);
                for (k, v) in entries {
                    let kr = self.compile_expr(k)?;
                    let vr = self.compile_value(v)?;
                    let d = self.emit_native_regs(
                        "coll_set",
                        &[map, kr, vr],
                    );
                    self.reg_alloc.free_temp(d);
                    self.reg_alloc.free_temp(vr);
                    self.reg_alloc.free_temp(kr);
                }
                Ok(map)
            }
            Expr::Index { object, index, span } => {
                self.emit_list_method("coll_get", object, std::slice::from_ref(&**index), *span)
            }
            Expr::TupleLiteral { elements, span } => {
                let type_idx = self.tuple_type_idx(elements.len());
                let dest = self.reg_alloc.alloc_temp();
                self.emit_new_object(dest, type_idx, *span);
                for (i, el) in elements.iter().enumerate() {
                    let val_reg = self.compile_value(el)?;
                    self.current_insts.push(encode_r3(Opcode::SETFIELD, dest, i as u8, val_reg));
                    self.reg_alloc.free_temp(val_reg);
                }
                Ok(dest)
            }
            Expr::Spawn { body, span } => {
                let task = self.compile_spawn(body, *span)?;
                self.stamp_at(task, *span);
                Ok(task)
            }
            other => Err((
                format!(
                    "{} is not supported by codegen yet",
                    Self::expr_kind(other)
                ),
                other.span(),
            )),
        }
    }

}
