//! Match lowering and pattern tests.

use super::*;

impl CodeGenerator {
    pub(crate) fn lower_match(
        &mut self,
        subject: &Expr,
        arms: &[MatchArm],
        span: Span,
    ) -> Result<(), (String, Span)> {
        let subj_reg = self.compile_expr(subject)?;
        let mut end_sites: Vec<usize> = Vec::new();
        let outer_bind_copies = self.bind_copies;
        self.bind_copies = may_hold_value_object(self.types.of(subject));
        let outer_enum = self.match_enum.take();
        if let Some(Type::Enum { name, .. }) = self.types.of(subject) {
            self.match_enum = Some(name.clone());
        }

        for arm in arms {
            self.reg_alloc.enter_scope();

            let mut skip_sites: Vec<(usize, u8)> = Vec::new();
            if let Some(test) = self.compile_pattern_test(&arm.pattern, subj_reg, span)? {
                let site = self.current_insts.len();
                self.current_insts.push(0);
                self.reg_alloc.free_temp(test);
                skip_sites.push((site, test));
            }
            self.bind_pattern(&arm.pattern, subj_reg)?;

            if let Some(guard) = &arm.guard {
                let g = self.compile_expr(guard)?;
                let site = self.current_insts.len();
                self.current_insts.push(0);
                self.reg_alloc.free_temp(g);
                skip_sites.push((site, g));
            }

            for st in &arm.body {
                self.compile_stmt(st)?;
            }

            self.leave_scope();

            let end_site = self.current_insts.len();
            self.current_insts.push(0);
            end_sites.push(end_site);

            let next_arm = self.current_insts.len() as i32;
            for (site, reg) in skip_sites {
                self.current_insts[site] =
                    encode_jc(Opcode::JMPIFNOT, reg, (next_arm - site as i32) as i16);
            }
        }

        let end = self.current_insts.len() as i32;
        self.patch_jumps(&end_sites, end);
        self.reg_alloc.free_temp(subj_reg);
        self.bind_copies = outer_bind_copies;
        self.match_enum = outer_enum;
        Ok(())
    }

