use mote_base::{Diagnostic, Id, Span, Symbol};
use mote_syntax as ast;
use mote_syntax::{BindingKind, ExprKind, ItemKind, Member, ParamKind, PatternKind, StmtKind, UnaryOp, VariantPayload};

use super::lower::Scope;
use crate::comptime;
use crate::cx::{Progress, Cx};
use crate::defs::{Bound, DefId, DefKind, ExtendId, Field, GenericParam, Param, Payload, Receiver, TyHead, Variant};
use crate::types::{ConstArg, ConstVal, GenericArg, IntTy, TyId, TyKind};

/// A function's signature as the syntax tree writes it.
pub(crate) struct FnAst<'a> {
    pub generics: &'a [ast::GenericParam],
    pub params: &'a [ast::Param],
    pub ret: Option<Id<ast::Type>>,
    pub body: Option<&'a ast::Block>,
}

fn generic_names(generics: &[GenericParam]) -> Vec<Symbol> {
    generics.iter().map(GenericParam::name).collect()
}

fn generic_const_flags(generics: &[GenericParam]) -> Vec<bool> {
    generics.iter().map(GenericParam::is_const).collect()
}

impl<'a> Cx<'a> {
    /// Resolves every definition's generics, then its types, then checks overloads.
    pub(super) fn signatures(&mut self) {
        let defs: Vec<DefId> = self.program.defs.iter().map(|(id, _)| id).collect();
        let extends: Vec<ExtendId> = self.program.extends.iter().map(|(id, _)| id).collect();
        for &def in &defs {
            self.lower_generic_names(def);
        }
        for &ext in &extends {
            self.lower_extend_generic_names(ext);
        }
        for &def in &defs {
            self.lower_bounds(def);
        }
        for &ext in &extends {
            self.lower_extend_header(ext);
        }
        for &def in &defs {
            match &self.program.defs[def].kind {
                DefKind::Function(_) => self.lower_function(def),
                DefKind::Record(_) => self.lower_record(def),
                DefKind::Enum(_) => self.lower_enum(def),
                DefKind::Alias(_) => self.ensure_alias(def),
                DefKind::Global(_) => self.lower_global(def),
                DefKind::Const(_) => {
                    let _ = self.ensure_const_def(def);
                }
                DefKind::Trait(_) | DefKind::Test => {}
            }
        }
        for &def in &defs {
            self.check_overloads(def);
        }
    }

    pub(crate) fn item_of(&self, def: DefId) -> Option<(&'a ast::Item, Option<u32>)> {
        let def = &self.program.defs[def];
        let origin = def.origin?;
        Some((&self.ast(def.module)[origin.item], origin.member))
    }

