use crate::cx::Cx;
use crate::defs::{Bound, Def, DefId, DefKind, Function, GenericParam, Param};
use crate::types::{TyId, TyKind};

impl Cx<'_> {
    /// Checks the traits each type lists: its methods exist and match, and it gets the defaults it lacks.
    pub(crate) fn check_listed_traits(&mut self) {
        let owners: Vec<(DefId, Vec<Bound>)> = self
            .program
            .defs
            .iter()
            .filter_map(|(id, def)| match &def.kind {
                DefKind::Record(record) if !record.traits.is_empty() => Some((id, record.traits.clone())),
                DefKind::Enum(en) if !en.traits.is_empty() => Some((id, en.traits.clone())),
                _ => None,
            })
            .collect();
        for (owner, traits) in owners {
            for listed in traits {
                self.list_trait(owner, &listed);
            }
        }
    }

    fn list_trait(&mut self, owner: DefId, listed: &Bound) {
        let DefKind::Trait(item) = &self.program.defs[listed.def].kind else { return };
        let methods = item.methods.clone();
        let trait_names: Vec<_> = item.generics.iter().map(|param| param.name).zip(listed.args.iter().copied()).collect();
        let Some(self_ty) = self.scope_for(owner).self_ty else { return };
        let owner_args = match self.program.types.kind(self_ty) {
            TyKind::Adt { args, .. } => args.clone(),
            _ => Vec::new(),
        };
        let type_name = self.text(self.program.defs[owner].name);
        let trait_name = self.show_bound(listed);
        for method in methods {
            let name = self.program.defs[method].name;
            let text = self.text(name);
            match self.methods_named(owner, name).first().copied() {
                Some(found) => {
                    self.program.defs[found].is_pub = true;
                    if !self.same_signature(method, self_ty, &listed.args, found, &owner_args) {
                        let span = self.program.defs[found].span;
                        self.error(span, format!("`{type_name}.{text}` does not match `{trait_name}.{text}`"));
                    }
                }
                None if self.program.defs[method].function().is_some_and(|function| function.has_body) => self.copy_default(owner, method, self_ty, listed, &trait_names, owner_args.len()),
                None => {
                    let span = self.program.defs[owner].span;
                    self.error(span, format!("`{type_name}` has no `{text}`, which `{trait_name}` requires"));
                }
            }
        }
    }

    /// Gives the type its own copy of a trait's default method, with `Self` the type and the trait's parameters its arguments.
    fn copy_default(&mut self, owner: DefId, method: DefId, self_ty: TyId, listed: &Bound, trait_names: &[(mote_base::Symbol, TyId)], owner_count: usize) {
        let (generics, params, receiver, ret) = {
            let function = self.program.defs[method].function().expect("a method");
            (function.generics.clone(), function.params.clone(), function.receiver, function.ret)
        };
        let mut args = vec![self_ty];
        args.extend(listed.args.iter().copied());
        for (index, param) in generics.iter().enumerate() {
            args.push(self.program.types.param((owner_count + index) as u32, param.name));
        }
        let generics: Vec<GenericParam> = generics
            .into_iter()
            .map(|param| GenericParam {
                bounds: param.bounds.into_iter().map(|bound| Bound { def: bound.def, args: bound.args.iter().map(|&arg| self.program.types.subst(arg, &args)).collect() }).collect(),
                ..param
            })
            .collect();
        let params: Vec<Param> = params.into_iter().map(|param| Param { ty: self.program.types.subst(param.ty, &args), ..param }).collect();
        let ret = self.program.types.subst(ret, &args);
        let (name, module, span, origin) = {
            let def = &self.program.defs[method];
            (def.name, def.module, def.span, def.origin)
        };
        let function = Function { generics, params, receiver, ret, has_body: true, native: false, link: None, intrinsic: None };
        let copy = self.program.defs.alloc(Def { name, module, span, is_pub: true, lang: None, owner: Some(owner), origin, kind: DefKind::Function(function) });
        self.program.listed_defaults.insert(copy, listed.def);
        if !trait_names.is_empty() {
            self.trait_names.insert(copy, trait_names.to_vec());
        }
        match &mut self.program.defs[owner].kind {
            DefKind::Record(record) => record.methods.push(copy),
            DefKind::Enum(en) => en.methods.push(copy),
            _ => {}
        }
    }
}