    pub(crate) fn bind_pattern(&mut self, pat: &Pattern, subj_reg: u8) -> Result<(), (String, Span)> {
        match pat {
            Pattern::Identifier { name, subpattern, .. } => {
                let is_unit_variant = self.bare_variant(name).is_some_and(|v| v.arity == 0);
                if !is_unit_variant {
                    if self.bind_copies {
                        let r = self.reg_alloc.alloc_var(name);
                        self.current_insts.push(encode_r2(Opcode::COPYVAL, r, subj_reg));
                    } else {
                        self.reg_alloc.alias_var(name, subj_reg);
                    }
                }
                if let Some(sub) = subpattern {
                    self.bind_pattern(sub, subj_reg)?;
                }
            }
            Pattern::Enum { payload: EnumPatternPayload::Tuple(subs), span, .. } => {
                let option = self.pattern_variant_info(pat).is_some_and(|i| i.is_option());
                for (i, sub) in subs.iter().enumerate() {
                    match sub {
                        Pattern::Wildcard(_) => {}
                        Pattern::Identifier { name, subpattern: None, .. } => {
                            let r = self.reg_alloc.alloc_var(name);
                            if option {
                                self.current_insts.push(encode_r2(Opcode::UNSOME, r, subj_reg));
                            } else {
                                self.current_insts.push(encode_r3(Opcode::GETFIELD, r, subj_reg, i as u8));
                            }
                            if self.bind_copies {
                                self.current_insts.push(encode_r2(Opcode::COPYVAL, r, r));
                            }
                        }
                        _ => {
                            return Err((
                                "a variant payload accepts only identifier or `_` sub-patterns for now"
                                    .into(),
                                *span,
                            ));
                        }
                    }
                }
            }
            Pattern::Enum { payload: EnumPatternPayload::Struct { fields, .. }, span, .. } => {
                let type_idx = self.pattern_variant_type_idx(pat, *span)?;
                for f in fields {
                    let bind_as = match &f.pattern {
                        None => &f.name,
                        Some(Pattern::Wildcard(_)) => continue,
                        Some(Pattern::Identifier { name, subpattern: None, .. }) => name,
                        Some(_) => {
                            return Err(("a variant field accepts only a name, `field: name` or `field: _` for now".into(), f.span));
                        }
                    };
                    let slot = self.field_slot(type_idx, &f.name, 0);
                    let r = self.reg_alloc.alloc_var(bind_as);
                    self.current_insts.push(encode_r3(Opcode::GETFIELD, r, subj_reg, slot));
                    if self.bind_copies {
                        self.current_insts.push(encode_r2(Opcode::COPYVAL, r, r));
                    }
                }
            }
            Pattern::Enum { payload: EnumPatternPayload::None, .. } => {}
            Pattern::Tuple(subs, _) => {
                for (i, sub) in subs.iter().enumerate() {
                    match sub {
                        Pattern::Wildcard(_) | Pattern::Literal(..) => {}
                        _ => {
                            let r = self.reg_alloc.alloc_temp();
                            self.current_insts.push(encode_r3(Opcode::GETFIELD, r, subj_reg, i as u8));
                            if self.bind_copies {
                                self.current_insts.push(encode_r2(Opcode::COPYVAL, r, r));
                            }
                            self.bind_pattern(sub, r)?;
                            if !matches!(sub, Pattern::Identifier { .. }) {
                                self.reg_alloc.free_temp(r);
                            }
                        }
                    }
                }
            }
            Pattern::Or(alts, _) => {
                for a in alts {
                    self.bind_pattern(a, subj_reg)?;
                }
            }
            Pattern::Type { name: Some(name), .. } => {
                if self.bind_copies {
                    let r = self.reg_alloc.alloc_var(name);
                    self.current_insts.push(encode_r2(Opcode::COPYVAL, r, subj_reg));
                } else {
                    self.reg_alloc.alias_var(name, subj_reg);
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn emit_typeof_eq(&mut self, subj_reg: u8, type_idx: u16) -> Option<u8> {
        let t = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r2(Opcode::TYPEOF, t, subj_reg));
        let k = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::LOADI, k, type_idx));
        let dest = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(Opcode::EQ, dest, t, k));
        self.reg_alloc.free_temp(k);
        self.reg_alloc.free_temp(t);
        Some(dest)
    }

    pub(crate) fn pattern_variant_info(&self, pat: &Pattern) -> Option<VariantInfo> {
        let Pattern::Enum { target: TypeNode::Named(head, _), variant, .. } = pat else { return None };
        match variant {
            Some(v) => self.enum_variant_map.get(&(head.clone(), v.clone())),
            None => self.bare_variant(head),
        }
        .cloned()
    }

    fn bare_variant(&self, name: &str) -> Option<&VariantInfo> {
        let own = self.match_enum.as_ref().and_then(|en| self.enum_variant_map.get(&(en.clone(), name.to_string())));
        own.or_else(|| self.variant_map.get(name))
    }

    pub(crate) fn pattern_variant_type_idx(&self, pat: &Pattern, span: Span) -> Result<u16, (String, Span)> {
        self.pattern_variant_info(pat).map(|i| i.type_idx).ok_or_else(|| match pat {
            Pattern::Enum { target: TypeNode::Named(head, _), .. } => (format!("unknown variant in pattern: '{head}'"), span),
            _ => ("not a variant pattern".into(), span),
        })
    }

