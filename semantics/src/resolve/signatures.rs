use mote_base::{Diagnostic, Id, Span, Symbol};
use mote_syntax as ast;
use mote_syntax::{BindingKind, ExprKind, ItemKind, Member, ParamKind, PatternKind, StmtKind, UnaryOp, VariantPayload};

use super::lower::Scope;
use crate::cx::{Progress, Cx};
use crate::defs::{Bound, DefId, DefKind, Field, GenericParam, LangItem, Param, Payload, Receiver, Variant};
use crate::types::TyId;

/// A function's signature as the syntax tree writes it.
pub(crate) struct FnAst<'a> {
    pub generics: &'a [ast::GenericParam],
    pub params: &'a [ast::Param],
    pub ret: Option<Id<ast::Type>>,
    pub body: Option<&'a ast::Block>,
}

fn generic_names(generics: &[GenericParam]) -> Vec<Symbol> {
    generics.iter().map(|param| param.name).collect()
}

impl<'a> Cx<'a> {
    /// Resolves every definition's generics, then its types, then checks overloads.
    pub(super) fn signatures(&mut self) {
        let defs: Vec<DefId> = self.program.defs.iter().map(|(id, _)| id).collect();
        for &def in &defs {
            self.lower_generic_names(def);
        }
        for &def in &defs {
            self.lower_bounds(def);
        }
        for &def in &defs {
            match &self.program.defs[def].kind {
                DefKind::Function(_) => self.lower_function(def),
                DefKind::Record(_) => self.lower_record(def),
                DefKind::Enum(_) => self.lower_enum(def),
                DefKind::Alias(_) => self.ensure_alias(def),
                DefKind::Global(_) => self.lower_global(def),
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
            (ItemKind::NativeFunction(f), None) => Some(FnAst { generics: &f.generics, params: &f.params, ret: f.ret, body: None }),
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
                ItemKind::NativeFunction(f) => &f.generics,
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

    /// A definition's type parameter names, without bounds.
    fn lower_generic_names(&mut self, def: DefId) {
        let mut lowered: Vec<GenericParam> = Vec::new();
        for generic in self.generics_ast(def) {
            if lowered.iter().any(|param| param.name == generic.name.name) {
                let message = format!("`{}` is defined twice", self.text(generic.name.name));
                self.error(generic.name.span, message);
                continue;
            }
            lowered.push(GenericParam { name: generic.name.name, bounds: Vec::new() });
        }
        if let Some(generics) = self.generics_mut(def) {
            *generics = lowered;
        }
    }

    /// The bounds of a definition's type parameters, once every name and trait arity is known.
    fn lower_bounds(&mut self, def: DefId) {
        let scope = self.scope_for(def);
        let written = self.generics_ast(def);
        let mut bounds: Vec<Vec<Bound>> = Vec::new();
        let names: Vec<Symbol> = match &self.program.defs[def].kind {
            DefKind::Function(function) => generic_names(&function.generics),
            DefKind::Record(record) => generic_names(&record.generics),
            DefKind::Enum(en) => generic_names(&en.generics),
            DefKind::Trait(item) => generic_names(&item.generics),
            DefKind::Alias(alias) => generic_names(&alias.generics),
            _ => return,
        };
        for name in names {
            let Some(generic) = written.iter().find(|generic| generic.name.name == name) else {
                bounds.push(Vec::new());
                continue;
            };
            bounds.push(generic.bounds.iter().filter_map(|&bound| self.trait_ref(&scope, bound)).collect());
        }
        if let Some(generics) = self.generics_mut(def) {
            for (param, bound) in generics.iter_mut().zip(bounds) {
                param.bounds = bound;
            }
        }
    }

    /// `Self` of a type: itself applied to its own parameters; a built-in type is its own type.
    fn self_type(&mut self, def: DefId, names: &[Symbol]) -> TyId {
        let args: Vec<TyId> = names.iter().enumerate().map(|(index, &name)| self.program.types.param(index as u32, name)).collect();
        let lang = self.program.defs[def].lang;
        if let Some(ty) = lang.and_then(LangItem::primitive) {
            return ty;
        }
        match lang {
            Some(LangItem::Option) if args.len() == 1 => self.program.types.optional(args[0]),
            Some(LangItem::Array) if args.len() == 1 => self.program.types.array(args[0]),
            _ => self.program.types.adt(def, args),
        }
    }

    /// The names a definition's types are written against.
    pub(crate) fn scope_for(&mut self, def: DefId) -> Scope {
        let module = self.program.defs[def].module;
        let mut params = Vec::new();
        let mut self_ty = None;
        if let Some(owner) = self.program.defs[def].owner {
            match &self.program.defs[owner].kind {
                DefKind::Trait(item) => {
                    params.push(self.self_name);
                    params.extend(generic_names(&item.generics));
                    self_ty = Some(self.program.types.param(0, self.self_name));
                }
                DefKind::Record(record) => params = generic_names(&record.generics),
                DefKind::Enum(en) => params = generic_names(&en.generics),
                _ => {}
            }
            if self_ty.is_none() && matches!(self.program.defs[owner].kind, DefKind::Record(_) | DefKind::Enum(_)) {
                self_ty = Some(self.self_type(owner, &params));
            }
        }
        match &self.program.defs[def].kind {
            DefKind::Function(function) => params.extend(generic_names(&function.generics)),
            DefKind::Record(record) => {
                params = generic_names(&record.generics);
                self_ty = Some(self.self_type(def, &params));
            }
            DefKind::Enum(en) => {
                params = generic_names(&en.generics);
                self_ty = Some(self.self_type(def, &params));
            }
            DefKind::Alias(alias) => params = generic_names(&alias.generics),
            DefKind::Trait(item) => {
                params = std::iter::once(self.self_name).chain(generic_names(&item.generics)).collect();
                self_ty = Some(self.program.types.param(0, self.self_name));
            }
            _ => {}
        }
        let bound = self.trait_names.get(&def).cloned().unwrap_or_default();
        Scope { module, params, self_ty, bound }
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
        let owner_is_enum = owner.is_some_and(|owner| matches!(self.program.defs[owner].kind, DefKind::Enum(_)));
        let mut receiver = Receiver::None;
        let mut params = Vec::new();
        for (position, param) in signature.params.iter().enumerate() {
            match &param.kind {
                ParamKind::SelfValue { is_var } => {
                    if owner.is_none() {
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
        seen.push(Field { name: field.name.name, ty, is_pub: field.is_pub, is_var: field.binding == Some(BindingKind::Var), default: field.default, span: field.name.span });
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
        let scope = Scope { module, params: Vec::new(), self_ty: None, bound: Vec::new() };
        let ty = self.lower_type(&scope, annotation);
        if let DefKind::Global(global) = &mut self.program.defs[def].kind {
            global.ty = ty;
        }
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
