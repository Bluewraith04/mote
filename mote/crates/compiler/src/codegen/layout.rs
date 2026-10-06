//! Struct layout: a field of plain struct type is embedded in its parent's slots.

use super::*;

const MAX_SLOTS: usize = 250;

pub(crate) struct FieldTarget {
    pub root: u8,
    pub slot: Option<u8>,
    pub embedded: Option<(u16, usize)>,
}

struct Decl<'a> {
    fields: &'a [FieldDecl],
    generic: HashSet<&'a str>,
    is_struct: bool,
}

impl CodeGenerator {
    pub(crate) fn layout_inline_fields(&mut self, groups: &[&[Item]]) {
        let mut decls: HashMap<u16, Decl> = HashMap::new();
        for item in groups.iter().flat_map(|g| g.iter()) {
            let (name, fields, params, is_struct) = match item {
                Item::Struct(s) => (&s.name, &s.fields, &s.generic_params, true),
                Item::Class(c) => (&c.name, &c.fields, &c.generic_params, false),
                _ => continue,
            };
            if let Some(&idx) = self.type_map.get(name) {
                let generic = params.iter().map(|p| p.name.as_str()).collect();
                decls.insert(idx, Decl { fields, generic, is_struct });
            }
        }
        let mut embedded: Vec<Vec<Option<u16>>> = vec![Vec::new(); self.type_descriptors.len()];
        let mut done = vec![false; self.type_descriptors.len()];
        let mut open = HashSet::new();
        let indices: Vec<u16> = decls.keys().copied().collect();
        for idx in indices {
            self.place_fields(idx, &decls, &mut embedded, &mut done, &mut open);
        }
        let ids: Vec<Vec<Option<u32>>> = (0..self.type_descriptors.len())
            .map(|t| match embedded[t].is_empty() {
                true => vec![None; self.type_descriptors[t].fields.len()],
                false => embedded[t].iter().map(|n| n.map(u32::from)).collect(),
            })
            .collect();
        isa::value::link_inline_fields(&mut self.type_descriptors, &ids);
        self.window_types.clear();
        for item in groups.iter().flat_map(|g| g.iter()) {
            let Item::Struct(s) = item else { continue };
            let Some(&idx) = self.type_map.get(&s.name) else { continue };
            let desc = &self.type_descriptors[idx as usize];
            let flat = s.generic_params.is_empty() && desc.fields.iter().all(|f| f.inline.is_none());
            if flat && !desc.fields.is_empty() && desc.slots as usize <= super::window::MAX_WINDOW_SLOTS {
                self.window_types.insert(s.name.clone(), (idx, desc.slots as usize));
            }
        }
    }

    fn embedded_field(&self, object: &Expr, member: &str) -> Option<(u16, usize)> {
        let owner = *self.type_map.get(self.static_class_name(object)?)? as usize;
        let nested = self.type_descriptors[owner].fields.iter().find(|f| f.name.as_deref() == Some(member))?.inline.as_ref()?;
        Some((nested.id as u16, nested.slots as usize))
    }