    pub(crate) fn compile_pattern_test(
        &mut self,
        pat: &Pattern,
        subj_reg: u8,
        span: Span,
    ) -> Result<Option<u8>, (String, Span)> {
        match pat {
            Pattern::Wildcard(_) => Ok(None),
            Pattern::Identifier { subpattern: None, name, .. } => {
                if let Some(info) = self.bare_variant(name).cloned() {
                    if info.arity == 0 && info.is_option() {
                        return Ok(Some(self.emit_none_test(subj_reg, true)));
                    }
                    if info.arity == 0 {
                        return Ok(self.emit_typeof_eq(subj_reg, info.type_idx));
                    }
                }
                Ok(None)
            }
            Pattern::Identifier { subpattern: Some(sub), .. } => {
                self.compile_pattern_test(sub, subj_reg, span)
            }
            Pattern::Literal(lit, _) => {
                let lit_reg = self.compile_expr(lit)?;
                let dest = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r3(Opcode::EQ, dest, subj_reg, lit_reg));
                self.reg_alloc.free_temp(lit_reg);
                Ok(Some(dest))
            }
            Pattern::Or(alts, _) => {
                let mut acc: Option<u8> = None;
                for alt in alts {
                    match self.compile_pattern_test(alt, subj_reg, span)? {
                        None => {
                            if let Some(a) = acc {
                                self.reg_alloc.free_temp(a);
                            }
                            return Ok(None);
                        }
                        Some(t) => {
                            acc = Some(match acc {
                                None => t,
                                Some(a) => {
                                    let dest = self.reg_alloc.alloc_temp();
                                    self.current_insts.push(encode_r3(Opcode::OR, dest, a, t));
                                    self.reg_alloc.free_temp(t);
                                    self.reg_alloc.free_temp(a);
                                    dest
                                }
                            });
                        }
                    }
                }
                Ok(acc)
            }
            Pattern::Range { start, end, inclusive, .. } => {
                let lo = self.pattern_bound_reg(start, span)?;
                let hi = self.pattern_bound_reg(end, span)?;
                let lo_ok = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r3(Opcode::GE, lo_ok, subj_reg, lo));
                let hi_ok = self.reg_alloc.alloc_temp();
                let hi_op = if *inclusive { Opcode::LE } else { Opcode::LT };
                self.current_insts.push(encode_r3(hi_op, hi_ok, subj_reg, hi));
                let dest = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r3(Opcode::AND, dest, lo_ok, hi_ok));
                self.reg_alloc.free_temp(hi_ok);
                self.reg_alloc.free_temp(lo_ok);
                self.reg_alloc.free_temp(hi);
                self.reg_alloc.free_temp(lo);
                Ok(Some(dest))
            }
            Pattern::Tuple(subs, _) => {
                let mut acc: Option<u8> = None;
                for (i, sub) in subs.iter().enumerate() {
                    let elem = self.reg_alloc.alloc_temp();
                    self.current_insts.push(encode_r3(Opcode::GETFIELD, elem, subj_reg, i as u8));
                    let sub_test = self.compile_pattern_test(sub, elem, span)?;
                    self.reg_alloc.free_temp(elem);
                    if let Some(t) = sub_test {
                        acc = Some(match acc {
                            None => t,
                            Some(a) => {
                                let dest = self.reg_alloc.alloc_temp();
                                self.current_insts.push(encode_r3(Opcode::AND, dest, a, t));
                                self.reg_alloc.free_temp(t);
                                self.reg_alloc.free_temp(a);
                                dest
                            }
                        });
                    }
                }
                Ok(acc)
            }
            Pattern::Type { span: p_span, .. } => {
                let target = self.types.type_test(*p_span).cloned();
                let t = self.reg_alloc.alloc_temp();
                self.load_type_into(t, target.as_ref());
                let dest = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r3(Opcode::ISTYPE, dest, subj_reg, t));
                self.reg_alloc.free_temp(t);
                Ok(Some(dest))
            }
            Pattern::Struct { .. } => {
                Err(("struct patterns are not supported by codegen yet".into(), span))
            }
            Pattern::Enum { .. } => {
                if let Some(info) = self.pattern_variant_info(pat).filter(VariantInfo::is_option) {
                    return Ok(Some(self.emit_none_test(subj_reg, info.arity == 0)));
                }
                let type_idx = self.pattern_variant_type_idx(pat, span)?;
                Ok(self.emit_typeof_eq(subj_reg, type_idx))
            }
        }
    }

    pub(crate) fn pattern_bound_reg(&mut self, pat: &Pattern, span: Span) -> Result<u8, (String, Span)> {
        match pat {
            Pattern::Literal(lit, _) => self.compile_expr(lit),
            _ => Err(("range-pattern bounds must be literals".into(), span)),
        }
    }
}
