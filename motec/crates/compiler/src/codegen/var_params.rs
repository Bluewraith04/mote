//! `var` parameters: a callee that replaces one returns its final value too, and the caller stores it back.

use super::cells::{walk_stmts, Node};
use super::*;

impl CodeGenerator {
    pub(crate) fn replaced_params(f: &FunctionDecl) -> Vec<usize> {
        let mut replaced = HashSet::new();
        walk_stmts(&f.body, &mut |node| match node {
            Node::Stmt(Stmt::Assign { target: Expr::Ident(n, _), .. })
            | Node::Stmt(Stmt::CompoundAssign { target: Expr::Ident(n, _), .. }) => {
                replaced.insert(n.clone());
            }
            Node::Expr(Expr::Call { args, .. }) => {
                replaced.extend(args.iter().filter_map(|a| if let Expr::Ident(n, _) = a { Some(n.clone()) } else { None }));
            }
            _ => {}
        });
        let regular = |p: &Param| matches!(p.kind, Some(ParameterKind::Regular { .. }));
        f.params.iter().enumerate().filter(|(_, p)| p.is_mut && regular(p) && replaced.contains(&p.name)).map(|(i, _)| i).collect()
    }

    pub(crate) fn note_writeback(&mut self, key: &str, f: &FunctionDecl) {
        let has_self = matches!(f.params.first().and_then(|p| p.kind.as_ref()), Some(ParameterKind::SelfValue { .. }));
        let var_args: Vec<usize> = (0..f.params.len())
            .filter(|&i| f.params[i].is_mut && matches!(f.params[i].kind, Some(ParameterKind::Regular { .. })))
            .map(|i| i - has_self as usize)
            .collect();
        if !var_args.is_empty() {
            self.var_args.insert(key.to_string(), var_args);
        }
        let args: Vec<usize> = Self::replaced_params(f).into_iter().map(|i| i - has_self as usize).collect();
        if !args.is_empty() {
            self.writeback.insert(key.to_string(), args);
        }
    }

    pub(crate) fn enter_writeback(&mut self, f: &FunctionDecl) -> Result<(), (String, Span)> {
        self.fn_writeback.clear();
        self.ret_param = None;
        let is_var = |p: &&Param| p.is_mut && matches!(p.kind, Some(ParameterKind::Regular { .. }));
        if let Some(p) = f.params.iter().find(is_var).filter(|_| stmts_yield(&f.body)) {
            return Err((format!("a generator cannot take `var {}`", p.name), p.span));
        }
        let celled: Vec<&Param> = f.params.iter().filter(is_var).filter(|p| self.cell_names.contains(&p.name)).collect();
        for p in celled {
            let reg = self.reg_alloc.get_var(&p.name).expect("a parameter's register");
            let cell = self.emit_new_cell(reg);
            self.reg_alloc.bind_var(&p.name, cell);
            self.reg_alloc.mark_cell(&p.name);
        }
        for i in Self::replaced_params(f) {
            let p = &f.params[i];
            let reg = self.reg_alloc.get_var(&p.name).expect("a parameter's register");
            self.fn_writeback.push((reg, self.reg_alloc.is_cell(&p.name)));
        }
        Ok(())
    }

    pub(crate) fn emit_ret(&mut self, r: u8) {
        if let Some(name) = self.ret_param.clone() {
            let reg = self.reg_alloc.get_var(&name).expect("a parameter's register");
            if self.reg_alloc.is_cell(&name) {
                let t = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r3(Opcode::GETFIELD, t, reg, 0));
                self.current_insts.push(encode_r2(Opcode::RET, t, 0));
                self.reg_alloc.free_temp(t);
            } else {
                self.current_insts.push(encode_r2(Opcode::RET, reg, 0));
            }
            return;
        }
        if self.fn_writeback.is_empty() {
            self.current_insts.push(encode_r2(Opcode::RET, r, 0));
            return;
        }
        let slots = self.fn_writeback.clone();
        let type_idx = self.tuple_type_idx(slots.len() + 1);
        let out = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::NEWOBJ, out, type_idx));
        self.current_insts.push(encode_r3(Opcode::SETFIELD, out, 0, r));
        for (i, (reg, cell)) in slots.into_iter().enumerate() {
            let slot = (i + 1) as u8;
            if cell {
                let t = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r3(Opcode::GETFIELD, t, reg, 0));
                self.current_insts.push(encode_r3(Opcode::SETFIELD, out, slot, t));
                self.reg_alloc.free_temp(t);
            } else {
                self.current_insts.push(encode_r3(Opcode::SETFIELD, out, slot, reg));
            }
        }
        self.current_insts.push(encode_r2(Opcode::RET, out, 0));
        self.reg_alloc.free_temp(out);
    }

    pub(crate) fn store_back(&mut self, key: Option<&str>, args: &[Expr], dest: u8, span: Span) -> Result<(), (String, Span)> {
        let Some(positions) = key.and_then(|k| self.writeback.get(k)).cloned() else { return Ok(()) };
        let packed = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r2(Opcode::MOVE, packed, dest));
        for (i, pos) in positions.into_iter().enumerate() {
            let Some(place) = args.get(pos).filter(|a| matches!(a, Expr::Ident(..) | Expr::SelfValue(_) | Expr::MemberAccess { .. } | Expr::Index { .. })) else { continue };
            if matches!(place, Expr::SelfValue(_)) {
                return Err(("`self` cannot be replaced; this call assigns its `var` parameter".into(), span));
            }
            let v = self.reg_alloc.alloc_temp();
            self.current_insts.push(encode_r3(Opcode::GETFIELD, v, packed, (i + 1) as u8));
            self.store_place(place, v, span)?;
            self.reg_alloc.free_temp(v);
        }
        self.current_insts.push(encode_r3(Opcode::GETFIELD, dest, packed, 0));
        self.reg_alloc.free_temp(packed);
        Ok(())
    }

    pub(crate) fn compile_arg(&mut self, arg: &Expr, user_code: bool) -> Result<u8, (String, Span)> {
        if user_code { self.compile_expr(arg) } else { self.compile_value(arg) }
    }

    pub(crate) fn store_place(&mut self, target: &Expr, val_reg: u8, span: Span) -> Result<(), (String, Span)> {
        match target {
            Expr::Ident(name, i_span) => {
                if let Some(var_reg) = self.reg_alloc.get_var(name) {
                    if self.reg_alloc.is_cell(name) {
                        self.current_insts.push(encode_r3(Opcode::SETFIELD, var_reg, 0, val_reg));
                    } else {
                        self.current_insts.push(encode_r2(Opcode::MOVE, var_reg, val_reg));
                    }
                } else if let Some(&gidx) = self.global_map.get(name) {
                    self.current_insts.push(encode_ri(Opcode::SETGLOBAL, val_reg, gidx));
                } else {
                    return Err((format!("Undefined variable '{}'", name), *i_span));
                }
            }
            Expr::MemberAccess { object, member, .. } => {
                let target = self.field_target(object, member)?;
                self.emit_write_target(&target, member, val_reg);
                self.reg_alloc.free_temp(target.root);
            }
            Expr::Index { object, index, .. } => {
                let obj_reg = self.compile_expr(object)?;
                let idx_reg = self.compile_expr(index)?;
                let d = self.emit_native_regs("coll_set", &[obj_reg, idx_reg, val_reg]);
                self.reg_alloc.free_temp(d);
                self.reg_alloc.free_temp(idx_reg);
                self.reg_alloc.free_temp(obj_reg);
            }
            other => return Err((format!("Invalid assignment target: {:?}", other), span)),
        }
        Ok(())
    }
}