    fn embedded_base<'e>(&self, e: &'e Expr) -> Option<(&'e Expr, usize)> {
        let Expr::MemberAccess { object, member, .. } = e else { return None };
        self.embedded_field(object, member)?;
        let (root, base) = self.embedded_base(object).unwrap_or((object, 0));
        Some((root, base + self.field_index(object, member)? as usize))
    }

    pub(crate) fn field_target(&mut self, object: &Expr, member: &str) -> Result<FieldTarget, (String, Span)> {
        if let Ok(n) = member.parse::<u8>() {
            return Ok(FieldTarget { root: self.compile_expr(object)?, slot: Some(n), embedded: None });
        }
        let (root_expr, base) = self.embedded_base(object).unwrap_or((object, 0));
        let slot = self.field_index(object, member).map(|s| s as usize + base);
        let embedded = self.embedded_field(object, member);
        Ok(FieldTarget { root: self.compile_expr(root_expr)?, slot: slot.map(|s| s as u8), embedded })
    }

    pub(crate) fn emit_read_target(&mut self, t: &FieldTarget, member: &str, span: Span) -> u8 {
        match (t.slot, t.embedded) {
            (Some(slot), Some((nested, size))) => {
                let dest = self.reg_alloc.alloc_temp();
                self.emit_new_object(dest, nested, span);
                self.copy_slots(t.root, slot as usize, dest, 0, size);
                dest
            }
            (Some(slot), None) => {
                let dest = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r3(Opcode::GETFIELD, dest, t.root, slot));
                dest
            }
            (None, _) => self.emit_named_field(t.root, member, None),
        }
    }

    pub(crate) fn emit_write_target(&mut self, t: &FieldTarget, member: &str, val: u8) {
        match (t.slot, t.embedded) {
            (Some(slot), Some((_, size))) => self.copy_slots(val, 0, t.root, slot as usize, size),
            (Some(slot), None) => self.current_insts.push(encode_r3(Opcode::SETFIELD, t.root, slot, val)),
            (None, _) => {
                let d = self.emit_named_field(t.root, member, Some(val));
                self.reg_alloc.free_temp(d);
            }
        }
    }

    pub(crate) fn copy_slots(&mut self, src: u8, from: usize, dst: u8, to: usize, count: usize) {
        let t = self.reg_alloc.alloc_temp();
        for k in 0..count {
            self.current_insts.push(encode_r3(Opcode::GETFIELD, t, src, (from + k) as u8));
            self.current_insts.push(encode_r3(Opcode::SETFIELD, dst, (to + k) as u8, t));
        }
        self.reg_alloc.free_temp(t);
    }

    pub(crate) fn emit_literal_fields(&mut self, dest: u8, base: usize, type_idx: u16, fields: &[(String, Expr)]) -> Result<(), (String, Span)> {
        for (i, (name, value)) in fields.iter().enumerate() {
            let slot = base + self.field_slot(type_idx, name, i) as usize;
            let embedded = self.type_descriptors[type_idx as usize]
                .fields
                .iter()
                .find(|f| f.name.as_deref() == Some(name.as_str()))
                .and_then(|f| f.inline.as_ref())
                .map(|n| (n.id as u16, n.slots as usize));
            match (embedded, value) {
                (Some((nested, _)), Expr::StructInit { name: init, fields: inner, .. }) if self.struct_init_target(init) == Some(nested) => {
                    self.emit_literal_fields(dest, slot, nested, inner)?;
                }
                (Some((_, size)), _) => {
                    let v = self.compile_value(value)?;
                    self.copy_slots(v, 0, dest, slot, size);
                    self.reg_alloc.free_temp(v);
                }
                (None, _) => {
                    let v = self.compile_value(value)?;
                    self.current_insts.push(encode_r3(Opcode::SETFIELD, dest, slot as u8, v));
                    self.reg_alloc.free_temp(v);
                }
            }
        }
        Ok(())
    }

    fn call_key(&self, callee: &Expr) -> Option<(String, bool)> {
        match callee {
            Expr::Ident(n, _) if self.func_map.contains_key(n) => Some((n.clone(), false)),
            Expr::MemberAccess { object, member, .. } => {
                let class = match self.static_class_name(object) {
                    Some(c) => c.to_string(),
                    None => match object.as_ref() {
                        Expr::Ident(c, _) if self.reg_alloc.get_var(c).is_none() => c.clone(),
                        _ => return None,
                    },
                };
                let key = Self::mangle_method_name(&class, member);
                self.func_map.contains_key(&key).then(|| {
                    let writes = self.var_self_methods.contains(&key);
                    (key, writes)
                })
            }
            _ => None,
        }
    }

    pub(crate) fn try_call_with_embedded_places(&mut self, expr: &Expr) -> Result<Option<u8>, (String, Span)> {
        let Expr::Call { callee, args, span, .. } = expr else { return Ok(None) };
        let Some((key, writes_self)) = self.call_key(callee) else { return Ok(None) };
        let var_args = self.var_args.get(&key).cloned().unwrap_or_default();
        let recv = match callee.as_ref() {
            Expr::MemberAccess { object, .. } if writes_self && self.reads_embedded(object) => Some(object.as_ref()),
            _ => None,
        };
        let embedded_args: Vec<usize> = var_args.into_iter().filter(|&i| args.get(i).is_some_and(|a| self.reads_embedded(a))).collect();
        if recv.is_none() && embedded_args.is_empty() {
            return Ok(None);
        }
        self.reg_alloc.enter_scope();
        let mut places: Vec<(FieldTarget, String, u8)> = Vec::new();
        let mut hold = |this: &mut Self, place: &Expr, name: String| -> Result<Expr, (String, Span)> {
            let Expr::MemberAccess { object, member, span: place_span } = place else { unreachable!("an embedded read is a member access") };
            let target = this.field_target(object, member)?;
            let copy = this.emit_read_target(&target, member, *place_span);
            this.reg_alloc.alias_var(&name, copy);
            places.push((target, member.clone(), copy));
            Ok(Expr::Ident(name, place.span()))
        };
        let mut new_callee = callee.as_ref().clone();
        if let (Some(place), Expr::MemberAccess { object, .. }) = (recv, &mut new_callee) {
            **object = hold(self, place, "$recv".to_string())?;
        }
        let mut new_args = args.clone();
        for &i in &embedded_args {
            new_args[i] = hold(self, &args[i], format!("$arg{i}"))?;
        }
        let call = Expr::Call { callee: Box::new(new_callee), args: new_args, names: Vec::new(), span: *span };
        let result = self.compile_expr(&call);
        self.reg_alloc.exit_scope();
        let dest = result?;
        for (target, member, copy) in places {
            self.emit_write_target(&target, &member, copy);
            self.reg_alloc.free_temp(copy);
            self.reg_alloc.free_temp(target.root);
        }
        Ok(Some(dest))
    }

    pub(crate) fn reads_embedded(&self, e: &Expr) -> bool {
        matches!(e, Expr::MemberAccess { object, member, .. } if self.embedded_field(object, member).is_some())
    }

    fn embeddable(&self, ty: &TypeNode, parent: &Decl, decls: &HashMap<u16, Decl>) -> Option<u16> {
        let TypeNode::Named(name, _) = ty else { return None };
        if parent.generic.contains(name.as_str()) {
            return None;
        }
        let idx = *self.type_map.get(name)?;
        let nested = decls.get(&idx)?;
        (nested.is_struct && nested.generic.is_empty()).then_some(idx)
    }

    fn place_fields(
        &mut self,
        idx: u16,
        decls: &HashMap<u16, Decl>,
        embedded: &mut Vec<Vec<Option<u16>>>,
        done: &mut Vec<bool>,
        open: &mut HashSet<u16>,
    ) -> usize {
        let i = idx as usize;
        if done[i] {
            return self.type_descriptors[i].slots as usize;
        }
        let Some(decl) = decls.get(&idx) else { return self.type_descriptors[i].slots as usize };
        open.insert(idx);
        let mut slot = 0;
        let mut plan = Vec::with_capacity(decl.fields.len());
        for (j, field) in decl.fields.iter().enumerate() {
            let nested = self.embeddable(&field.ty, decl, decls).filter(|n| !open.contains(n));
            let size = nested.map(|n| self.place_fields(n, decls, embedded, done, open));
            let fits = size.is_some_and(|s| slot + s <= MAX_SLOTS);
            self.type_descriptors[i].fields[j].slot = slot as u32;
            plan.push(nested.filter(|_| fits));
            slot += if fits { size.unwrap_or(1) } else { 1 };
        }
        open.remove(&idx);
        self.type_descriptors[i].slots = slot as u32;
        embedded[i] = plan;
        done[i] = true;
        slot
    }
}
