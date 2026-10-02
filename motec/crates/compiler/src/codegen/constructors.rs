//! Static calls: `T(args)` calls `T.new(args)`, and `Name<Args>` in an expression.

use super::*;

impl CodeGenerator {
    fn static_call_target(&self, callee: &Expr) -> Option<(String, String)> {
        let is_type = |n: &str| self.reg_alloc.get_var(n).is_none() && !self.func_map.contains_key(n);
        match callee {
            Expr::Ident(n, _) if is_type(n) => Some((n.clone(), "new".into())),
            Expr::MemberAccess { object, member, .. } => match object.as_ref() {
                Expr::Ident(n, _) if is_type(n) => Some((n.clone(), member.clone())),
                _ => None,
            },
            Expr::StaticAccess { target: TypeNode::Generic(n, _, _) | TypeNode::Named(n, _), member, .. } => Some((n.clone(), member.clone())),
            _ => None,
        }
    }

    pub(crate) fn try_lower_static_call(&mut self, callee: &Expr, args: &[Expr], span: Span) -> Result<Option<u8>, (String, Span)> {
        let Some((name, member)) = self.static_call_target(callee) else { return Ok(None) };
        let written_fn = matches!(callee, Expr::StaticAccess { target: TypeNode::Generic(..), .. });
        if let Some(idx) = self.func_map.get(&name).copied().filter(|_| written_fn && member == "new" && self.reg_alloc.get_var(&name).is_none()) {
            if crate::foreign::is_bind(&name) {
                return self.emit_bind(&name, args, span).map(Some);
            }
            return self.emit_call(Self::code_target(idx, span)?, args, Some(&name), span).map(Some);
        }
        if member == "new"
            && let Some(r) = self.lower_builtin_new(&name, args, span)? {
                return Ok(Some(r));
            }
        let mangled = Self::mangle_method_name(&name, &member);
        match self.func_map.get(&mangled).copied() {
            Some(idx) => self.emit_call(Self::code_target(idx, span)?, args, Some(&mangled), span).map(Some),
            None => Ok(None),
        }
    }

    fn lower_builtin_new(&mut self, name: &str, args: &[Expr], span: Span) -> Result<Option<u8>, (String, Span)> {
        if let ("Shared", [v]) = (name, args) {
            return self.emit_shared_new(v, false, span).map(Some);
        }
        if let ("Channel", [capacity]) = (name, args) {
            return self.emit_channel_new(capacity, span).map(Some);
        }
        let native = match (name, args.len()) {
            ("Map", 0) => Callee::from("map_new"),
            ("Set", 0) => Callee::from("set_new"),
            ("Bytes", 0) => Callee::from("bytes_new"),
            ("Bytes", 1) => Callee::from("bytes_zeros"),
            ("List", 0) => Callee::from("list_new"),
            _ => return Ok(None),
        };
        let arg = match args.first() {
            Some(a) => self.compile_expr(a)?,
            None => {
                let zero = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_ri(Opcode::LOADI, zero, 0));
                zero
            }
        };
        let dest = self.emit_native_regs(native, &[arg]);
        self.reg_alloc.free_temp(arg);
        self.stamp_at(dest, span);
        Ok(Some(dest))
    }

    fn emit_bind(&mut self, name: &str, args: &[Expr], span: Span) -> Result<u8, (String, Span)> {
        let f = match self.types.at(span) {
            Some(Type::Enum { args, .. }) => args.first().cloned(),
            Some(Type::Result(ok, _)) => Some(ok.as_ref().clone()),
            _ => None,
        };
        let (signature, arity) = crate::foreign::signature_of(f.as_ref()).map_err(|m| (m, span))?;
        let blocking = crate::foreign::is_bind_blocking(name);
        let raw = name.replace("libtools_bind_blocking", "libtools_bind").replace("libtools_bind", "libtools___bind_raw");
        let idx = self.func_map.get(&raw).copied().ok_or_else(|| ("`bind` needs `import std.dev.libtools`".to_string(), span))?;
        let mut all = args.to_vec();
        all.push(Expr::String(signature, span));
        all.push(Expr::Int(arity as i64, span));
        all.push(Expr::Bool(blocking, span));
        self.emit_call(Self::code_target(idx, span)?, &all, Some(&raw), span)
    }

    fn emit_channel_new(&mut self, capacity: &Expr, span: Span) -> Result<u8, (String, Span)> {
        let arg = self.compile_expr(capacity)?;
        let receiver = self.emit_native_regs(isa::intrinsics::CHANNEL_NEW_INTRINSIC, &[arg]);
        self.reg_alloc.free_temp(arg);
        let sender = self.emit_native_regs(isa::intrinsics::CHANNEL_SENDER_INTRINSIC, &[receiver]);
        if let Some(Type::Tuple(ends)) = self.types.at(span).cloned() {
            self.stamp(sender, &ends[0]);
            self.stamp(receiver, &ends[1]);
        }
        let type_idx = self.tuple_type_idx(2);
        let dest = self.reg_alloc.alloc_temp();
        self.emit_new_object(dest, type_idx, span);
        for (i, end) in [sender, receiver].into_iter().enumerate() {
            self.current_insts.push(encode_r3(Opcode::SETFIELD, dest, i as u8, end));
            self.reg_alloc.free_temp(end);
        }
        Ok(dest)
    }
}
