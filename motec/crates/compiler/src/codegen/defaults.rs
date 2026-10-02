//! Default parameters: each default becomes a hidden zero-argument function; a call that omits the argument calls it.

use super::*;

impl CodeGenerator {
    fn default_fn_name(callee: &str, n: usize) -> String {
        format!("{callee}::default_{n}")
    }

    pub(super) fn add_default_functions(&mut self, mut items: Vec<Item>) -> Vec<Item> {
        let mut added = Vec::new();
        for item in &items {
            match item {
                Item::Function(f) => self.collect_defaults(&f.name, f, &mut added),
                Item::Class(c) => {
                    for m in &c.methods {
                        self.collect_defaults(&Self::mangle_method_name(&c.name, &m.name), m, &mut added);
                    }
                }
                Item::Struct(s) => {
                    for m in &s.methods {
                        self.collect_defaults(&Self::mangle_method_name(&s.name, &m.name), m, &mut added);
                    }
                }
                Item::Enum(e) => {
                    for m in &e.methods {
                        self.collect_defaults(&Self::mangle_method_name(&e.name, &m.name), m, &mut added);
                    }
                }
                _ => {}
            }
        }
        items.extend(added);
        items
    }

    fn collect_defaults(&mut self, callee: &str, f: &FunctionDecl, added: &mut Vec<Item>) {
        let params = f.params.iter().filter(|p| !matches!(p.kind, Some(ParameterKind::SelfValue { .. })));
        let mut names = Vec::new();
        for (n, p) in params.enumerate() {
            let Some(ParameterKind::Regular { default_value: Some(default), .. }) = &p.kind else {
                names.push(None);
                continue;
            };
            let name = Self::default_fn_name(callee, n);
            added.push(Item::Function(FunctionDecl {
                name: name.clone(),
                generic_params: Vec::new(),
                params: Vec::new(),
                return_type: None,
                body: vec![Stmt::Return { value: Some(default.clone()), span: p.span }],
                is_pub: false,
                span: p.span,
            }));
            names.push(Some(name));
        }
        if names.iter().any(Option::is_some) {
            self.default_fns.insert(callee.to_string(), names);
        }
    }

    pub(super) fn call_arity(&self, callee: &str, given: usize) -> usize {
        self.default_fns.get(callee).map_or(given, |d| d.len().max(given))
    }

    pub(super) fn emit_default_args(&mut self, callee: &str, given: usize, first_slot: u8) -> Result<(), (String, Span)> {
        let Some(names) = self.default_fns.get(callee).cloned() else { return Ok(()) };
        for (i, name) in names.iter().enumerate().skip(given) {
            let idx = name
                .as_ref()
                .and_then(|n| self.func_map.get(n).copied())
                .ok_or_else(|| (format!("`{callee}` needs argument {}: it has no default", i + 1), Span::new(0, 0, 1, 1)))?;
            let dest = self.reg_alloc.alloc_temp();
            let target = Self::code_target(idx, Span::new(0, 0, 1, 1))?;
            self.emit_call_op(target, dest, dest.wrapping_add(1));
            self.current_insts.push(encode_r2(Opcode::MOVE, first_slot + i as u8, dest));
            self.reg_alloc.free_temp(dest);
        }
        Ok(())
    }
}
