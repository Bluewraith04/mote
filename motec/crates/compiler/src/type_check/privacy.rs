//! Module privacy: who may read a field, write one, call a method, build a literal, or name a type.

use super::*;

impl TypeChecker {
    /// Names the module the next `check_program` call checks.
    pub fn set_module(&mut self, module: &str) {
        self.module = module.to_string();
    }

    pub(super) fn note_type_privacy(&mut self, name: &str, is_pub: bool, fields: &[FieldDecl]) {
        self.type_module.insert(name.to_string(), self.module.clone());
        if is_pub {
            self.pub_types.insert(name.to_string());
        }
        for f in fields.iter().filter(|f| f.is_pub) {
            self.pub_fields.insert((name.to_string(), f.name.clone()));
        }
    }

    pub(super) fn check_pub_field_types(&mut self, owner: &str, decls: &[FieldDecl], fields: &[(String, Type)]) {
        if !self.pub_types.contains(owner) {
            return;
        }
        for d in decls.iter().filter(|d| d.is_pub) {
            if let Some((_, ty)) = fields.iter().find(|(n, _)| *n == d.name) {
                self.check_exposed(&format!("{owner}.{}", d.name), &[ty], d.span);
            }
        }
    }

    fn is_foreign(&self, ty: &str) -> bool {
        self.type_module.get(ty).is_some_and(|m| *m != self.module)
    }

    pub(super) fn check_field_read(&mut self, ty: &str, field: &str, span: Span) {
        if self.is_foreign(ty) && !self.pub_fields.contains(&(ty.to_string(), field.to_string())) {
            self.errors.push((format!("field `{field}` of `{ty}` is private; mark it `pub`"), span));
        }
    }

    pub(super) fn field_write_reason(&self, ty: &str, field: &str) -> Option<String> {
        self.is_foreign(ty).then(|| format!("field `{field}` of `{ty}` can only be written in its module; add a method"))
    }

    pub(super) fn check_method_visible(&mut self, ty: &str, method: &str, is_pub: bool, span: Span) {
        if !is_pub && self.is_foreign(ty) {
            self.errors.push((format!("method `{method}` of `{ty}` is private; mark it `pub`"), span));
        }
    }

    pub(super) fn check_literal_allowed(&mut self, ty: &str, span: Span) {
        if !self.is_foreign(ty) {
            return;
        }
        let mut ctors: Vec<String> = self
            .class_method_sigs
            .iter()
            .filter(|((t, _), s)| t == ty && s.is_static && s.is_pub)
            .map(|((_, m), _)| format!("`{ty}.{m}`"))
            .collect();
        ctors.sort();
        let hint = if ctors.is_empty() { format!("`{ty}` has no public constructor") } else { format!("use {}", ctors.join(" or ")) };
        self.errors.push((format!("`{ty} {{ … }}` can only be written in its module; {hint}"), span));
    }

    pub(super) fn check_exposed(&mut self, what: &str, tys: &[&Type], span: Span) {
        let mut names = Vec::new();
        tys.iter().for_each(|t| t.named_types(&mut names));
        names.sort();
        names.dedup();
        for n in names {
            let here = self.type_module.get(&n).is_some_and(|m| *m == self.module);
            if here && !self.pub_types.contains(&n) {
                self.errors.push((format!("`{what}` is `pub` but its type `{n}` is not; mark `{n}` `pub`"), span));
            }
        }
    }
}
