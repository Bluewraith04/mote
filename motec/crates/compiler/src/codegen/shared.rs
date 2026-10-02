//! `Shared<T>` cells: `Shared(v)`, `into_shared`, `get`, `update`, `set` and `wait_until`.

use isa::intrinsics::{SEAL_CLAIM, SEAL_COPY, SEAL_IN_PLACE, SHARED_BEGIN_INTRINSIC, SHARED_COMMIT_INTRINSIC, SHARED_GET_INTRINSIC, SHARED_NEW_INTRINSIC, SHARED_WAIT_INTRINSIC};

use super::*;

impl CodeGenerator {
    fn is_fresh(&self, e: &Expr) -> bool {
        match e {
            Expr::Int(..) | Expr::Float(..) | Expr::Bool(..) | Expr::String(..) | Expr::Char(..) | Expr::Null(..)
            | Expr::StructInit { .. } | Expr::ListLiteral { .. } | Expr::MapLiteral { .. } | Expr::TupleLiteral { .. } => true,
            Expr::Call { callee, .. } => match callee.as_ref() {
                Expr::Ident(n, _) => self.reg_alloc.get_var(n).is_none() && !self.func_map.contains_key(n),
                Expr::StaticAccess { member, .. } => member == "new",
                _ => false,
            },
            _ => false,
        }
    }

    fn emit_mode(&mut self, mode: i64) -> u8 {
        let r = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::LOADI, r, mode as u16));
        r
    }

    pub(crate) fn emit_shared_new(&mut self, value: &Expr, moved: bool, span: Span) -> Result<u8, (String, Span)> {
        let v = self.compile_expr(value)?;
        let mode = self.emit_mode(if moved || self.is_fresh(value) { SEAL_IN_PLACE } else { SEAL_COPY });
        let dest = self.emit_native_regs(SHARED_NEW_INTRINSIC, &[v, mode]);
        self.reg_alloc.free_temp(mode);
        self.reg_alloc.free_temp(v);
        self.stamp_at(dest, span);
        Ok(dest)
    }

    pub(crate) fn try_lower_shared_method(&mut self, object: &Expr, method: &str, args: &[Expr], span: Span) -> Result<Option<u8>, (String, Span)> {
        if !matches!(self.types.of(object), Some(Type::Shared(_))) {
            return Ok(None);
        }
        let dest = match (method, args) {
            ("get", []) => {
                let cell = self.compile_expr(object)?;
                let d = self.emit_native_regs(SHARED_GET_INTRINSIC, &[cell]);
                self.reg_alloc.free_temp(cell);
                d
            }
            ("update", [f]) => {
                let cell = self.compile_expr(object)?;
                let is_lambda = matches!(f, Expr::Lambda { .. });
                self.next_lambda_returns_param = is_lambda;
                let func = self.compile_expr(f)?;
                self.next_lambda_returns_param = false;
                let want = self.emit_mode(1);
                let working = self.emit_native_regs(SHARED_BEGIN_INTRINSIC, &[cell, want]);
                self.reg_alloc.free_temp(want);
                let ret = self.emit_call_value(func, Some(working));
                let result = if is_lambda {
                    self.reg_alloc.free_temp(working);
                    ret
                } else {
                    self.reg_alloc.free_temp(ret);
                    working
                };
                self.reg_alloc.free_temp(func);
                let mode = self.emit_mode(SEAL_CLAIM);
                let d = self.emit_native_regs(SHARED_COMMIT_INTRINSIC, &[cell, result, mode]);
                self.reg_alloc.free_temp(mode);
                self.reg_alloc.free_temp(result);
                self.reg_alloc.free_temp(cell);
                d
            }
            ("wait_until", [pred]) => {
                let cell = self.compile_expr(object)?;
                let func = self.compile_expr(pred)?;
                let seen = self.emit_mode(0);
                let first = self.emit_mode(1);
                let top = self.current_insts.len() as i32;
                let v = self.emit_native_regs(SHARED_WAIT_INTRINSIC, &[cell, seen, first]);
                self.current_insts.push(encode_r2(Opcode::MOVE, seen, v));
                self.current_insts.push(encode_ri(Opcode::LOADI, first, 0));
                let ok = self.emit_call_value(func, Some(v));
                let at = self.current_insts.len() as i32;
                self.current_insts.push(encode_jc(Opcode::JMPIFNOT, ok, (top - at) as i16));
                for r in [ok, first, seen, func, cell] {
                    self.reg_alloc.free_temp(r);
                }
                v
            }
            ("set", [v]) => {
                let cell = self.compile_expr(object)?;
                let value = self.compile_expr(v)?;
                let want = self.emit_mode(0);
                let none = self.emit_native_regs(SHARED_BEGIN_INTRINSIC, &[cell, want]);
                self.reg_alloc.free_temp(none);
                self.reg_alloc.free_temp(want);
                let mode = self.emit_mode(if self.is_fresh(v) { SEAL_IN_PLACE } else { SEAL_COPY });
                let d = self.emit_native_regs(SHARED_COMMIT_INTRINSIC, &[cell, value, mode]);
                self.reg_alloc.free_temp(mode);
                self.reg_alloc.free_temp(value);
                self.reg_alloc.free_temp(cell);
                d
            }
            _ => return Ok(None),
        };
        self.stamp_at(dest, span);
        Ok(Some(dest))
    }
}