    pub(crate) fn fn_ast(&self, def: DefId) -> Option<FnAst<'a>> {
        let (item, member) = self.item_of(def)?;
        match (&item.kind, member) {
            (ItemKind::Function(f), None) => Some(FnAst { generics: &f.generics, params: &f.params, ret: f.ret, body: Some(&f.body) }),
            (ItemKind::ExternBlock(eb), Some(member)) => {
                let decl = &eb.decls[member as usize];
                Some(FnAst { generics: &[], params: &decl.params, ret: decl.ret, body: None })
            }
            (ItemKind::Extend(ext), Some(member)) => {
                let function = &ext.methods[member as usize].function;
                Some(FnAst { generics: &function.generics, params: &function.params, ret: function.ret, body: Some(&function.body) })
            }
            (ItemKind::Record(record), Some(member)) => match &record.members[member as usize] {
                Member::Method(method) => Some(FnAst { generics: &method.function.generics, params: &method.function.params, ret: method.function.ret, body: Some(&method.function.body) }),
                Member::Field(_) => None,
            },
            (ItemKind::Enum(en), Some(member)) => {
                let function = &en.methods[member as usize].function;
                Some(FnAst { generics: &function.generics, params: &function.params, ret: function.ret, body: Some(&function.body) })
            }
            (ItemKind::Trait(tr), Some(member)) => {
                let entry = &tr.members[member as usize];
                Some(FnAst { generics: &entry.generics, params: &entry.params, ret: entry.ret, body: entry.body.as_ref() })
            }
            _ => None,
        }
    }

    fn generics_ast(&self, def: DefId) -> &'a [ast::GenericParam] {
        match self.item_of(def) {
            Some((item, None)) => match &item.kind {
                ItemKind::Function(f) => &f.generics,
                ItemKind::Record(record) => &record.generics,
                ItemKind::Enum(en) => &en.generics,
                ItemKind::Trait(tr) => &tr.generics,
                ItemKind::TypeAlias(alias) => &alias.generics,
                _ => &[],
            },
            _ => match self.fn_ast(def) {
                Some(function) => function.generics,
                None => &[],
            },
        }
    }

    fn generics_mut(&mut self, def: DefId) -> Option<&mut Vec<GenericParam>> {
        match &mut self.program.defs[def].kind {
            DefKind::Function(function) => Some(&mut function.generics),
            DefKind::Record(record) => Some(&mut record.generics),
            DefKind::Enum(en) => Some(&mut en.generics),
            DefKind::Trait(item) => Some(&mut item.generics),
            DefKind::Alias(alias) => Some(&mut alias.generics),
            _ => None,
        }
    }

    fn ast_generic_name(param: &'a ast::GenericParam) -> ast::Ident {
        match param {
            ast::GenericParam::Type { name, .. } | ast::GenericParam::Const { name, .. } => *name,
        }
    }

    fn lower_generic_name_list(&mut self, written: &'a [ast::GenericParam]) -> Vec<GenericParam> {
        let mut lowered = Vec::new();
        for generic in written {
            let ident = Self::ast_generic_name(generic);
            if lowered.iter().any(|param: &GenericParam| param.name() == ident.name) {
                let message = format!("`{}` is defined twice", self.text(ident.name));
                self.error(ident.span, message);
                continue;
            }
            match generic {
                ast::GenericParam::Type { .. } => lowered.push(GenericParam::Type { name: ident.name, bounds: Vec::new() }),
                ast::GenericParam::Const { .. } => lowered.push(GenericParam::Const { name: ident.name, ty: TyId::INT }),
            }
        }
        lowered
    }

    /// A definition's generic parameter names, without bounds.
    fn lower_generic_names(&mut self, def: DefId) {
        if let Some(owner) = self.program.defs[def].owner
            && matches!(self.program.defs[owner].kind, DefKind::Trait(_))
        {
            let generics = self.generics_ast(def);
            if let Some(first) = generics.first() {
                self.error(Self::ast_generic_name(first).span, "trait methods cannot have type parameters");
            }
        }
        let lowered = self.lower_generic_name_list(self.generics_ast(def));
        if let Some(generics) = self.generics_mut(def) {
            *generics = lowered;
        }
    }

    fn lower_extend_generic_names(&mut self, ext: ExtendId) {
        let origin = self.program.extends[ext].origin;
        let module = self.program.extends[ext].module;
        let ItemKind::Extend(decl) = &self.ast(module)[origin.item].kind else { return };
        let lowered = self.lower_generic_name_list(&decl.generics);
        self.program.extends[ext].generics = lowered;
    }

    fn lower_generic_bounds_with(&mut self, scope: &Scope, written: &'a [ast::GenericParam], generics: &mut [GenericParam]) {
        for param in generics.iter_mut() {
            let name = param.name();
            let Some(ast_param) = written.iter().find(|g| Self::ast_generic_name(g).name == name) else {
                continue;
            };
            match (param, ast_param) {
                (GenericParam::Type { bounds, .. }, ast::GenericParam::Type { bounds: ast_bounds, .. }) => {
                    *bounds = ast_bounds.iter().filter_map(|&b| self.trait_ref(scope, b)).collect();
                }
                (GenericParam::Const { ty, .. }, ast::GenericParam::Const { ty: ast_ty, .. }) => {
                    let lowered_ty = self.lower_type(scope, *ast_ty);
                    if !matches!(self.program.types.kind(lowered_ty), TyKind::Error | TyKind::Int(_) | TyKind::Bool | TyKind::String) {
                        self.error(self.ast(scope.module)[*ast_ty].span, "`const` generic parameter type must be an integer, `Bool`, or `String`");
                    }
                    *ty = lowered_ty;
                }
                _ => {}
            }
        }
    }

    /// The bounds of a definition's type parameters and types of its const parameters.
    fn lower_bounds(&mut self, def: DefId) {
        let scope = self.scope_for(def);
        let written = self.generics_ast(def);
        let mut current = match &self.program.defs[def].kind {
            DefKind::Function(function) => function.generics.clone(),
            DefKind::Record(record) => record.generics.clone(),
            DefKind::Enum(en) => en.generics.clone(),
            DefKind::Trait(item) => item.generics.clone(),
            DefKind::Alias(alias) => alias.generics.clone(),
            _ => return,
        };
        self.lower_generic_bounds_with(&scope, written, &mut current);
        if let Some(generics) = self.generics_mut(def) {
            *generics = current;
        }
    }

    fn lower_extend_header(&mut self, ext: ExtendId) {
        let origin = self.program.extends[ext].origin;
        let module = self.program.extends[ext].module;
        let ItemKind::Extend(decl) = &self.ast(module)[origin.item].kind else { return };
        let mut generics = self.program.extends[ext].generics.clone();
        let params = generic_names(&generics);
        let const_params = generic_const_flags(&generics);
        let mut scope = Scope { module, params, const_params, self_ty: None, bound: Vec::new() };
        self.lower_generic_bounds_with(&scope, &decl.generics, &mut generics);
        let target = self.lower_type(&scope, decl.target);
        scope.self_ty = Some(target);
        let traits = self.lower_traits(&scope, &decl.traits);
        let extend_def = &mut self.program.extends[ext];
        extend_def.generics = generics;
        extend_def.target = target;
        extend_def.traits = traits;
        if let Some(head) = TyHead::of(&self.program.types, target) {
            self.program.extend_by_head.entry(head).or_default().push(ext);
        } else if target != TyId::ERROR {
            self.error(self.ast(module)[decl.target].span, "cannot `extend` this type");
        }
    }

    /// `Self` of a type: itself applied to its own parameters.
    fn self_type(&mut self, def: DefId, generics: &[GenericParam]) -> TyId {
        let args: Vec<GenericArg> = generics
            .iter()
            .enumerate()
            .map(|(index, param)| match param {
                GenericParam::Type { name, .. } => GenericArg::Type(self.program.types.param(index as u32, *name)),
                GenericParam::Const { name, .. } => GenericArg::Const(ConstArg::Param { index: index as u32, name: *name }),
            })
            .collect();
        self.program.types.adt(def, args)
    }

    /// The names a definition's types are written against.
    pub(crate) fn scope_for(&mut self, def: DefId) -> Scope {
        let module = self.program.defs[def].module;
        let mut params = Vec::new();
        let mut const_params = Vec::new();
        let mut self_ty = None;
        if let Some(ext) = self.program.defs[def].extend {
            let extend_def = &self.program.extends[ext];
            params = generic_names(&extend_def.generics);
            const_params = generic_const_flags(&extend_def.generics);
            self_ty = (extend_def.target != TyId::ERROR).then_some(extend_def.target);
        } else if let Some(owner) = self.program.defs[def].owner {
            match &self.program.defs[owner].kind {
                DefKind::Trait(item) => {
                    params.push(self.self_name);
                    const_params.push(false);
                    params.extend(generic_names(&item.generics));
                    const_params.extend(generic_const_flags(&item.generics));
                    self_ty = Some(self.program.types.param(0, self.self_name));
                }
                DefKind::Record(record) => {
                    let g = record.generics.clone();
                    params = generic_names(&g);
                    const_params = generic_const_flags(&g);
                    self_ty = Some(self.self_type(owner, &g));
                }
                DefKind::Enum(en) => {
                    let g = en.generics.clone();
                    params = generic_names(&g);
                    const_params = generic_const_flags(&g);
                    self_ty = Some(self.self_type(owner, &g));
                }
                _ => {}
            }
        }
        match &self.program.defs[def].kind {
            DefKind::Function(function) => {
                params.extend(generic_names(&function.generics));
                const_params.extend(generic_const_flags(&function.generics));
            }
            DefKind::Record(record) => {
                let g = record.generics.clone();
                params = generic_names(&g);
                const_params = generic_const_flags(&g);
                self_ty = Some(self.self_type(def, &g));
            }
            DefKind::Enum(en) => {
                let g = en.generics.clone();
                params = generic_names(&g);
                const_params = generic_const_flags(&g);
                self_ty = Some(self.self_type(def, &g));
            }
            DefKind::Alias(alias) => {
                params = generic_names(&alias.generics);
                const_params = generic_const_flags(&alias.generics);
            }
            DefKind::Trait(item) => {
                params = std::iter::once(self.self_name).chain(generic_names(&item.generics)).collect();
                const_params = std::iter::once(false).chain(generic_const_flags(&item.generics)).collect();
                self_ty = Some(self.program.types.param(0, self.self_name));
            }
            _ => {}
        }
        let bound = self.trait_names.get(&def).cloned().unwrap_or_default();
        Scope { module, params, const_params, self_ty, bound }
    }

    fn param_type(&mut self, scope: &Scope, name: ast::Ident, ty: Option<Id<ast::Type>>) -> TyId {
        match ty {
            Some(ty) => self.lower_type(scope, ty),
            None => {
                let message = format!("parameter `{}` needs a type", self.text(name.name));
                self.error(name.span, message);
                TyId::ERROR
            }
        }
    }

    fn lower_function(&mut self, def: DefId) {
        let Some(signature) = self.fn_ast(def) else { return };
        let scope = self.scope_for(def);
        let owner = self.program.defs[def].owner;
        let extend = self.program.defs[def].extend;
        let has_owner = owner.is_some() || extend.is_some();
        let owner_is_enum = owner.is_some_and(|owner| matches!(self.program.defs[owner].kind, DefKind::Enum(_)))
            || extend.is_some_and(|ext| matches!(self.program.types.kind(self.program.extends[ext].target), TyKind::Adt { def: target_def, .. } if self.program.defs[*target_def].enum_def().is_some()));
        let mut receiver = Receiver::None;
        let mut params = Vec::new();
        for (position, param) in signature.params.iter().enumerate() {
            match &param.kind {
                ParamKind::SelfValue { is_var } => {
                    if !has_owner {
                        self.error(param.span, "`self` is only for methods");
                    } else if position != 0 {
                        self.error(param.span, "`self` comes first");
                    } else if *is_var && owner_is_enum {
                        self.error(param.span, "an enum method takes `self`, not `var self`");
                    } else {
                        receiver = if *is_var { Receiver::Write } else { Receiver::Read };
                    }
                }
                ParamKind::Variadic { name, ty } => {
                    if position + 1 != signature.params.len() {
                        self.error(param.span, "`...` is the last parameter");
                    }
                    let ty = self.param_type(&scope, *name, *ty);
                    params.push(Param { name: name.name, ty, is_var: false, default: None, variadic: true });
                }
                ParamKind::Named { is_var, name, ty, default } => {
                    if *is_var && default.is_some() {
                        self.error(param.span, "a `var` parameter has no default");
                    } else if default.is_none() && params.iter().any(|earlier| earlier.default.is_some()) {
                        self.error(param.span, "a parameter after a default has a default");
                    }
                    let ty = self.param_type(&scope, *name, *ty);
                    params.push(Param { name: name.name, ty, is_var: *is_var, default: *default, variadic: false });
                }
            }
        }
        let ret = signature.ret.map_or(TyId::UNIT, |ret| self.lower_type(&scope, ret));
        if let DefKind::Function(function) = &mut self.program.defs[def].kind {
            function.params = params;
            function.receiver = receiver;
            function.ret = ret;
        }
    }

    fn lower_field(&mut self, scope: &Scope, field: &ast::Field, seen: &mut Vec<Field>) {
        if let Some(first) = seen.iter().find(|earlier| earlier.name == field.name.name) {
            let message = format!("`{}` is defined twice", self.text(field.name.name));
            self.sink.emit(Diagnostic::error(message, field.name.span).label(first.span, "first defined here"));
            return;
        }
        let ty = self.lower_type(scope, field.ty);
        seen.push(Field { name: field.name.name, ty, is_pub: field.is_pub, is_var: field.binding == Some(BindingKind::Var), default: field.default, doc: field.doc, span: field.name.span });
    }

    fn lower_traits(&mut self, scope: &Scope, traits: &[Id<ast::Type>]) -> Vec<Bound> {
        traits.iter().filter_map(|&listed| self.trait_ref(scope, listed)).collect()
    }

    fn lower_record(&mut self, def: DefId) {
        let Some((item, None)) = self.item_of(def) else { return };
        let ItemKind::Record(declared) = &item.kind else { return };
        let scope = self.scope_for(def);
        let mut fields = Vec::new();
        for member in &declared.members {
            if let Member::Field(field) = member {
                self.lower_field(&scope, field, &mut fields);
            }
        }
        let traits = self.lower_traits(&scope, &declared.traits);
        if let DefKind::Record(record) = &self.program.defs[def].kind {
            let clashes: Vec<(Symbol, Span)> = record.methods.iter().map(|&method| &self.program.defs[method]).filter(|method| fields.iter().any(|field| field.name == method.name)).map(|method| (method.name, method.span)).collect();
            for (name, span) in clashes {
                let message = format!("`{}` is both a field and a method", self.text(name));
                self.error(span, message);
            }
        }
        if let DefKind::Record(record) = &mut self.program.defs[def].kind {
            record.fields = fields;
            record.traits = traits;
        }
    }

    fn lower_enum(&mut self, def: DefId) {
        let Some((item, None)) = self.item_of(def) else { return };
        let ItemKind::Enum(declared) = &item.kind else { return };
        let scope = self.scope_for(def);
        let ast = self.ast(scope.module);
        let mut variants: Vec<Variant> = Vec::new();
        for variant in &declared.variants {
            if variants.iter().any(|earlier| earlier.name == variant.name.name) {
                continue;
            }
            let (payload, discriminant) = match &variant.payload {
                VariantPayload::Unit => (Payload::Unit, None),
                VariantPayload::Discriminant(expr) => (Payload::Unit, self.discriminant(ast, *expr)),
                VariantPayload::Tuple(types) => (Payload::Tuple(types.iter().map(|&ty| self.lower_type(&scope, ty)).collect()), None),
                VariantPayload::Struct(fields) => {
                    let mut lowered = Vec::new();
                    for field in fields {
                        self.lower_field(&scope, field, &mut lowered);
                    }
                    (Payload::Struct(lowered), None)
                }
            };
            variants.push(Variant { name: variant.name.name, payload, discriminant, span: variant.name.span });
        }
        let traits = self.lower_traits(&scope, &declared.traits);
        if let DefKind::Enum(en) = &mut self.program.defs[def].kind {
            en.variants = variants;
            en.traits = traits;
        }
    }

    fn discriminant(&mut self, ast: &ast::Ast, expr: Id<ast::Expr>) -> Option<i128> {
        let node = &ast[expr];
        let value = match &node.kind {
            ExprKind::Int(literal) => i128::try_from(literal.value).ok(),
            ExprKind::Unary { op: UnaryOp::Neg, operand } => match &ast[*operand].kind {
                ExprKind::Int(literal) => i128::try_from(literal.value).ok().map(|value| -value),
                _ => None,
            },
            _ => None,
        };
        if value.is_none() {
            self.error(node.span, "a discriminant is an integer literal");
        }
        value
    }

    fn lower_global(&mut self, def: DefId) {
        let Some((item, None)) = self.item_of(def) else { return };
        let ItemKind::Stmt(stmt) = &item.kind else { return };
        let module = self.program.defs[def].module;
        let ast = self.ast(module);
        let StmtKind::Binding(binding) = &ast[*stmt].kind else { return };
        if !matches!(ast[binding.pattern].kind, PatternKind::Binding(_)) {
            return;
        }
        let Some(annotation) = binding.ty else { return };
        let scope = Scope::empty(module);
        let ty = self.lower_type(&scope, annotation);
        if let DefKind::Global(global) = &mut self.program.defs[def].kind {
            global.ty = ty;
        }
    }

    /// Evaluates a module-level `const` item once; a cycle is reported cleanly.
    pub(crate) fn ensure_const_def(&mut self, def: DefId) -> ConstVal {
        if let Some(val) = self.program.const_vals.get(&def) {
            return val.clone();
        }
        let span = self.program.defs[def].span;
        if self.const_progress.get(&def) == Some(&Progress::Resolving) {
            let name = self.text(self.program.defs[def].name);
            self.error(span, format!("`const {name}` refers to itself"));
            return ConstVal::Int(0, IntTy::I64);
        }
        self.const_progress.insert(def, Progress::Resolving);
        let module = self.program.defs[def].module;
        let DefKind::Const(c) = &self.program.defs[def].kind else {
            self.const_progress.insert(def, Progress::Done);
            return ConstVal::Int(0, IntTy::I64);
        };
        let (ast_ty, expr) = (c.ast_ty, c.expr);
        let declared_ty = ast_ty.map(|t| {
            let scope = Scope::empty(module);
            self.lower_type(&scope, t)
        });
        let val = match comptime::eval_ast_const(self, module, expr) {
            Ok(v) => v,
            Err(msg) => {
                self.error(self.ast(module)[expr].span, msg);
                ConstVal::Int(0, IntTy::I64)
            }
        };
        let inferred_ty = match &val {
            ConstVal::Int(_, int) => self.program.types.int(*int),
            ConstVal::Bool(_) => TyId::BOOL,
            ConstVal::String(_) => TyId::STRING,
            ConstVal::Null => TyId::UNIT,
            ConstVal::Type(_) => TyId::STRING,
            ConstVal::Struct { ty, .. } | ConstVal::Enum { ty, .. } => *ty,
            ConstVal::Tuple(_) | ConstVal::Array(_) => declared_ty.unwrap_or(TyId::ERROR),
        };
        let final_ty = if let Some(d_ty) = declared_ty {
            if let ConstVal::Int(n, _) = &val
                && let TyKind::Int(target_int) = self.program.types.kind(d_ty)
            {
                if !target_int.fits(*n) {
                    self.error(span, format!("constant value {n} does not fit in {}", self.program.render(d_ty, self.interner)));
                } else {
                    self.program.const_vals.insert(def, ConstVal::Int(*n, *target_int));
                }
            }
            d_ty
        } else {
            inferred_ty
        };
        self.program.const_vals.entry(def).or_insert_with(|| val.clone());
        if let DefKind::Const(c) = &mut self.program.defs[def].kind {
            c.ty = final_ty;
        }
        self.const_progress.insert(def, Progress::Done);
        self.program.const_vals[&def].clone()
    }

    /// Resolves an alias's target once; a use while it resolves is a cycle.
    pub(super) fn ensure_alias(&mut self, def: DefId) {
        if self.aliases.contains_key(&def) {
            return;
        }
        self.aliases.insert(def, Progress::Resolving);
        if let Some((item, None)) = self.item_of(def)
            && let ItemKind::TypeAlias(declared) = &item.kind
        {
            let scope = self.scope_for(def);
            let target = self.lower_type(&scope, declared.target);
            if let DefKind::Alias(alias) = &mut self.program.defs[def].kind {
                alias.target = target;
            }
        }
        self.aliases.insert(def, Progress::Done);
    }

    /// A type may have several `from` methods, one per source type.
    fn check_overloads(&mut self, def: DefId) {
        let methods = match &self.program.defs[def].kind {
            DefKind::Record(record) => record.methods.clone(),
            DefKind::Enum(en) => en.methods.clone(),
            _ => return,
        };
        let mut sources: Vec<TyId> = Vec::new();
        for method in methods {
            let method = &self.program.defs[method];
            if method.name != self.from {
                continue;
            }
            let span = method.span;
            let Some(source) = method.function().and_then(|function| function.params.first()).map(|param| param.ty) else { continue };
            if sources.contains(&source) {
                let rendered = self.program.render(source, self.interner);
                self.error(span, format!("`from` for {rendered} is defined twice"));
            } else {
                sources.push(source);
            }
        }
    }
}
