//! Recorded type arguments: instance descriptors, `STAMP`/`STAMPT`, and hidden type-argument parameters.

use super::*;

#[derive(Clone)]
pub(crate) struct HiddenTypeParams {
    pub user_params: usize,
    pub names: Vec<String>,
}

impl CodeGenerator {
    pub(crate) fn instance_of(&mut self, template: TypeDescriptor, term: TypeTerm) -> u16 {
        let key = (template.id, template.fields.len(), term);
        if let Some(&idx) = self.instances.get(&key) {
            return idx;
        }
        let idx = self.type_descriptors.len() as u16;
        self.type_descriptors.push(TypeDescriptor { instance: Some(key.2.clone()), ..template });
        self.instances.insert(key, idx);
        idx
    }

    pub(crate) fn instance_idx(&mut self, base: u16, span: Span) -> u16 {
        match self.types.at(span).and_then(Type::instance_term) {
            Some(term) => self.instance_of(self.type_descriptors[base as usize].clone(), term),
            None => base,
        }
    }

    pub(crate) fn emit_new_object(&mut self, dest: u8, base: u16, span: Span) {
        self.recording_calls.insert(span);
        let idx = self.instance_idx(base, span);
        self.current_insts.push(encode_ri(Opcode::NEWOBJ, dest, idx));
        if idx == base {
            self.stamp_at(dest, span);
        }
    }

    pub(crate) fn stamp_at(&mut self, reg: u8, span: Span) {
        self.recording_calls.insert(span);
        if let Some(ty) = self.types.at(span).cloned() {
            self.stamp(reg, &ty);
        }
    }

