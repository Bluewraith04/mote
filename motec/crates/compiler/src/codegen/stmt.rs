//! Statement and loop lowering.

use super::*;

impl CodeGenerator {
    pub(crate) fn compile_for_range(
        &mut self,
        var_name: &str,
        start: &Expr,
        end: &Expr,
        inclusive: bool,
        body: &[Stmt],
    ) -> Result<(), (String, Span)> {
        let start_reg = self.compile_expr(start)?;
        let end_reg = self.compile_expr(end)?;
        self.reg_alloc.enter_scope();
        let var_reg = self.reg_alloc.alloc_var(var_name);
        self.current_insts.push(encode_r2(Opcode::MOVE, var_reg, start_reg));

        let loop_start = self.current_insts.len() as i32;
        let cmp_reg = self.reg_alloc.alloc_temp();
        let cmp_op = if inclusive { Opcode::LE } else { Opcode::LT };
        self.current_insts.push(encode_r3(cmp_op, cmp_reg, var_reg, end_reg));

        let jmp_out_idx = self.current_insts.len();
        self.current_insts.push(0);

        self.loop_stack.push(LoopCtx { region_depth: self.region_stack.len(), with_depth: self.with_stack.len(), ..Default::default() });
        for s in body {
            self.compile_stmt(s)?;
        }
        let loop_ctx = self.loop_stack.pop().unwrap();

        let continue_target = self.current_insts.len() as i32;

        let one_reg = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::LOADI, one_reg, 1));
        self.current_insts.push(encode_r3(Opcode::ADD, var_reg, var_reg, one_reg));
        self.reg_alloc.free_temp(one_reg);

        let loop_end = self.current_insts.len() as i32;
        self.current_insts.push(encode_ju(Opcode::JMP, loop_start - loop_end));

        let after_loop = self.current_insts.len() as i32;
        self.current_insts[jmp_out_idx] =
            encode_jc(Opcode::JMPIFNOT, cmp_reg, (after_loop - jmp_out_idx as i32) as i16);

        self.patch_jumps(&loop_ctx.break_sites, after_loop);
        self.patch_jumps(&loop_ctx.continue_sites, continue_target);

        self.leave_scope();
        self.reg_alloc.free_temp(cmp_reg);
        self.reg_alloc.free_temp(end_reg);
        self.reg_alloc.free_temp(start_reg);
        Ok(())
    }

    pub(crate) fn compile_for_iter(
        &mut self,
        var_name: &str,
        iter: &Expr,
        body: &[Stmt],
        span: Span,
    ) -> Result<(), (String, Span)> {
        self.reg_alloc.enter_scope();

        let iter_call = Expr::Call {
            callee: Box::new(Expr::MemberAccess {
                object: Box::new(iter.clone()),
                member: "iter".into(),
                span,
            }),
            args: Vec::new(),
            span,
        };
        let it_tmp = self.compile_expr(&iter_call).map_err(|(e, s)| {
            (format!("cannot `for`-iterate this value (no `iter()` method): {e}"), s)
        })?;
        let it_reg = self.reg_alloc.alloc_var("__for_iter");
        if it_reg != it_tmp {
            self.current_insts.push(encode_r2(Opcode::MOVE, it_reg, it_tmp));
        }
        self.reg_alloc.free_temp(it_tmp);

        let user_protocol = matches!(
            self.types.of(iter),
            Some(Type::Struct { .. } | Type::Class { .. } | Type::Enum { .. })
        );
        let (chan, generator) = match self.types.of(iter) {
            Some(Type::Receiver(_)) => (IterFlag::Yes, IterFlag::No),
            Some(Type::Stream(_)) => (IterFlag::No, IterFlag::Yes),
            Some(Type::Any | Type::Hole) | None => (
                self.pin_flag(CHANNEL_TYPE_ID, it_reg, "__for_is_chan"),
                self.pin_flag(GENERATOR_TYPE_ID, it_reg, "__for_is_gen"),
            ),
            Some(_) => (IterFlag::No, IterFlag::No),
        };

        let var_reg = self.reg_alloc.alloc_var(var_name);
        let loop_start = self.current_insts.len() as i32;

        let next_call = Expr::Call {
            callee: Box::new(Expr::MemberAccess {
                object: Box::new(Expr::Ident("__for_iter".into(), span)),
                member: "next".into(),
                span,
            }),
            args: Vec::new(),
            span,
        };
        let opt_reg = if user_protocol {
            self.compile_expr(&next_call)?
        } else {
            self.emit_iter_next(it_reg, chan, generator, span)?
        };
        let is_some = self.emit_none_test(opt_reg, false);

        let jmp_out_idx = self.current_insts.len();
        self.current_insts.push(0);

        self.current_insts.push(encode_r2(Opcode::UNSOME, var_reg, opt_reg));
        self.reg_alloc.free_temp(opt_reg);
        if may_hold_value_object(iterated_element(self.types.of(iter))) {
            self.current_insts.push(encode_r2(Opcode::COPYVAL, var_reg, var_reg));
        }

        self.loop_stack.push(LoopCtx { region_depth: self.region_stack.len(), with_depth: self.with_stack.len(), ..Default::default() });
        for s in body {
            self.compile_stmt(s)?;
        }
        let loop_ctx = self.loop_stack.pop().unwrap();

        let loop_end = self.current_insts.len() as i32;
        self.current_insts
            .push(encode_ju(Opcode::JMP, loop_start - loop_end));

        let after_loop = self.current_insts.len() as i32;
        self.current_insts[jmp_out_idx] =
            encode_jc(Opcode::JMPIFNOT, is_some, (after_loop - jmp_out_idx as i32) as i16);

        self.patch_jumps(&loop_ctx.break_sites, after_loop);
        self.patch_jumps(&loop_ctx.continue_sites, loop_start);

        self.reg_alloc.free_temp(is_some);
        self.leave_scope();
        Ok(())
    }

    pub(crate) fn leave_scope(&mut self) {
        let depth = self.reg_alloc.scope_depth();
        while self.region_stack.last() == Some(&depth) {
            self.region_stack.pop();
            self.current_insts.push(encode_none(Opcode::EXITARENA));
        }
        while self.with_stack.last().is_some_and(|(d, ..)| *d == depth) {
            let (_, name, span) = self.with_stack.pop().unwrap();
            self.emit_close_call(&name, span);
        }
        self.reg_alloc.exit_scope();
    }

    pub(crate) fn exit_regions_above(&mut self, keep: usize) {
        for _ in keep..self.region_stack.len() {
            self.current_insts.push(encode_none(Opcode::EXITARENA));
        }
    }

    pub(crate) fn exit_withs_above(&mut self, keep: usize) {
        let pending: Vec<_> = self.with_stack[keep..].iter().rev().cloned().collect();
        for (_, name, span) in pending {
            self.emit_close_call(&name, span);
        }
    }

    fn emit_close_call(&mut self, name: &str, span: Span) {
        let call = Expr::Call {
            callee: Box::new(Expr::MemberAccess {
                object: Box::new(Expr::Ident(name.to_string(), span)),
                member: "close".to_string(),
                span,
            }),
            args: Vec::new(),
            span,
        };
        let r = self.compile_expr(&call).expect("checker already verified `Closeable`");
        self.reg_alloc.free_temp(r);
    }

    fn fits_in_region(&self, init: &Expr) -> bool {
        let Expr::StructInit { name, .. } = init else { return false };
        self.struct_init_target(name).is_some_and(|idx| self.type_descriptors[idx as usize].slots as usize <= isa::value::MAX_REGION_SLOTS)
    }

    fn note_let_site(&mut self, init: &Expr, stmt_span: Span, is_cell: bool, name: &str) {
        let Expr::StructInit { name: type_name, span: init_span, .. } = init else { return };
        self.reported_inits.insert(*init_span);
        let placed = self.regions.contains(&stmt_span) && !is_cell && self.fits_in_region(init);
        let reason = if placed {
            None
        } else if let Some(r) = self.region_heap.get(&stmt_span) {
            Some(r.clone())
        } else if is_cell {
            Some(crate::regions::HeapReason::SharedVar(name.to_string()))
        } else if self.regions.contains(&stmt_span) {
            let fields = self.struct_init_target(type_name).map_or(0, |idx| self.type_descriptors[idx as usize].fields.len());
            Some(crate::regions::HeapReason::TooManyFields(fields))
        } else {
            Some(crate::regions::HeapReason::Unanalysed)
        };
        self.memory_sites.push(crate::regions::MemorySite { span: *init_span, type_name: type_name.clone(), reason, in_registers: false });
    }

    pub(crate) fn compile_region_let(&mut self, name: &str, init: &Expr) -> Result<(), (String, Span)> {
        let Expr::StructInit { name: type_name, fields, span, .. } = init else { unreachable!() };
        let type_idx = self.struct_init_target(type_name)
            .ok_or_else(|| (format!("Unknown struct type '{}'", type_name), *span))?;
        self.current_insts.push(encode_none(Opcode::ENTERARENA));
        self.region_stack.push(self.reg_alloc.scope_depth());
        let dest = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::ARENAALLOC, dest, type_idx));
        self.emit_literal_fields(dest, 0, type_idx, fields)?;
        self.reg_alloc.bind_var(name, dest);
        Ok(())
    }

    pub(crate) fn compile_returned(&mut self, val: &Expr) -> Result<u8, (String, Span)> {
        let handed_over = matches!(val, Expr::Ident(n, _) if self.reg_alloc.get_var(n).is_some() && !self.reg_alloc.is_cell(n) && !self.param_names.contains(n) && !self.region_vars.contains(n));
        if handed_over { self.compile_expr(val) } else { self.compile_value(val) }
    }

    pub(crate) fn compile_stmt(&mut self, stmt: &Stmt) -> Result<(), (String, Span)> {
        let start = self.current_insts.len();
        let result = self.compile_stmt_node(stmt);
        self.mark_span(stmt.span(), start);
        result
    }

    fn compile_stmt_node(&mut self, stmt: &Stmt) -> Result<(), (String, Span)> {
        match stmt {
            Stmt::Let { name, init, span, .. }
            | Stmt::Var { name, init, span, .. } => {
                if self.window_lets.contains(span) {
                    if self.report_memory {
                        self.note_window_site(init);
                    }
                    return self.compile_window_let(name, init);
                }
                let is_cell = matches!(stmt, Stmt::Var { .. }) && self.cell_names.contains(name);
                if self.report_memory {
                    self.note_let_site(init, *span, is_cell, name);
                }
                if self.regions.contains(span) && !is_cell && self.fits_in_region(init) {
                    return self.compile_region_let(name, init);
                }
                let init_reg = self.compile_value(init)?;
                if is_cell {
                    self.bind_cell(name, init_reg);
                } else {
                    self.reg_alloc.bind_var(name, init_reg);
                }
            }
            Stmt::TupleLet { names, init: Expr::TupleLiteral { elements, .. }, .. } if elements.len() == names.len() => {
                let mut regs = Vec::new();
                for el in elements {
                    regs.push(self.compile_value(el)?);
                }
                for (name, reg) in names.iter().zip(regs) {
                    if name == "_" {
                        self.reg_alloc.free_temp(reg);
                    } else {
                        self.reg_alloc.bind_var(name, reg);
                    }
                }
            }
            Stmt::TupleLet { names, init, .. } => {
                let tmp = self.compile_value(init)?;
                for (i, name) in names.iter().enumerate() {
                    if name == "_" {
                        continue;
                    }
                    let r = self.reg_alloc.alloc_var(name);
                    self.current_insts.push(encode_r3(Opcode::GETFIELD, r, tmp, i as u8));
                    self.current_insts.push(encode_r2(Opcode::COPYVAL, r, r));
                }
                self.reg_alloc.free_temp(tmp);
            }
            Stmt::Assign { target, value, span } => {
                if self.try_window_assign(target, value)? {
                    return Ok(());
                }
                let val_reg = self.compile_value(value)?;
                self.store_place(target, val_reg, *span)?;
                self.reg_alloc.free_temp(val_reg);
            }
            Stmt::Expr { expr, .. } => {
                let r = self.compile_expr(expr)?;
                self.reg_alloc.free_temp(r);
            }
            Stmt::If { cond, then_branch, else_branch, .. } => {
                let cond_reg = self.compile_expr(cond)?;
                let jmp_if_not_idx = self.current_insts.len();
                self.current_insts.push(0);
                self.reg_alloc.free_temp(cond_reg);

                self.reg_alloc.enter_scope();
                for s in then_branch {
                    self.compile_stmt(s)?;
                }
                self.leave_scope();

                if let Some(else_b) = else_branch {
                    let jmp_else_idx = self.current_insts.len();
                    self.current_insts.push(0);

                    let then_end = self.current_insts.len() as i32;
                    let offset_if_not = then_end - (jmp_if_not_idx as i32);
                    self.current_insts[jmp_if_not_idx] = encode_jc(Opcode::JMPIFNOT, cond_reg, offset_if_not as i16);

                    self.reg_alloc.enter_scope();
                    for s in else_b {
                        self.compile_stmt(s)?;
                    }
                    self.leave_scope();
                    let else_end = self.current_insts.len() as i32;
                    let offset_jmp = else_end - (jmp_else_idx as i32);
                    self.current_insts[jmp_else_idx] = encode_ju(Opcode::JMP, offset_jmp);
                } else {
                    let then_end = self.current_insts.len() as i32;
                    let offset_if_not = then_end - (jmp_if_not_idx as i32);
                    self.current_insts[jmp_if_not_idx] = encode_jc(Opcode::JMPIFNOT, cond_reg, offset_if_not as i16);
                }
            }
            Stmt::While { cond, body, .. } => {
                let loop_start = self.current_insts.len() as i32;
                let cond_reg = self.compile_expr(cond)?;
                let jmp_out_idx = self.current_insts.len();
                self.current_insts.push(0);
                self.reg_alloc.free_temp(cond_reg);

                self.loop_stack.push(LoopCtx { region_depth: self.region_stack.len(), with_depth: self.with_stack.len(), ..Default::default() });
                self.reg_alloc.enter_scope();
                for s in body {
                    self.compile_stmt(s)?;
                }
                self.leave_scope();
                let loop_ctx = self.loop_stack.pop().unwrap();

                let loop_end = self.current_insts.len() as i32;
                let back_offset = loop_start - loop_end;
                self.current_insts.push(encode_ju(Opcode::JMP, back_offset));

                let after_loop = self.current_insts.len() as i32;
                let out_offset = after_loop - (jmp_out_idx as i32);
                self.current_insts[jmp_out_idx] = encode_jc(Opcode::JMPIFNOT, cond_reg, out_offset as i16);

                self.patch_jumps(&loop_ctx.break_sites, after_loop);
                self.patch_jumps(&loop_ctx.continue_sites, loop_start);
            }
            Stmt::ForIn { var_name, iter, body, span } => {
                match iter {
                    Expr::Range { start: Some(s), end: Some(e), inclusive, .. } => {
                        self.compile_for_range(var_name, s, e, *inclusive, body)?;
                    }
                    Expr::Range { .. } => {
                        return Err((
                            "`for` over an open-ended range (`a..` / `..b`) is not supported".into(),
                            *span,
                        ));
                    }
                    _ => self.compile_for_iter(var_name, iter, body, *span)?,
                }
            }
            Stmt::ScopeBlock { body, span } => {
                self.current_insts.push(encode_none(Opcode::SCOPEENTER));
                self.reg_alloc.enter_scope();
                for s in body {
                    self.compile_stmt(s)?;
                }
                self.leave_scope();
                let outcome = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_scopeexit(outcome));
                self.emit_scope_fault_check(outcome, *span)?;
                self.reg_alloc.free_temp(outcome);
            }
            Stmt::SpawnBlock { body, span } => {
                let handle = self.compile_spawn(body, *span)?;
                self.reg_alloc.free_temp(handle);
            }
            Stmt::Return { value, .. } => {
                if let (Some(ty), Some(val)) = (self.current_sig.as_ref().and_then(|s| s.ret.clone()), value) {
                    return self.compile_wr_return(&ty, val);
                }
                let ret_reg = if let Some(val) = value {
                    self.compile_returned(val)?
                } else {
                    let r = self.reg_alloc.alloc_temp();
                    self.current_insts.push(encode_ri(Opcode::LOADI, r, 0));
                    r
                };
                self.exit_regions_above(0);
                self.exit_withs_above(0);
                self.emit_ret(ret_reg);
                self.reg_alloc.free_temp(ret_reg);
            }
            Stmt::CompoundAssign { target, op, value, span } => {
                let Some(bin_op) = op.binary_op() else {
                    return self.compile_coalesce_assign(target, value, *span);
                };
                let opcode = binop_opcode(bin_op);
                let guard = self.is_untyped(target) || self.is_untyped(value);
                if self.try_window_compound(target, opcode, bin_op.symbol(), guard, value)? {
                    return Ok(());
                }
                let val_reg = self.compile_expr(value)?;
                match target {
                    Expr::Ident(name, i_span) => {
                        if let Some(var_reg) = self.reg_alloc.get_var(name) {
                            let cell = self.reg_alloc.is_cell(name).then_some(var_reg);
                            let cur = match cell {
                                Some(c) => {
                                    let tmp = self.reg_alloc.alloc_temp();
                                    self.current_insts.push(encode_r3(Opcode::GETFIELD, tmp, c, 0));
                                    tmp
                                }
                                None => var_reg,
                            };
                            if guard {
                                self.emit_operand_guard("__operand_check", bin_op.symbol(), &[cur, val_reg]);
                            }
                            self.current_insts.push(encode_r3(opcode, cur, cur, val_reg));
                            if let Some(c) = cell {
                                self.current_insts.push(encode_r3(Opcode::SETFIELD, c, 0, cur));
                                self.reg_alloc.free_temp(cur);
                            }
                        } else if let Some(&gidx) = self.global_map.get(name) {
                            let tmp = self.reg_alloc.alloc_temp();
                            self.current_insts.push(encode_ri(Opcode::GETGLOBAL, tmp, gidx));
                            if guard {
                                self.emit_operand_guard("__operand_check", bin_op.symbol(), &[tmp, val_reg]);
                            }
                            self.current_insts.push(encode_r3(opcode, tmp, tmp, val_reg));
                            self.current_insts.push(encode_ri(Opcode::SETGLOBAL, tmp, gidx));
                            self.reg_alloc.free_temp(tmp);
                        } else {
                            return Err((format!("Undefined variable '{}'", name), *i_span));
                        }
                    }
                    Expr::MemberAccess { object, member, span } => {
                        let target = self.field_target(object, member)?;
                        let tmp = self.emit_read_target(&target, member, *span);
                        if guard {
                            self.emit_operand_guard("__operand_check", bin_op.symbol(), &[tmp, val_reg]);
                        }
                        self.current_insts.push(encode_r3(opcode, tmp, tmp, val_reg));
                        self.emit_write_target(&target, member, tmp);
                        self.reg_alloc.free_temp(tmp);
                        self.reg_alloc.free_temp(target.root);
                    }
                    Expr::Index { object, index, .. } => {
                        let obj_reg = self.compile_expr(object)?;
                        let idx_reg = self.compile_expr(index)?;
                        let tmp = self.emit_native_regs("coll_get", &[obj_reg, idx_reg]);
                        if guard {
                            self.emit_operand_guard("__operand_check", bin_op.symbol(), &[tmp, val_reg]);
                        }
                        self.current_insts.push(encode_r3(opcode, tmp, tmp, val_reg));
                        let d = self.emit_native_regs("coll_set", &[obj_reg, idx_reg, tmp]);
                        self.reg_alloc.free_temp(d);
                        self.reg_alloc.free_temp(tmp);
                        self.reg_alloc.free_temp(idx_reg);
                        self.reg_alloc.free_temp(obj_reg);
                    }
                    _ => return Err(("Invalid assignment target".into(), *span)),
                }
                self.reg_alloc.free_temp(val_reg);
            }
            Stmt::Break { span, .. } => {
                if self.loop_stack.is_empty() {
                    return Err(("`break` outside a loop".to_string(), *span));
                }
                self.exit_regions_above(self.loop_stack.last().unwrap().region_depth);
                self.exit_withs_above(self.loop_stack.last().unwrap().with_depth);
                let site = self.current_insts.len();
                self.current_insts.push(0);
                self.loop_stack.last_mut().unwrap().break_sites.push(site);
            }
            Stmt::Continue { span } => {
                if self.loop_stack.is_empty() {
                    return Err(("`continue` outside a loop".to_string(), *span));
                }
                self.exit_regions_above(self.loop_stack.last().unwrap().region_depth);
                self.exit_withs_above(self.loop_stack.last().unwrap().with_depth);
                let site = self.current_insts.len();
                self.current_insts.push(0);
                self.loop_stack.last_mut().unwrap().continue_sites.push(site);
            }
            Stmt::Match { expr, arms, span } => {
                self.lower_match(expr, arms, *span)?;
            }
            Stmt::Yield { value, .. } => {
                let r = self.compile_value(value)?;
                self.current_insts.push(encode_r2(Opcode::YIELD, r, 0));
                self.reg_alloc.free_temp(r);
            }
            Stmt::Block { body, .. } => {
                self.reg_alloc.enter_scope();
                for s in body {
                    self.compile_stmt(s)?;
                }
                self.leave_scope();
            }
            Stmt::WithBlock { name, init, body, .. } => {
                let init_span = init.span();
                let init_reg = self.compile_value(init)?;
                self.reg_alloc.enter_scope();
                self.reg_alloc.bind_var(name, init_reg);
                self.with_stack.push((self.reg_alloc.scope_depth(), name.clone(), init_span));
                for s in body {
                    self.compile_stmt(s)?;
                }
                self.leave_scope();
            }
        }
        Ok(())
    }

}
