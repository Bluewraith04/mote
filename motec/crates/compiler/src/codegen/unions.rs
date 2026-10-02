//! Method calls on a union receiver.

use super::*;
use crate::types::Members;

impl CodeGenerator {
    pub(crate) fn try_lower_union_method_call(&mut self, object: &Expr, method: &str, args: &[Expr], span: Span) -> Result<Option<u8>, (String, Span)> {
        let Some(Type::Union(Members(members))) = self.types.of(object).cloned() else { return Ok(None) };
        let declared: Vec<(Type, String)> = members
            .iter()
            .filter_map(|m| match m {
                Type::Struct { name, .. } | Type::Class { name, .. } if self.func_map.contains_key(&Self::mangle_method_name(name, method)) => {
                    Some((m.clone(), name.clone()))
                }
                _ => None,
            })
            .collect();
        if declared.is_empty() {
            return Ok(None);
        }
        let dest = self.reg_alloc.alloc_temp();
        let recv = self.compile_expr(object)?;
        self.reg_alloc.enter_scope();
        let bound = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r2(Opcode::MOVE, bound, recv));
        let temp = format!("$recv{}", span.start);
        self.reg_alloc.bind_var(&temp, bound);
        let receiver = Expr::Ident(temp, object.span());
        let mut ends = Vec::new();
        for (ty, class) in &declared {
            let t = self.reg_alloc.alloc_temp();
            self.load_type_into(t, Some(ty));
            let hit = self.reg_alloc.alloc_temp();
            self.current_insts.push(encode_r3(Opcode::ISTYPE, hit, bound, t));
            self.reg_alloc.free_temp(t);
            let miss = self.jump_unless(hit);
            let r = self.lower_class_method_call(class.clone(), &receiver, method, args, span)?.expect("a declared method");
            self.current_insts.push(encode_r2(Opcode::MOVE, dest, r));
            self.reg_alloc.free_temp(r);
            ends.push(self.current_insts.len());
            self.current_insts.push(0);
            self.patch_unless(miss);
        }
        if declared.len() < members.len() {
            let r = self
                .try_lower_builtin_method(&receiver, method, args, span)?
                .ok_or_else(|| (format!("`.{method}()` has no lowering for every member of this union"), span))?;
            self.current_insts.push(encode_r2(Opcode::MOVE, dest, r));
            self.reg_alloc.free_temp(r);
        }
        for site in ends {
            self.patch_jmp(site);
        }
        self.leave_scope();
        self.reg_alloc.free_temp(recv);
        Ok(Some(dest))
    }
}