    pub(crate) fn stamp(&mut self, reg: u8, ty: &Type) {
        if !ty.records_instance() {
            return;
        }
        if let (Some(id), Some(term)) = (ty.intrinsic_id(), ty.runtime_term()) {
            let template = TypeDescriptor::generic_intrinsic(id).expect("a generic intrinsic id");
            let idx = self.instance_of(template, term);
            self.current_insts.push(encode_ri(Opcode::STAMP, reg, idx));
            return;
        }
        let Some(term) = self.pattern(ty) else { return };
        let idx = self.term_idx(term);
        let t = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::LOADTYPE, t, idx));
        self.current_insts.push(encode_r2(Opcode::STAMPT, reg, t));
        self.reg_alloc.free_temp(t);
    }

    pub(crate) fn stamp_call_result(&mut self, reg: u8, span: Span) {
        let Some(ty) = self.types.at(span).cloned() else { return };
        let generator = matches!(ty, Type::Stream(_));
        if generator || (!self.recording_calls.contains(&span) && ty.is_exact()) {
            self.stamp(reg, &ty);
        }
    }

    pub(crate) fn pattern(&self, ty: &Type) -> Option<TypeTerm> {
        ty.term_with(&|name| self.type_scope.get(name).cloned())
    }

    pub(crate) fn emit_any_check(&mut self, reg: u8, want: &Type) {
        let Some(target) = self.pattern(want) else { return };
        let idx = self.term_idx(target);
        let t = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::LOADTYPE, t, idx));
        self.current_insts.push(encode_r2(Opcode::ASTYPE, reg, t));
        self.reg_alloc.free_temp(t);
    }

    fn term_idx(&mut self, term: TypeTerm) -> u16 {
        self.instance_of(TypeDescriptor::new(isa::value::TYPE_TERM_ID, Vec::new()), term)
    }

    pub(crate) fn load_type_into(&mut self, dest: u8, ty: Option<&Type>) {
        match ty.and_then(|t| self.pattern(t)) {
            Some(term) => {
                let idx = self.term_idx(term);
                self.current_insts.push(encode_ri(Opcode::LOADTYPE, dest, idx));
            }
            None => {
                let k = self.add_constant(Value::null());
                self.current_insts.push(encode_ri(Opcode::LOADK, dest, k));
            }
        }
    }

    pub(crate) fn hidden_type_args(&self, callee: Option<&str>) -> usize {
        callee.and_then(|c| self.hidden_params.get(c)).map_or(0, |h| h.names.len())
    }

    pub(crate) fn emit_type_args(&mut self, callee: Option<&str>, first: u8, span: Span) {
        let Some(hidden) = callee.and_then(|c| self.hidden_params.get(c)).cloned() else { return };
        let args = self.types.type_args(span).map(<[Type]>::to_vec).unwrap_or_default();
        for j in 0..hidden.names.len() {
            self.load_type_into(first + j as u8, args.get(j));
        }
    }

    pub(crate) fn note_hidden_params(&mut self, key: &str, f: &FunctionDecl, class_params: &[String]) {
        let names = Self::hidden_names(f, class_params);
        if !names.is_empty() {
            self.hidden_params.insert(key.to_string(), HiddenTypeParams { user_params: f.params.len(), names });
        }
    }

    fn hidden_names(f: &FunctionDecl, class_params: &[String]) -> Vec<String> {
        let class = class_params.iter().filter(|_| !Self::has_self(f)).cloned();
        class.chain(f.generic_params.iter().map(|p| p.name.clone())).collect()
    }

    fn has_self(f: &FunctionDecl) -> bool {
        matches!(f.params.first().and_then(|p| p.kind.as_ref()), Some(ParameterKind::SelfValue { .. }))
    }

    pub(crate) fn physical_params(f: &FunctionDecl, class_params: &[String]) -> Vec<String> {
        let own = f.params.iter().map(|p| p.name.clone());
        own.chain(Self::hidden_names(f, class_params).into_iter().map(|n| format!("${n}"))).collect()
    }

    pub(crate) fn enter_type_scope(&mut self, f: &FunctionDecl, class_params: &[String]) {
        self.type_scope.clear();
        if Self::has_self(f)
            && let Some(self_reg) = self.reg_alloc.get_var("self") {
                for (i, name) in class_params.iter().enumerate() {
                    self.type_scope.insert(name.clone(), TypeTerm::ArgOf(self_reg, i as u8));
                }
            }
        for name in Self::hidden_names(f, class_params) {
            if let Some(reg) = self.reg_alloc.get_var(&format!("${name}")) {
                self.type_scope.insert(name, TypeTerm::Reg(reg));
            }
        }
    }

    pub(crate) fn type_scope_captures(&mut self) -> Vec<(String, u8)> {
        let mut names: Vec<String> = self.type_scope.keys().cloned().collect();
        names.sort();
        names
            .into_iter()
            .map(|name| {
                let reg = self.reg_alloc.alloc_temp();
                self.load_type_into(reg, Some(&Type::Param(name.clone())));
                (format!("${name}"), reg)
            })
            .collect()
    }

    pub(crate) fn bind_captured_type_scope(&mut self, captures: &[String]) {
        self.type_scope.clear();
        for name in captures {
            if let (Some(param), Some(reg)) = (name.strip_prefix('$'), self.reg_alloc.get_var(name)) {
                self.type_scope.insert(param.to_string(), TypeTerm::Reg(reg));
            }
        }
    }

    pub(crate) fn generic_fn_value(&mut self, name: &str, func_idx: usize, span: Span) -> Result<u8, (String, Span)> {
        let hidden = self.hidden_params[name].clone();
        let bound: Option<Vec<TypeTerm>> =
            self.types.type_args(span).and_then(|args| args.iter().map(Type::runtime_term).collect());
        let key = (name.to_string(), bound.clone());
        let code_idx = match self.generic_fn_wrappers.get(&key) {
            Some(&idx) => idx,
            None => {
                let (u, h) = (hidden.user_params, hidden.names.len());
                if 2 * u + 1 + h > 255 || func_idx > u16::MAX as usize {
                    return Err((format!("`{name}` has too many parameters to be used as a value"), span));
                }
                let window = u as u8;
                let mut insts: Vec<u32> = (0..u).map(|i| encode_r2(Opcode::MOVE, window + 1 + i as u8, i as u8)).collect();
                for j in 0..h {
                    let slot = window + 1 + (u + j) as u8;
                    insts.push(match bound.as_ref().and_then(|b| b.get(j)) {
                        Some(term) => encode_ri(Opcode::LOADTYPE, slot, self.term_idx(term.clone())),
                        None => encode_ri(Opcode::LOADK, slot, 0),
                    });
                }
                insts.push(encode_call(window, func_idx as u16));
                insts.push(encode_r2(Opcode::RET, window, 0));
                let code = CodeObject::new(insts, vec![Value::null()], (2 * u + 1 + h) as u16, u as u8);
                let idx = self.next_code_idx;
                self.next_code_idx += 1;
                self.pending_lambdas.push_back(PendingLambda::prebuilt(idx, code));
                self.generic_fn_wrappers.insert(key, idx);
                idx
            }
        };
        self.emit_function_value(code_idx, &[], span)
    }
}
