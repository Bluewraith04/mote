//! Optionals as values: `None` is `null`, `Some(v)` is `v` or a `Some` cell.

use super::*;

#[derive(Clone)]
pub(crate) enum Fallible {
    Option,
    Tagged(Vec<u16>),
    Unknown,
}

pub(crate) fn lang_variants() -> impl Iterator<Item = (&'static str, VariantInfo)> {
    [("Some", 1), ("None", 0)]
        .into_iter()
        .map(|(v, arity)| (v, VariantInfo { type_idx: u16::MAX, arity, enum_name: "Option".to_string() }))
}

impl VariantInfo {
    pub(crate) fn is_option(&self) -> bool {
        self.enum_name == "Option"
    }
}

impl CodeGenerator {
    pub(crate) fn fallible_kind(&self, e: &Expr) -> Fallible {
        let variant_idx = |enum_name: &str, variant: &str| {
            self.enum_variant_map.get(&(enum_name.to_string(), variant.to_string())).map(|v| v.type_idx)
        };
        match self.types.of(e) {
            Some(Type::Nullable(_)) => Fallible::Option,
            Some(Type::Enum { name, .. }) => {
                Fallible::Tagged(["None", "Err"].iter().filter_map(|v| variant_idx(name, v)).collect())
            }
            Some(Type::Result(..)) => Fallible::Tagged(self.variant_idx("Err").into_iter().collect()),
            _ => Fallible::Unknown,
        }
    }

    fn variant_idx(&self, name: &str) -> Option<u16> {
        self.variant_map.get(name).map(|v| v.type_idx)
    }

    pub(crate) fn emit_null(&mut self) -> u8 {
        let dest = self.reg_alloc.alloc_temp();
        let k = self.add_constant(Value::null());
        self.current_insts.push(encode_ri(Opcode::LOADK, dest, k));
        dest
    }

    pub(crate) fn emit_none_test(&mut self, r: u8, want_none: bool) -> u8 {
        let null = self.emit_null();
        let dest = self.reg_alloc.alloc_temp();
        let op = if want_none { Opcode::EQ } else { Opcode::NE };
        self.current_insts.push(encode_r3(op, dest, r, null));
        self.reg_alloc.free_temp(null);
        dest
    }

    pub(crate) fn emit_some(&mut self, v: u8) -> u8 {
        let dest = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r2(Opcode::SOME, dest, v));
        dest
    }

    fn emit_descriptor_in(&mut self, r: u8, idxs: &[u16]) -> u8 {
        let t = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r2(Opcode::TYPEOF, t, r));
        let acc = self.reg_alloc.alloc_temp();
        let k = self.reg_alloc.alloc_temp();
        for (i, &idx) in idxs.iter().enumerate() {
            self.current_insts.push(encode_ri(Opcode::LOADI, k, idx));
            if i == 0 {
                self.current_insts.push(encode_r3(Opcode::EQ, acc, t, k));
            } else {
                self.current_insts.push(encode_r3(Opcode::EQ, k, t, k));
                self.current_insts.push(encode_r3(Opcode::OR, acc, acc, k));
            }
        }
        self.reg_alloc.free_temp(k);
        self.reg_alloc.free_temp(t);
        acc
    }

    pub(crate) fn emit_failure_test(&mut self, r: u8, kind: &Fallible, span: Span) -> Result<u8, (String, Span)> {
        let none_in_scope = || ("`?` / `!` need `Option` / `Result` in scope (the prelude enums)".to_string(), span);
        match kind {
            Fallible::Option => Ok(self.emit_none_test(r, true)),
            Fallible::Tagged(idxs) if idxs.is_empty() => Err(none_in_scope()),
            Fallible::Tagged(idxs) => Ok(self.emit_descriptor_in(r, idxs)),
            Fallible::Unknown => {
                let none = self.emit_none_test(r, true);
                if let Some(err) = self.variant_idx("Err") {
                    let is_err = self.emit_descriptor_in(r, &[err]);
                    self.current_insts.push(encode_r3(Opcode::OR, none, none, is_err));
                    self.reg_alloc.free_temp(is_err);
                }
                Ok(none)
            }
        }
    }

    pub(crate) fn emit_rewrap(&mut self, dest: u8, v: u8, r: u8, kind: &Fallible, succ: Option<u16>) {
        let tagged_ok = match (kind, succ) {
            (Fallible::Tagged(_), Some(idx)) => Some(idx),
            (Fallible::Unknown, _) => self.variant_idx("Ok"),
            _ => None,
        };
        let Some(ok) = tagged_ok else {
            self.current_insts.push(encode_r2(Opcode::SOME, dest, v));
            return;
        };
        let is_ok = match kind {
            Fallible::Unknown => Some(self.emit_descriptor_in(r, &[ok])),
            _ => None,
        };
        if let Some(t) = is_ok {
            self.current_insts.push(encode_jc(Opcode::JMPIFNOT, t, 4));
        }
        self.current_insts.push(encode_ri(Opcode::NEWOBJ, dest, ok));
        self.current_insts.push(encode_r3(Opcode::SETFIELD, dest, 0, v));
        if let Some(t) = is_ok {
            self.current_insts.push(encode_ju(Opcode::JMP, 2));
            self.current_insts.push(encode_r2(Opcode::SOME, dest, v));
            self.reg_alloc.free_temp(t);
        }
    }

    pub(crate) fn emit_payload(&mut self, dest: u8, r: u8, kind: &Fallible) {
        match kind {
            Fallible::Option => self.current_insts.push(encode_r2(Opcode::UNSOME, dest, r)),
            Fallible::Tagged(_) => self.current_insts.push(encode_r3(Opcode::GETFIELD, dest, r, 0)),
            Fallible::Unknown => {
                let Some(ok) = self.variant_idx("Ok") else {
                    self.current_insts.push(encode_r2(Opcode::UNSOME, dest, r));
                    return;
                };
                let err = self.variant_idx("Err").unwrap_or(ok);
                let tagged = self.emit_descriptor_in(r, &[ok, err]);
                self.current_insts.push(encode_jc(Opcode::JMPIFNOT, tagged, 3));
                self.current_insts.push(encode_r3(Opcode::GETFIELD, dest, r, 0));
                self.current_insts.push(encode_ju(Opcode::JMP, 2));
                self.current_insts.push(encode_r2(Opcode::UNSOME, dest, r));
                self.reg_alloc.free_temp(tagged);
            }
        }
    }
}
