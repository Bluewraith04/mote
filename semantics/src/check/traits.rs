use crate::cx::Cx;
use crate::defs::{Bound, Def, DefId, DefKind, ExtendId, Function, GenericParam, Param};
use crate::types::{ConstArg, GenericArg, TyId, TyKind};

impl Cx<'_> {
    /// Checks the traits each type or `extend` block lists: its methods exist and match, and it gets the defaults it lacks.
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
                self.list_trait_owner(owner, &listed);
            }
        }

        let extends: Vec<(ExtendId, Vec<Bound>)> = self
            .program
            .extends
            .iter()
            .filter(|&(_, ext)| !ext.traits.is_empty() && ext.target != TyId::ERROR)
            .map(|(id, ext)| (id, ext.traits.clone()))
            .collect();
        for (ext_id, traits) in extends {
            for listed in traits {
                self.list_trait_extend(ext_id, &listed);
            }
        }
    }

    fn list_trait_owner(&mut self, owner: DefId, listed: &Bound) {
        let DefKind::Trait(item) = &self.program.defs[listed.def].kind else { return };
        let methods = item.methods.clone();
        let trait_names: Vec<_> = item.generics.iter().map(|param| param.name()).zip(listed.args.iter().copied()).collect();
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
                None if self.program.defs[method].function().is_some_and(|function| function.has_body) => {
                    self.copy_default(Some(owner), None, method, self_ty, listed, &trait_names, &owner_args);
                }
                None => {
                    let span = self.program.defs[owner].span;
                    self.error(span, format!("`{type_name}` has no `{text}`, which `{trait_name}` requires"));
                }
            }
        }
    }

    fn list_trait_extend(&mut self, ext_id: ExtendId, listed: &Bound) {
        let DefKind::Trait(item) = &self.program.defs[listed.def].kind else { return };
        let methods = item.methods.clone();
        let trait_names: Vec<_> = item.generics.iter().map(|param| param.name()).zip(listed.args.iter().copied()).collect();
        let (target, generics, span) = {
            let ext = &self.program.extends[ext_id];
            (ext.target, ext.generics.clone(), ext.span)
        };
        let owner_args: Vec<GenericArg> = generics
            .iter()
            .enumerate()
            .map(|(index, param)| match param {
                GenericParam::Type { name, .. } => GenericArg::Type(self.program.types.param(index as u32, *name)),
                GenericParam::Const { name, .. } => GenericArg::Const(ConstArg::Param { index: index as u32, name: *name }),
            })
            .collect();
        let type_name = self.show_ty(target);
        let trait_name = self.show_bound(listed);
        for method in methods {
            let name = self.program.defs[method].name;
            let text = self.text(name);
            let direct = self.program.extend_methods_named(ext_id, name).first().copied();
            let found = direct.map(|d| (d, owner_args.clone())).or_else(|| {
                self.program
                    .method_owner(target)
                    .and_then(|(owner_def, def_args)| self.methods_named(owner_def, name).first().copied().map(|m| (m, def_args)))
            });
            match found {
                Some((cand, cand_args)) => {
                    self.program.defs[cand].is_pub = true;
                    if !self.same_signature(method, target, &listed.args, cand, &cand_args) {
                        let cspan = self.program.defs[cand].span;
                        self.error(cspan, format!("`{type_name}.{text}` does not match `{trait_name}.{text}`"));
                    }
                }
                None if self.program.defs[method].function().is_some_and(|function| function.has_body) => {
                    self.copy_default(None, Some(ext_id), method, target, listed, &trait_names, &owner_args);
                }
                None => {
                    self.error(span, format!("`{type_name}` has no `{text}`, which `{trait_name}` requires"));
                }
            }
        }
    }

    /// Gives the type or `extend` block its own copy of a trait's default method.
    #[allow(clippy::too_many_arguments)]
    fn copy_default(
        &mut self,
        owner: Option<DefId>,
        extend: Option<ExtendId>,
        method: DefId,
        self_ty: TyId,
        listed: &Bound,
        trait_names: &[(mote_base::Symbol, TyId)],
        owner_args: &[GenericArg],
    ) {
        let (generics, params, receiver, ret) = {
            let function = self.program.defs[method].function().expect("a method");
            (function.generics.clone(), function.params.clone(), function.receiver, function.ret)
        };
        let owner_count = owner_args.len();
        let mut args: Vec<GenericArg> = std::iter::once(GenericArg::Type(self_ty))
            .chain(listed.args.iter().copied().map(GenericArg::Type))
            .collect();
        for (index, param) in generics.iter().enumerate() {
            match param {
                GenericParam::Type { name, .. } => {
                    args.push(GenericArg::Type(self.program.types.param((owner_count + index) as u32, *name)));
                }
                GenericParam::Const { name, .. } => {
                    args.push(GenericArg::Const(ConstArg::Param { index: (owner_count + index) as u32, name: *name }));
                }
            }
        }
        let generics: Vec<GenericParam> = generics
            .into_iter()
            .map(|param| match param {
                GenericParam::Type { name, bounds } => GenericParam::Type {
                    name,
                    bounds: bounds
                        .into_iter()
                        .map(|bound| Bound {
                            def: bound.def,
                            args: bound.args.iter().map(|&arg| self.program.types.subst_generic(arg, &args)).collect(),
                        })
                        .collect(),
                },
                GenericParam::Const { name, ty } => GenericParam::Const {
                    name,
                    ty: self.program.types.subst_generic(ty, &args),
                },
            })
            .collect();
        let params: Vec<Param> = params
            .into_iter()
            .map(|param| Param { ty: self.program.types.subst_generic(param.ty, &args), ..param })
            .collect();
        let ret = self.program.types.subst_generic(ret, &args);
        let (name, module, span, origin) = {
            let def = &self.program.defs[method];
            (def.name, def.module, def.span, def.origin)
        };
        let function = Function { generics, params, receiver, ret, has_body: true, link: None };
        let copy = self.program.defs.alloc(Def {
            name,
            module,
            span,
            is_pub: true,
            owner,
            extend,
            origin,
            doc: None,
            kind: DefKind::Function(function),
        });
        self.program.listed_defaults.insert(copy, listed.def);
        if !trait_names.is_empty() {
            self.trait_names.insert(copy, trait_names.to_vec());
        }
        if let Some(owner_id) = owner {
            match &mut self.program.defs[owner_id].kind {
                DefKind::Record(record) => record.methods.push(copy),
                DefKind::Enum(en) => en.methods.push(copy),
                _ => {}
            }
        } else if let Some(ext_id) = extend {
            self.program.extends[ext_id].methods.push(copy);
        }
    }
}
