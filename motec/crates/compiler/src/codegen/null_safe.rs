//! `??`, `??=` and `?.`.

use super::*;

enum Place {
    Local(u8),
    Cell(u8),
    Global(u16),
    Field(u8, Option<u8>, String),
    Index(u8, u8),
}

impl CodeGenerator {
    pub(crate) fn jump_unless(&mut self, cond: u8) -> (usize, u8) {
        let site = self.current_insts.len();
        self.current_insts.push(0);
        self.reg_alloc.free_temp(cond);
        (site, cond)
    }

    pub(crate) fn patch_unless(&mut self, (site, cond): (usize, u8)) {
        let off = self.current_insts.len() as i32 - site as i32;
        self.current_insts[site] = encode_jc(Opcode::JMPIFNOT, cond, off as i16);
    }

    pub(crate) fn patch_jmp(&mut self, site: usize) {
        let off = self.current_insts.len() as i32 - site as i32;
        self.current_insts[site] = encode_ju(Opcode::JMP, off);
    }

    fn wraps_into_optional(&self, e: &Expr) -> bool {
        !matches!(self.types.of(e), None | Some(Type::Nullable(_) | Type::Any | Type::Null))
    }

    pub(crate) fn compile_coalesce(&mut self, whole: &Expr, left: &Expr, right: &Expr) -> Result<u8, (String, Span)> {
        let dest = self.reg_alloc.alloc_temp();
        let r = self.compile_expr(left)?;
        let present = self.emit_none_test(r, false);
        let to_else = self.jump_unless(present);
        let result = self.types.of(whole);
        if matches!(result, Some(Type::Nullable(_))) && result == self.types.of(left) {
            let p = self.reg_alloc.alloc_temp();
            self.current_insts.push(encode_r2(Opcode::UNSOME, p, r));
            let s = self.emit_some(p);
            self.current_insts.push(encode_r2(Opcode::MOVE, dest, s));
            self.reg_alloc.free_temp(s);
            self.reg_alloc.free_temp(p);
        } else {
            self.current_insts.push(encode_r2(Opcode::UNSOME, dest, r));
        }
        let to_end = self.current_insts.len();
        self.current_insts.push(0);
        self.patch_unless(to_else);
        let b = self.compile_value(right)?;
        self.current_insts.push(encode_r2(Opcode::MOVE, dest, b));
        self.reg_alloc.free_temp(b);
        self.patch_jmp(to_end);
        self.reg_alloc.free_temp(r);
        Ok(dest)
    }

    pub(crate) fn compile_optional_chain(&mut self, object: &Expr, temp: &str, body: &Expr) -> Result<u8, (String, Span)> {
        let dest = self.reg_alloc.alloc_temp();
        let r = self.compile_expr(object)?;
        let present = self.emit_none_test(r, false);
        let to_none = self.jump_unless(present);
        self.reg_alloc.enter_scope();
        let p = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r2(Opcode::UNSOME, p, r));
        self.reg_alloc.bind_var(temp, p);
        let v = self.compile_expr(body)?;
        if self.wraps_into_optional(body) {
            let s = self.emit_some(v);
            self.current_insts.push(encode_r2(Opcode::MOVE, dest, s));
            self.reg_alloc.free_temp(s);
        } else {
            self.current_insts.push(encode_r2(Opcode::MOVE, dest, v));
        }
        self.reg_alloc.free_temp(v);
        self.leave_scope();
        let to_end = self.current_insts.len();
        self.current_insts.push(0);
        self.patch_unless(to_none);
        let null = self.emit_null();
        self.current_insts.push(encode_r2(Opcode::MOVE, dest, null));
        self.reg_alloc.free_temp(null);
        self.patch_jmp(to_end);
        self.reg_alloc.free_temp(r);
        Ok(dest)
    }

    pub(crate) fn compile_coalesce_assign(&mut self, target: &Expr, value: &Expr, span: Span) -> Result<(), (String, Span)> {
        let place = match target {
            Expr::Ident(name, i_span) => match self.reg_alloc.get_var(name) {
                Some(reg) if self.reg_alloc.is_cell(name) => Place::Cell(reg),
                Some(reg) => Place::Local(reg),
                None => match self.global_map.get(name) {
                    Some(&g) => Place::Global(g),
                    None => return Err((format!("Undefined variable '{}'", name), *i_span)),
                },
            },
            Expr::MemberAccess { object, member, .. } => {
                let target = self.field_target(object, member)?;
                Place::Field(target.root, target.slot, member.clone())
            }
            Expr::Index { object, index, .. } => {
                let o = self.compile_expr(object)?;
                Place::Index(o, self.compile_expr(index)?)
            }
            _ => return Err(("Invalid assignment target".into(), span)),
        };
        let cur = match &place {
            Place::Local(reg) => *reg,
            Place::Cell(c) => {
                let t = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r3(Opcode::GETFIELD, t, *c, 0));
                t
            }
            Place::Global(g) => {
                let t = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_ri(Opcode::GETGLOBAL, t, *g));
                t
            }
            Place::Field(o, Some(idx), _) => {
                let t = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r3(Opcode::GETFIELD, t, *o, *idx));
                t
            }
            Place::Field(o, None, member) => {
                let member = member.clone();
                self.emit_named_field(*o, &member, None)
            }
            Place::Index(o, i) => self.emit_native_regs("coll_get", &[*o, *i]),
        };
        let none = self.emit_none_test(cur, true);
        let to_end = self.jump_unless(none);
        let v = self.compile_value(value)?;
        match &place {
            Place::Local(reg) => self.current_insts.push(encode_r2(Opcode::MOVE, *reg, v)),
            Place::Cell(c) => self.current_insts.push(encode_r3(Opcode::SETFIELD, *c, 0, v)),
            Place::Global(g) => self.current_insts.push(encode_ri(Opcode::SETGLOBAL, v, *g)),
            Place::Field(o, Some(idx), _) => self.current_insts.push(encode_r3(Opcode::SETFIELD, *o, *idx, v)),
            Place::Field(o, None, member) => {
                let member = member.clone();
                let d = self.emit_named_field(*o, &member, Some(v));
                self.reg_alloc.free_temp(d);
            }
            Place::Index(o, i) => {
                let d = self.emit_native_regs("coll_set", &[*o, *i, v]);
                self.reg_alloc.free_temp(d);
            }
        }
        self.reg_alloc.free_temp(v);
        self.patch_unless(to_end);
        match place {
            Place::Local(_) => {}
            Place::Cell(_) | Place::Global(_) => self.reg_alloc.free_temp(cur),
            Place::Field(o, ..) => {
                self.reg_alloc.free_temp(cur);
                self.reg_alloc.free_temp(o);
            }
            Place::Index(o, i) => {
                self.reg_alloc.free_temp(cur);
                self.reg_alloc.free_temp(i);
                self.reg_alloc.free_temp(o);
            }
        }
        Ok(())
    }
}
