//! Variadic parameter call-site packing.

use super::*;

impl CodeGenerator {
    pub(super) fn collect_variadic_fns(&mut self, items: &[Item]) {
        for item in items {
            match item {
                Item::Function(f) => self.register_variadic_fn(f.name.clone(), f),
                Item::Class(c) => {
                    for m in &c.methods {
                        self.register_variadic_fn(Self::mangle_method_name(&c.name, &m.name), m);
                    }
                }
                Item::Struct(s) => {
                    for m in &s.methods {
                        self.register_variadic_fn(Self::mangle_method_name(&s.name, &m.name), m);
                    }
                }
                Item::Enum(e) => {
                    for m in &e.methods {
                        self.register_variadic_fn(Self::mangle_method_name(&e.name, &m.name), m);
                    }
                }
                _ => {}
            }
        }
    }

    fn register_variadic_fn(&mut self, key: String, f: &FunctionDecl) {
        if !f.params.iter().any(|p| matches!(p.kind, Some(ParameterKind::Variadic { .. }))) {
            return;
        }
        let fixed = f
            .params
            .iter()
            .filter(|p| !matches!(p.kind, Some(ParameterKind::Variadic { .. }) | Some(ParameterKind::SelfValue { .. })))
            .count();
        self.variadic_fns.insert(key, fixed);
    }

    pub(super) fn emit_variadic_call(
        &mut self,
        call: CallTarget,
        args: &[Expr],
        fixed: usize,
        callee: Option<&str>,
        span: Span,
    ) -> Result<u8, (String, Span)> {
        let n = fixed + 1 + self.hidden_type_args(callee);
        let (arg_base, dest) = self.call_frame(call, n);
        if self.reg_alloc.overflowed() {
            return Err(("call expression needs more than 256 registers".into(), span));
        }
        for (i, arg) in args.iter().take(fixed).enumerate() {
            let r = self.compile_arg(arg, matches!(call, CallTarget::Code(_)))?;
            let target = arg_base + i as u8;
            if r != target {
                self.current_insts.push(encode_r2(Opcode::MOVE, target, r));
            }
            self.reg_alloc.free_temp(r);
        }
        let list = self.pack_variadic_tail(&args[fixed.min(args.len())..], span)?;
        let target = arg_base + fixed as u8;
        if list != target {
            self.current_insts.push(encode_r2(Opcode::MOVE, target, list));
        }
        self.reg_alloc.free_temp(list);
        self.emit_type_args(callee, arg_base + fixed as u8 + 1, span);
        let dest = dest.unwrap_or_else(|| self.reg_alloc.alloc_temp());
        self.emit_call_op(call, dest, arg_base);
        self.reg_alloc.free_block(arg_base, n);
        self.store_back(callee, args, dest, span)?;
        Ok(dest)
    }

    pub(super) fn pack_variadic_tail(&mut self, tail: &[Expr], call_span: Span) -> Result<u8, (String, Span)> {
        let cap = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::LOADI, cap, tail.len() as u16));
        let list = self.emit_native_regs("list_new", &[cap]);
        self.reg_alloc.free_temp(cap);
        if let Some(ty) = self.types.variadic_list(call_span).cloned() {
            self.stamp(list, &ty);
        }
        for el in tail {
            let er = self.compile_value(el)?;
            let d = self.emit_native_regs("coll_push", &[list, er]);
            self.reg_alloc.free_temp(d);
            self.reg_alloc.free_temp(er);
        }
        Ok(list)
    }
}
