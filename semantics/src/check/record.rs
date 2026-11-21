use mote_base::{Id, Span, Symbol};
use mote_syntax as ast;
use mote_syntax::{FieldInit, Ident};

use super::call::Args;
use super::coerce::plan;
use super::FnCx;
use crate::cx::Cx;
use crate::defs::{DefId, DefKind, Field, ModuleId, Receiver, Resolved};
use crate::tree::{Expr, ExprKind, FieldValue};
use crate::types::{TyId, TyKind};

impl Cx<'_> {
    /// The names of a dotted path written as an expression.
    fn expr_names(&self, module: ModuleId, id: Id<ast::Expr>, out: &mut Vec<Symbol>) -> bool {
        let ast = self.ast(module);
        match &ast[id].kind {
            ast::ExprKind::Ident(name) => {
                out.push(*name);
                true
            }
            ast::ExprKind::Member { object, name } => {
                let found = self.expr_names(module, *object, out);
                out.push(name.name);
                found
            }
            _ => false,
        }
    }

    /// What a dotted name stands for when its first name is no local; reports nothing.
    pub(crate) fn static_target(&self, f: &FnCx, id: Id<ast::Expr>) -> Option<Resolved> {
        let mut names = Vec::new();
        if !self.expr_names(f.module, id, &mut names) || f.find(names[0]).is_some() {
            return None;
        }
        self.quiet_path(f.module, &names)
    }

    /// What a dotted name leads to through modules; reports nothing.
    pub(crate) fn quiet_path(&self, module: ModuleId, names: &[Symbol]) -> Option<Resolved> {
        let mut resolved = self.lookup(module, names[0])?;
        for name in &names[1..] {
            let Resolved::Module(next) = resolved else { return None };
            resolved = *self.program.modules[next].exports.get(name)?;
        }
        Some(resolved)
    }

    fn type_apply(&mut self, f: &mut FnCx, path: &ast::Path, written: &[Id<ast::Type>]) -> Option<(DefId, Vec<TyId>)> {
        let def = match self.resolve_path(f.module, path)? {
            Resolved::Def(def) if matches!(self.program.defs[def].kind, DefKind::Record(_) | DefKind::Enum(_)) => def,
            _ => {
                let last = path.segments.last().expect("a path has a segment");
                self.error(path.span, format!("`{}` takes no type arguments", self.text(last.name)));
                return None;
            }
        };
        let given = written.iter().map(|&ty| self.written_type(f, ty)).collect();
        Some((def, given))
    }

    /// The record or enum an expression names, with the type arguments written after it.
    fn type_object(&mut self, f: &mut FnCx, id: Id<ast::Expr>) -> Option<(DefId, Vec<TyId>)> {
        let ast = self.ast(f.module);
        if let ast::ExprKind::TypeApply { path, args } = &ast[id].kind {
            return self.type_apply(f, path, args);
        }
        match self.static_target(f, id) {
            Some(Resolved::Def(def)) if matches!(self.program.defs[def].kind, DefKind::Record(_) | DefKind::Enum(_)) => Some((def, Vec::new())),
            _ => None,
        }
    }

    /// The type of a record or enum with its arguments, taken from the context when none are written.
    pub(crate) fn instance(&mut self, def: DefId, mut given: Vec<TyId>, expected: Option<TyId>, span: Span) -> Option<TyId> {
        let expected = expected.map(|ty| match self.program.types.kind(ty) {
            TyKind::Optional(inner) => *inner,
            _ => ty,
        });
        if given.is_empty()
            && let Some(TyKind::Adt { def: other, args }) = expected.map(|ty| self.program.types.kind(ty))
            && *other == def
        {
            given = args.clone();
        }
        let ty = self.def_type(def, Some(given), span);
        if ty != TyId::ERROR && !matches!(self.program.types.kind(ty), TyKind::Adt { .. }) {
            let message = format!("{} cannot be built here", self.show_ty(ty));
            self.error(span, message);
            return None;
        }
        (ty != TyId::ERROR).then_some(ty)
    }

    /// Like `instance`, but a generic type given no arguments and expected none gets a variable for each.
    pub(crate) fn open_instance(&mut self, f: &mut FnCx, def: DefId, given: Vec<TyId>, expected: Option<TyId>, span: Span) -> Option<TyId> {
        let arity = match &self.program.defs[def].kind {
            DefKind::Record(record) => record.generics.len(),
            DefKind::Enum(en) => en.generics.len(),
            _ => 0,
        };
        if arity > 0 && given.is_empty() && self.program.defs[def].lang.is_none() && self.expected_args(def, expected).is_none() {
            let args = (0..arity).map(|_| self.fresh(f)).collect();
            return Some(self.program.types.intern(TyKind::Adt { def, args }));
        }
        self.instance(def, given, expected, span)
    }

    /// The type of a literal once its fields are checked; every argument must be found.
    pub(crate) fn close_instance(&mut self, f: &mut FnCx, ty: TyId, span: Span) -> Option<TyId> {
        let ty = self.solved(f, ty);
        if self.program.types.has_infer(ty) {
            let TyKind::Adt { def, .. } = self.program.types.kind(ty) else { return None };
            let message = format!("cannot infer the type arguments of `{}`", self.text(self.program.defs[*def].name));
            self.error(span, message);
            return None;
        }
        self.check_type_bounds(f, ty, span);
        Some(ty)
    }

    fn type_args(&self, ty: TyId) -> Vec<TyId> {
        match self.program.types.kind(ty) {
            TyKind::Adt { args, .. } => args.clone(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn methods_named(&self, def: DefId, name: Symbol) -> Vec<DefId> {
        self.program.methods_named(def, name)
    }

    pub(crate) fn visible(&self, f: &FnCx, def: DefId, is_pub: bool) -> bool {
        is_pub || self.program.defs[def].module == f.module
    }

    /// The method of a type a name stands for; reports what is wrong with the name.
    fn find_method(&mut self, f: &FnCx, owner: DefId, name: Ident) -> Option<DefId> {
        let found = self.methods_named(owner, name.name);
        let type_name = self.text(self.program.defs[owner].name);
        let text = self.text(name.name);
        match found.as_slice() {
            [] => {
                self.error(name.span, format!("{type_name} has no method `{text}`"));
                None
            }
            [method] if !self.visible(f, *method, self.program.defs[*method].is_pub) => {
                self.error(name.span, format!("`{text}` is private"));
                None
            }
            [method] => Some(*method),
            _ => {
                self.error(name.span, format!("`{text}` is overloaded; call it as `Type.{text}(value)`"));
                None
            }
        }
    }

    fn receiver_of(&self, method: DefId) -> Receiver {
        self.program.defs[method].function().expect("a method").receiver
    }

    /// A call whose callee is a name: a function, a construction, or a method on a type or a value.
    pub(crate) fn named_call(&mut self, f: &mut FnCx, callee: Id<ast::Expr>, args: &[ast::Arg], expected: Option<TyId>, span: Span) -> Option<Id<Expr>> {
        let ast = self.ast(f.module);
        match &ast[callee].kind {
            ast::ExprKind::TypeApply { path, args: written } => {
                let names: Vec<Symbol> = path.segments.iter().map(|segment| segment.name).collect();
                if let Some(Resolved::Def(def)) = self.quiet_path(f.module, &names)
                    && matches!(self.program.defs[def].kind, DefKind::Function(_))
                {
                    let given: Vec<TyId> = written.iter().map(|&ty| self.written_type(f, ty)).collect();
                    return Some(self.call_with(f, def, &[], &given, None, Args::Written(args), expected, span));
                }
                let Some((def, given)) = self.type_apply(f, path, written) else { return Some(self.error_expr(f, span)) };
                Some(self.construct(f, def, given, args, expected, span))
            }
            ast::ExprKind::Ident(name) => {
                if let Some(node) = self.defined_call(f, callee, args, expected, span) {
                    return Some(node);
                }
                if f.find(*name).is_some() || self.lookup(f.module, *name).is_some() {
                    return None;
                }
                if let Some((ty, index)) = self.variant_hint(expected, *name) {
                    return Some(self.variant_value(f, ty, index, args, span));
                }
                self.prelude_call(f, Ident { name: *name, span: ast[callee].span }, args, expected, span)
            }
            ast::ExprKind::OptionalMember { object, name } => {
                let subject = self.expr(f, *object, None);
                Some(self.optional_chain(f, subject, span, |this, f, payload| this.method_call_on(f, payload, *name, args, None, span)))
            }
            ast::ExprKind::Member { object, name } => {
                if let Some(node) = self.defined_call(f, callee, args, expected, span) {
                    return Some(node);
                }
                if let Some(Resolved::Module(_)) = self.static_target(f, *object) {
                    return None;
                }
                if let ast::ExprKind::Ident(param) = ast[*object].kind
                    && f.find(param).is_none()
                    && let Some(index) = f.scope.params.iter().position(|&declared| declared == param)
                {
                    return Some(self.bound_static_call(f, index as u32, param, *name, args, expected, span));
                }
                if let Some((def, given)) = self.type_object(f, *object) {
                    return Some(self.static_call(f, def, given, *name, args, expected, span));
                }
                Some(self.method_call(f, *object, *name, args, expected, span))
            }
            _ => None,
        }
    }

    /// A call of a function or a type that a name leads to.
    fn defined_call(&mut self, f: &mut FnCx, callee: Id<ast::Expr>, args: &[ast::Arg], expected: Option<TyId>, span: Span) -> Option<Id<Expr>> {
        let Some(Resolved::Def(def)) = self.static_target(f, callee) else { return None };
        match self.program.defs[def].kind {
            DefKind::Function(_) => Some(self.call_def(f, def, &[], None, args, expected, span)),
            DefKind::Record(_) => Some(self.construct(f, def, Vec::new(), args, expected, span)),
            DefKind::Enum(_) => {
                self.error(span, "an enum value is made with a variant");
                Some(self.error_expr(f, span))
            }
            _ => None,
        }
    }

    /// `T(args)`: the static `new`, or every field's default.
    fn construct(&mut self, f: &mut FnCx, def: DefId, given: Vec<TyId>, args: &[ast::Arg], expected: Option<TyId>, span: Span) -> Id<Expr> {
        if self.program.defs[def].enum_def().is_some() {
            self.error(span, "an enum value is made with a variant");
            return self.error_expr(f, span);
        }
        let new = self.interner.intern("new");
        let constructor = self.methods_named(def, new).into_iter().find(|&method| self.receiver_of(method) == Receiver::None);
        let generic = !self.program.defs[def].record().expect("a record").generics.is_empty();
        let open = constructor.is_some() && generic && given.is_empty() && self.expected_args(def, expected).is_none();
        let (ty, type_args) = if open {
            (TyId::ERROR, Vec::new())
        } else {
            let Some(ty) = self.instance(def, given, expected, span) else { return self.error_expr(f, span) };
            (ty, self.type_args(ty))
        };
        if let Some(method) = constructor {
            if !self.visible(f, method, self.program.defs[method].is_pub) {
                self.error(span, "`new` is private");
                return self.error_expr(f, span);
            }
            return self.call_def(f, method, &type_args, None, args, expected, span);
        }
        if !args.is_empty() {
            let message = format!("{} has no `new`", self.show_ty(ty));
            self.error(span, message);
            return self.error_expr(f, span);
        }
        let module = self.program.defs[def].module;
        let fields = self.program.defs[def].record().expect("a record").fields.clone();
        let mut values = Vec::new();
        for (index, field) in fields.iter().enumerate() {
            let Some(default) = field.default else {
                let message = format!("{} has no `new` and `{}` has no default", self.show_ty(ty), self.text(field.name));
                self.error(span, message);
                return self.error_expr(f, span);
            };
            let field_ty = self.program.types.subst(field.ty, &type_args);
            let value = self.default_value(f, module, default, field_ty);
            values.push(FieldValue { index: index as u32, value });
        }
        f.body.expr(ExprKind::Record { fields: values, base: None }, ty, span)
    }

    /// `Type.method(args)`.
    #[allow(clippy::too_many_arguments)]
    fn static_call(&mut self, f: &mut FnCx, def: DefId, given: Vec<TyId>, name: Ident, args: &[ast::Arg], expected: Option<TyId>, span: Span) -> Id<Expr> {
        if let Some(index) = self.variant_index(def, name.name) {
            let Some(ty) = self.open_instance(f, def, given, expected, span) else { return self.error_expr(f, span) };
            return self.variant_value(f, ty, index, args, span);
        }
        let type_args = if self.program.defs[def].lang.is_some_and(|item| item.primitive().is_some()) {
            Vec::new()
        } else if given.is_empty() {
            self.expected_args(def, expected).unwrap_or_default()
        } else {
            let Some(ty) = self.instance(def, given, None, span) else { return self.error_expr(f, span) };
            self.type_args(ty)
        };
        let overloads = self.methods_named(def, name.name);
        if overloads.len() > 1 && self.text(name.name) == "from" {
            return self.pick_from(f, def, &type_args, overloads, args, expected, span);
        }
        let Some(method) = self.find_method(f, def, name) else { return self.error_expr(f, span) };
        if self.receiver_of(method) != Receiver::None {
            self.error(name.span, format!("`{}` is called on a value", self.text(name.name)));
            return self.error_expr(f, span);
        }
        self.call_def(f, method, &type_args, None, args, expected, span)
    }

    /// The arguments of `def` that an expected type of that declaration gives.
    pub(crate) fn expected_args(&self, def: DefId, expected: Option<TyId>) -> Option<Vec<TyId>> {
        let expected = expected?;
        let ty = match self.program.types.kind(expected) {
            TyKind::Optional(inner) => *inner,
            _ => expected,
        };
        match self.program.types.kind(ty) {
            TyKind::Adt { def: other, args } if *other == def => Some(args.clone()),
            _ => None,
        }
    }

    /// `T.from(x)` where `T` has several `from`: the argument's type picks one.
    #[allow(clippy::too_many_arguments)]
    fn pick_from(&mut self, f: &mut FnCx, def: DefId, type_args: &[TyId], overloads: Vec<DefId>, args: &[ast::Arg], expected: Option<TyId>, span: Span) -> Id<Expr> {
        let [arg] = args else {
            self.error(span, "`from` takes one argument");
            return self.error_expr(f, span);
        };
        if let Some(name) = arg.name {
            self.error(name.span, "`from` takes one argument");
            return self.error_expr(f, span);
        }
        let node = self.expr(f, arg.value, None);
        let found = f.body.exprs[node].ty;
        if found == TyId::ERROR {
            return self.error_expr(f, span);
        }
        let mut options = Vec::new();
        for method in overloads {
            if !self.visible(f, method, self.program.defs[method].is_pub) {
                continue;
            }
            let param = self.program.defs[method].function().and_then(|function| function.params.first().map(|param| param.ty));
            if let Some(param) = param {
                options.push((method, self.program.types.subst(param, type_args)));
            }
        }
        let exact = options.iter().find(|&&(_, param)| param == found).map(|&(method, _)| method);
        let fits: Vec<DefId> = options.iter().filter(|&&(_, param)| plan(self.program, found, param).is_some()).map(|&(method, _)| method).collect();
        let picked = exact.or_else(|| if let [only] = fits[..] { Some(only) } else { None });
        let Some(method) = picked else {
            let message = format!("{} has no `from` for {}", self.text(self.program.defs[def].name), self.show_ty(found));
            self.error(span, message);
            return self.error_expr(f, span);
        };
        self.call_with(f, method, type_args, &[], None, Args::Checked(node), expected, span)
    }

    /// `value.method(args)`.
    fn method_call(&mut self, f: &mut FnCx, object: Id<ast::Expr>, name: Ident, args: &[ast::Arg], expected: Option<TyId>, span: Span) -> Id<Expr> {
        let object = self.expr(f, object, None);
        self.method_call_on(f, object, name, args, expected, span)
    }

    /// The declaration whose methods a value of the type has, with the type's arguments.
    pub(crate) fn method_owner(&self, ty: TyId) -> Option<(DefId, Vec<TyId>)> {
        self.program.method_owner(ty)
    }

    pub(crate) fn method_call_on(&mut self, f: &mut FnCx, object: Id<Expr>, name: Ident, args: &[ast::Arg], expected: Option<TyId>, span: Span) -> Id<Expr> {
        let ty = f.body.exprs[object].ty;
        if ty == TyId::ERROR {
            return self.error_expr(f, span);
        }
        if let TyKind::Param { index, .. } = *self.program.types.kind(ty) {
            return self.bound_call(f, object, index, name, args, expected, span);
        }
        if args.is_empty() && self.text(name.name) == "to_string" && self.builtin_show(f, ty) {
            return f.body.expr(ExprKind::Show(object), TyId::STRING, span);
        }
        let Some((def, type_args)) = self.method_owner(ty) else {
            let message = format!("{} has no methods", self.show_ty(ty));
            self.error(name.span, message);
            return self.error_expr(f, span);
        };
        let Some(method) = self.find_method(f, def, name) else { return self.error_expr(f, span) };
        if self.receiver_of(method) == Receiver::None {
            self.error(name.span, format!("`{}` is called on the type", self.text(name.name)));
            return self.error_expr(f, span);
        }
        self.call_def(f, method, &type_args, Some(object), args, expected, span)
    }

    /// A method of a type parameter: one of its bounds' traits supplies it.
    #[allow(clippy::too_many_arguments)]
    fn bound_call(&mut self, f: &mut FnCx, object: Id<Expr>, index: u32, name: Ident, args: &[ast::Arg], expected: Option<TyId>, span: Span) -> Id<Expr> {
        let ty = f.body.exprs[object].ty;
        let Some((method, owner_args)) = self.bound_method(f, ty, index, name) else { return self.error_expr(f, span) };
        if self.receiver_of(method) == Receiver::None {
            self.error(name.span, format!("`{}` is called on the type", self.text(name.name)));
            return self.error_expr(f, span);
        }
        self.call_def(f, method, &owner_args, Some(object), args, expected, span)
    }

    /// The trait method a name stands for on a type parameter, with the trait's arguments after `Self`.
    fn bound_method(&mut self, f: &FnCx, ty: TyId, index: u32, name: Ident) -> Option<(DefId, Vec<TyId>)> {
        let traits = f.bounds.get(index as usize).cloned().unwrap_or_default();
        let found = traits.iter().find_map(|bound| self.methods_named(bound.def, name.name).into_iter().next().map(|method| (method, bound.args.clone())));
        let Some((method, trait_args)) = found else {
            let message = format!("{} has no method `{}`", self.show_ty(ty), self.text(name.name));
            self.error(name.span, message);
            return None;
        };
        Some((method, std::iter::once(ty).chain(trait_args).collect()))
    }

    /// `T.name(args)` where `T` is a type parameter: a method without a receiver from one of its bounds' traits.
    #[allow(clippy::too_many_arguments)]
    fn bound_static_call(&mut self, f: &mut FnCx, index: u32, param: Symbol, name: Ident, args: &[ast::Arg], expected: Option<TyId>, span: Span) -> Id<Expr> {
        let ty = self.program.types.param(index, param);
        let Some((method, owner_args)) = self.bound_method(f, ty, index, name) else { return self.error_expr(f, span) };
        if self.receiver_of(method) != Receiver::None {
            self.error(name.span, format!("`{}` is called on a value", self.text(name.name)));
            return self.error_expr(f, span);
        }
        self.call_def(f, method, &owner_args, None, args, expected, span)
    }

    /// `object.name` as a value: a module's name, or a field.
    pub(crate) fn member(&mut self, f: &mut FnCx, object: Id<ast::Expr>, name: Ident, expected: Option<TyId>, span: Span) -> Id<Expr> {
        if let Some(Resolved::Module(module)) = self.static_target(f, object) {
            let found = self.program.modules[module].exports.get(&name.name).copied();
            return match found {
                Some(Resolved::Def(def)) => self.def_value(f, def, expected, span),
                Some(Resolved::Module(_)) => {
                    self.error(span, format!("`{}` is a module, not a value", self.text(name.name)));
                    self.error_expr(f, span)
                }
                None => {
                    self.missing_name(module, name);
                    self.error_expr(f, span)
                }
            };
        }
        if let Some((def, given)) = self.type_object(f, object) {
            if let Some(index) = self.variant_index(def, name.name) {
                let Some(ty) = self.open_instance(f, def, given, expected, span) else { return self.error_expr(f, span) };
                return self.unit_variant(f, ty, index, span);
            }
            if self.find_method(f, def, name).is_some() {
                self.error(span, format!("`{}` is a method; call it", self.text(name.name)));
            }
            return self.error_expr(f, span);
        }
        let object = self.expr(f, object, None);
        self.field(f, object, name, span)
    }

    pub(crate) fn field(&mut self, f: &mut FnCx, object: Id<Expr>, name: Ident, span: Span) -> Id<Expr> {
        let ty = f.body.exprs[object].ty;
        let (def, type_args) = match self.program.types.kind(ty).clone() {
            TyKind::Error => return self.error_expr(f, span),
            TyKind::Adt { def, args } if self.program.defs[def].record().is_some() => (def, args),
            _ => {
                let message = format!("{} has no fields", self.show_ty(ty));
                self.error(name.span, message);
                return self.error_expr(f, span);
            }
        };
        let found = {
            let record = self.program.defs[def].record().expect("a record");
            record.fields.iter().position(|field| field.name == name.name).map(|index| (index, record.fields[index].ty, record.fields[index].is_pub))
        };
        let text = self.text(name.name);
        let Some((index, declared, is_pub)) = found else {
            if self.methods_named(def, name.name).is_empty() {
                let message = format!("{} has no field `{text}`", self.show_ty(ty));
                self.error(name.span, message);
            } else {
                self.error(name.span, format!("`{text}` is a method; call it"));
            }
            return self.error_expr(f, span);
        };
        if !self.visible(f, def, is_pub) {
            self.error(name.span, format!("`{text}` is private"));
            return self.error_expr(f, span);
        }
        let field_ty = self.program.types.subst(declared, &type_args);
        f.body.expr(ExprKind::Field { object, index: index as u32 }, field_ty, span)
    }

    /// `Name { field: value, field, ..base }`.
    pub(crate) fn struct_init(&mut self, f: &mut FnCx, path: &ast::Path, written: &[Id<ast::Type>], inits: &[FieldInit], expected: Option<TyId>, span: Span) -> Id<Expr> {
        let names: Vec<Symbol> = path.segments.iter().map(|segment| segment.name).collect();
        let def = match self.quiet_path(f.module, &names) {
            Some(Resolved::Def(def)) if self.program.defs[def].record().is_some() => def,
            Some(_) => {
                let last = path.segments.last().expect("a path has a segment");
                self.error(path.span, format!("`{}` is not a struct or class", self.text(last.name)));
                return self.error_expr(f, span);
            }
            None => {
                if let Some(node) = self.variant_literal(f, path, &names, inits, expected, span) {
                    return node;
                }
                self.resolve_path(f.module, path);
                return self.error_expr(f, span);
            }
        };
        let given = written.iter().map(|&ty| self.written_type(f, ty)).collect();
        let Some(open) = self.open_instance(f, def, given, expected, path.span) else { return self.error_expr(f, span) };
        let type_args = self.type_args(open);
        let module = self.program.defs[def].module;
        let fields = self.program.defs[def].record().expect("a record").fields.clone();
        let outside = module != f.module;
        let Some((fields, base)) = self.init_fields(f, &fields, module, &type_args, open, inits, true, outside, span) else { return self.error_expr(f, span) };
        let Some(ty) = self.close_instance(f, open, span) else { return self.error_expr(f, span) };
        f.body.expr(ExprKind::Record { fields, base }, ty, span)
    }

    /// The values of a literal's fields, with defaults filled in; `None` after reporting a problem.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn init_fields(&mut self, f: &mut FnCx, fields: &[Field], module: ModuleId, type_args: &[TyId], ty: TyId, inits: &[FieldInit], allow_base: bool, outside: bool, span: Span) -> Option<(Vec<FieldValue>, Option<Id<Expr>>)> {
        let ast = self.ast(f.module);
        let mut values = Vec::new();
        let mut seen = vec![false; fields.len()];
        let mut base = None;
        let mut ok = true;
        for (position, init) in inits.iter().enumerate() {
            let (name, value) = match *init {
                FieldInit::Spread(inner) => {
                    if !allow_base {
                        self.error(ast[inner].span, "a variant literal takes no `..base`");
                        ok = false;
                        continue;
                    }
                    if position != 0 {
                        self.error(ast[inner].span, "`..base` comes first");
                        ok = false;
                    }
                    base = Some(self.expr_to(f, inner, ty));
                    continue;
                }
                FieldInit::Named { name, value } => (name, Some(value)),
                FieldInit::Shorthand(name) => (name, None),
            };
            let text = self.text(name.name);
            let Some(index) = fields.iter().position(|field| field.name == name.name) else {
                let message = format!("{} has no field `{text}`", self.show_ty(ty));
                self.error(name.span, message);
                ok = false;
                continue;
            };
            if seen[index] {
                self.error(name.span, format!("`{text}` is given twice"));
                ok = false;
                continue;
            }
            seen[index] = true;
            if outside && !fields[index].is_pub {
                self.error(name.span, format!("`{text}` is private"));
                ok = false;
                continue;
            }
            let field_ty = self.program.types.subst(fields[index].ty, type_args);
            let value = match value {
                Some(value) => self.expr_to(f, value, field_ty),
                None => {
                    let node = self.ident(f, name.name, Some(field_ty), name.span);
                    self.coerce(f, node, field_ty, name.span)
                }
            };
            values.push(FieldValue { index: index as u32, value });
        }
        for (index, field) in fields.iter().enumerate() {
            if seen[index] {
                continue;
            }
            let text = self.text(field.name);
            if base.is_some() {
                if outside && !field.is_pub {
                    self.error(span, format!("`..base` copies the private field `{text}`"));
                    ok = false;
                }
                continue;
            }
            match field.default {
                Some(default) => {
                    let field_ty = self.program.types.subst(field.ty, type_args);
                    let value = self.default_value(f, module, default, field_ty);
                    values.push(FieldValue { index: index as u32, value });
                }
                None => {
                    self.error(span, format!("missing field `{text}`"));
                    ok = false;
                }
            }
        }
        ok.then_some((values, base))
    }
}
