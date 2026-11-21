use mote_base::{Id, Span, Symbol};
use mote_syntax as ast;
use mote_syntax::{FieldInit, Ident};

use super::call::Args;
use super::coerce::plan;
use super::FnCx;
use crate::cx::Cx;
use crate::defs::{DefId, DefKind, ExtendId, Field, GenericParam, ModuleId, Receiver, Resolved, TyHead};
use crate::tree::{Callee, Expr, ExprKind, FieldValue};
use crate::types::{BuiltinTy, ConstArg, GenericArg, TyId, TyKind};

enum TypeTarget {
    Def(DefId, Vec<GenericArg>),
    Builtin(TyId),
}

impl Cx<'_> {
    /// Lowers a generic argument written in an expression/method/type position.
    pub(crate) fn written_generic(&mut self, f: &mut FnCx, written: &ast::GenericArg) -> GenericArg {
        let scope = f.scope.clone();
        let arg = self.lower_generic_arg_with(&scope, Some(f), written);
        if let GenericArg::Type(ty) = arg {
            let span = match written {
                ast::GenericArg::Type(t) => self.ast(f.module)[*t].span,
                ast::GenericArg::Expr(e) => self.ast(f.module)[*e].span,
            };
            self.check_type_bounds(f, ty, span);
        }
        arg
    }

    /// The names of a dotted path written as an expression.
    fn expr_names(&self, module: ModuleId, id: Id<ast::Expr>, out: &mut Vec<Symbol>) -> bool {
        let ast = self.ast(module);
        match &ast[id].kind {
            ast::ExprKind::Ident(name) => {
                out.push(*name);
                true
            }
            ast::ExprKind::Member { object, name, .. } => {
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

    fn type_apply(&mut self, f: &mut FnCx, path: &ast::Path, written: &[ast::GenericArg]) -> Option<(DefId, Vec<GenericArg>)> {
        let def = match self.resolve_path(f.module, path)? {
            Resolved::Def(def) if matches!(self.program.defs[def].kind, DefKind::Record(_) | DefKind::Enum(_)) => def,
            _ => {
                let last = path.segments.last().expect("a path has a segment");
                self.error(path.span, format!("`{}` takes no type arguments", self.text(last.name)));
                return None;
            }
        };
        let given = written.iter().map(|g| self.written_generic(f, g)).collect();
        Some((def, given))
    }

    fn builtin_type_target(&mut self, f: &mut FnCx, id: Id<ast::Expr>) -> Option<TyId> {
        let ast = self.ast(f.module);
        match ast[id].kind.clone() {
            ast::ExprKind::Ident(sym) => {
                if f.find(sym).is_some() || self.lookup(f.module, sym).is_some() {
                    return None;
                }
                let text = self.text(sym);
                if let Some(prim) = self.primitive_ty_by_name(&text) {
                    return Some(prim);
                }
                if text == "Option" {
                    let inner = self.fresh(f);
                    return Some(self.program.types.optional(inner));
                }
                if text == "Array" {
                    let inner = self.fresh(f);
                    return Some(self.program.types.array(inner));
                }
                if let Some(kind) = BuiltinTy::from_name(&text) {
                    let args = (0..kind.arity()).map(|_| self.fresh(f)).collect();
                    return Some(self.program.types.builtin(kind, args));
                }
                None
            }
            ast::ExprKind::TypeApply { path, args } if path.segments.len() == 1 => {
                let sym = path.segments[0].name;
                if f.find(sym).is_some() || self.lookup(f.module, sym).is_some() {
                    return None;
                }
                let text = self.text(sym);
                if text == "Option" || text == "Array" || BuiltinTy::from_name(&text).is_some() {
                    let g_args: Vec<GenericArg> = args.iter().map(|g| self.written_generic(f, g)).collect();
                    return Some(self.lower_path(&f.scope, &path, Some(g_args), path.span));
                }
                None
            }
            _ => None,
        }
    }

    fn primitive_ty_by_name(&self, text: &str) -> Option<TyId> {
        match text {
            "Null" => Some(TyId::UNIT),
            "Bool" => Some(TyId::BOOL),
            "Int" => Some(TyId::INT),
            "Int8" => Some(TyId::INT8),
            "Int16" => Some(TyId::INT16),
            "Int32" => Some(TyId::INT32),
            "Int64" => Some(TyId::INT64),
            "UInt8" => Some(TyId::UINT8),
            "UInt16" => Some(TyId::UINT16),
            "UInt32" => Some(TyId::UINT32),
            "UInt64" => Some(TyId::UINT64),
            "Float" => Some(TyId::FLOAT),
            "Float32" => Some(TyId::FLOAT32),
            "Float64" => Some(TyId::FLOAT64),
            "String" => Some(TyId::STRING),
            "Any" => Some(TyId::ANY),
            _ => None,
        }
    }

    /// The record, enum or builtin type an expression names, with the type arguments written after it.
    fn type_object(&mut self, f: &mut FnCx, id: Id<ast::Expr>) -> Option<TypeTarget> {
        let ast_kind = self.ast(f.module)[id].kind.clone();
        if let Some(builtin) = self.builtin_type_target(f, id) {
            return Some(TypeTarget::Builtin(builtin));
        }
        if let ast::ExprKind::TypeApply { path, args } = ast_kind {
            return self.type_apply(f, &path, &args).map(|(def, g)| TypeTarget::Def(def, g));
        }
        match self.static_target(f, id) {
            Some(Resolved::Def(def)) if matches!(self.program.defs[def].kind, DefKind::Record(_) | DefKind::Enum(_)) => Some(TypeTarget::Def(def, Vec::new())),
            _ => None,
        }
    }

    /// The type of a record or enum with its arguments, taken from the context when none are written.
    pub(crate) fn instance(&mut self, def: DefId, mut given: Vec<GenericArg>, expected: Option<TyId>, span: Span) -> Option<TyId> {
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
    pub(crate) fn open_instance(&mut self, f: &mut FnCx, def: DefId, given: Vec<GenericArg>, expected: Option<TyId>, span: Span) -> Option<TyId> {
        let generics = match &self.program.defs[def].kind {
            DefKind::Record(record) => record.generics.clone(),
            DefKind::Enum(en) => en.generics.clone(),
            _ => Vec::new(),
        };
        if !generics.is_empty() && given.is_empty() && self.expected_args(def, expected).is_none() {
            let args = generics
                .iter()
                .map(|p| match p {
                    GenericParam::Type { .. } => GenericArg::Type(self.fresh(f)),
                    GenericParam::Const { .. } => GenericArg::Const(self.fresh_const(f)),
                })
                .collect();
            return Some(self.program.types.adt(def, args));
        }
        self.instance(def, given, expected, span)
    }

    /// The type of a literal once its fields are checked; every argument must be found.
    pub(crate) fn close_instance(&mut self, f: &mut FnCx, ty: TyId, span: Span) -> Option<TyId> {
        let ty = self.solved(f, ty);
        if self.program.types.has_infer(ty) {
            let TyKind::Adt { def, .. } = self.program.types.kind(ty) else { return None };
            let message = format!("cannot infer the generic arguments of `{}`", self.text(self.program.defs[*def].name));
            self.error(span, message);
            return None;
        }
        self.check_type_bounds(f, ty, span);
        Some(ty)
    }

    fn type_args(&self, ty: TyId) -> Vec<GenericArg> {
        match self.program.types.kind(ty) {
            TyKind::Adt { args, .. } => args.clone(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn methods_named(&self, def: DefId, name: Symbol) -> Vec<DefId> {
        self.program.methods_named(def, name)
    }

    /// Matches `extend.target` against concrete/parameterized `ty`, returning `extend`'s substituted generic arguments.
    pub(crate) fn match_extend_target(&mut self, ext_id: ExtendId, ty: TyId) -> Option<Vec<GenericArg>> {
        let (target, generic_count) = {
            let ext = &self.program.extends[ext_id];
            (ext.target, ext.generics.len())
        };
        if target == TyId::ERROR || ty == TyId::ERROR {
            return None;
        }
        let mut bindings: Vec<Option<GenericArg>> = vec![None; generic_count];
        if !self.match_extend_ty(target, ty, &mut bindings) {
            return None;
        }
        bindings.into_iter().collect()
    }

    fn match_extend_const(&self, pattern: &ConstArg, concrete: &ConstArg, bindings: &mut [Option<GenericArg>]) -> bool {
        if let ConstArg::Param { index, .. } = pattern {
            let idx = *index as usize;
            let Some(slot) = bindings.get_mut(idx) else { return false };
            return match slot {
                Some(GenericArg::Const(prev)) => prev == concrete,
                Some(_) => false,
                None => {
                    *slot = Some(GenericArg::Const(concrete.clone()));
                    true
                }
            };
        }
        pattern == concrete
    }

    fn match_extend_ty(&self, pattern: TyId, concrete: TyId, bindings: &mut [Option<GenericArg>]) -> bool {
        let pk = self.program.types.kind(pattern).clone();
        if let TyKind::Param { index, .. } = pk {
            let idx = index as usize;
            let Some(slot) = bindings.get_mut(idx) else { return false };
            return match slot {
                Some(GenericArg::Type(prev)) => *prev == concrete,
                Some(_) => false,
                None => {
                    *slot = Some(GenericArg::Type(concrete));
                    true
                }
            };
        }
        if pattern == concrete && !self.program.types.has_param(pattern) {
            return true;
        }
        let ck = self.program.types.kind(concrete).clone();
        match (pk, ck) {
            (TyKind::Optional(p), TyKind::Optional(c)) => self.match_extend_ty(p, c, bindings),
            (TyKind::Array(p), TyKind::Array(c)) => self.match_extend_ty(p, c, bindings),
            (TyKind::FixedArray(p, pn), TyKind::FixedArray(c, cn)) => {
                self.match_extend_ty(p, c, bindings) && self.match_extend_const(&pn, &cn, bindings)
            }
            (TyKind::Tuple(p), TyKind::Tuple(c)) if p.len() == c.len() => {
                p.into_iter().zip(c).all(|(a, b)| self.match_extend_ty(a, b, bindings))
            }
            (TyKind::Builtin { kind: pk, args: pa }, TyKind::Builtin { kind: ck, args: ca }) if pk == ck && pa.len() == ca.len() => {
                pa.into_iter().zip(ca).all(|(a, b)| self.match_extend_ty(a, b, bindings))
            }
            (TyKind::Adt { def: pd, args: pa }, TyKind::Adt { def: cd, args: ca }) if pd == cd && pa.len() == ca.len() => {
                pa.into_iter().zip(ca).all(|(a, b)| match (a, b) {
                    (GenericArg::Type(pt), GenericArg::Type(ct)) => self.match_extend_ty(pt, ct, bindings),
                    (GenericArg::Const(pc), GenericArg::Const(cc)) => self.match_extend_const(&pc, &cc, bindings),
                    _ => false,
                })
            }
            _ => false,
        }
    }

    /// All methods named `name` applicable to `ty` (inherent on `Adt` plus matching `extend` blocks).
    pub(crate) fn methods_for_type(&mut self, ty: TyId, name: Symbol) -> Vec<(DefId, Vec<GenericArg>)> {
        let mut out = Vec::new();
        if let Some((owner, owner_args)) = self.program.method_owner(ty) {
            for method in self.methods_named(owner, name) {
                out.push((method, owner_args.clone()));
            }
        }
        if let Some(head) = TyHead::of(&self.program.types, ty)
            && let Some(extends) = self.program.extend_by_head.get(&head).cloned()
        {
            for ext_id in extends {
                let cand_methods = self.program.extend_methods_named(ext_id, name);
                if cand_methods.is_empty() {
                    continue;
                }
                if let Some(ext_args) = self.match_extend_target(ext_id, ty) {
                    for method in cand_methods {
                        out.push((method, ext_args.clone()));
                    }
                }
            }
        }
        out
    }

    /// Whether `ty` has any inherent or `extend` methods in scope.
    pub(crate) fn has_methods(&self, ty: TyId) -> bool {
        self.program.method_owner(ty).is_some()
            || TyHead::of(&self.program.types, ty).is_some_and(|head| self.program.extend_by_head.contains_key(&head))
    }

    pub(crate) fn visible(&self, f: &FnCx, def: DefId, is_pub: bool) -> bool {
        is_pub || self.program.defs[def].module == f.module
    }

    fn find_method_on_ty(&mut self, f: &FnCx, ty: TyId, name: Ident) -> Option<(DefId, Vec<GenericArg>)> {
        let found = self.methods_for_type(ty, name.name);
        let type_name = self.show_ty(ty);
        let text = self.text(name.name);
        match found.as_slice() {
            [] => {
                self.error(name.span, format!("{type_name} has no method `{text}`"));
                None
            }
            [(method, args)] if !self.visible(f, *method, self.program.defs[*method].is_pub) => {
                let _ = args;
                self.error(name.span, format!("`{text}` is private"));
                None
            }
            [(method, args)] => Some((*method, args.clone())),
            _ => {
                // Multiple candidates (e.g. multiple extend blocks or overloads): pick single visible match if unambiguous
                let visible: Vec<_> = found
                    .into_iter()
                    .filter(|(m, _)| self.visible(f, *m, self.program.defs[*m].is_pub))
                    .collect();
                match visible.as_slice() {
                    [] => {
                        self.error(name.span, format!("`{text}` is private"));
                        None
                    }
                    [(method, args)] => Some((*method, args.clone())),
                    _ => {
                        self.error(name.span, format!("`{text}` is overloaded; call it as `Type.{text}(value)`"));
                        None
                    }
                }
            }
        }
    }

    pub(crate) fn receiver_of(&self, method: DefId) -> Receiver {
        self.program.defs[method].function().expect("a method").receiver
    }

    pub(crate) fn quiet_method(&mut self, f: &FnCx, ty: TyId, name: Symbol) -> Option<(DefId, Vec<GenericArg>)> {
        if let TyKind::Param { index, .. } = *self.program.types.kind(ty) {
            let traits = f.bounds.get(index as usize)?;
            let (method, bound_args) = traits.iter().find_map(|bound| {
                self.methods_named(bound.def, name).into_iter().next().map(|method| (method, bound.args.clone()))
            })?;
            let args = std::iter::once(ty).chain(bound_args).map(GenericArg::Type).collect();
            return Some((method, args));
        }
        let methods = self.methods_for_type(ty, name);
        let [(method, args)] = methods.as_slice() else { return None };
        if !self.visible(f, *method, self.program.defs[*method].is_pub) {
            return None;
        }
        Some((*method, args.clone()))
    }

    /// A call whose callee is a name: a function, a construction, or a method on a type or a value.
    pub(crate) fn named_call(&mut self, f: &mut FnCx, callee: Id<ast::Expr>, args: &[ast::Arg], expected: Option<TyId>, span: Span) -> Option<Id<Expr>> {
        let ast = self.ast(f.module);
        match &ast[callee].kind.clone() {
            ast::ExprKind::TypeApply { path, args: written } => {
                let names: Vec<Symbol> = path.segments.iter().map(|segment| segment.name).collect();
                if let Some(Resolved::Def(def)) = self.quiet_path(f.module, &names)
                    && matches!(self.program.defs[def].kind, DefKind::Function(_))
                {
                    let given: Vec<GenericArg> = written.iter().map(|g| self.written_generic(f, g)).collect();
                    return Some(self.call_with(f, def, &[], &given, None, Args::Written(args), expected, span));
                }
                if path.segments.len() >= 2 {
                    let (last, prefix) = path.segments.split_last().unwrap();
                    let prefix_names: Vec<Symbol> = prefix.iter().map(|s| s.name).collect();
                    if f.find(prefix[0].name).is_some()
                        || matches!(self.lookup(f.module, prefix[0].name), Some(Resolved::Def(d)) if matches!(self.program.defs[d].kind, DefKind::Global(_)))
                    {
                        let mut subject = self.ident(f, prefix[0].name, None, prefix[0].span);
                        for segment in &prefix[1..] {
                            subject = self.field(f, subject, *segment, segment.span);
                        }
                        let own_args: Vec<GenericArg> = written.iter().map(|g| self.written_generic(f, g)).collect();
                        return Some(self.method_call_on(f, subject, *last, &own_args, args, expected, span));
                    }
                    if prefix.len() == 1 && let Some(index) = f.scope.params.iter().position(|&declared| declared == prefix[0].name) {
                        let own_args: Vec<GenericArg> = written.iter().map(|g| self.written_generic(f, g)).collect();
                        return Some(self.bound_static_call(f, index as u32, prefix[0].name, *last, &own_args, args, expected, span));
                    }
                    if let Some(Resolved::Def(def)) = self.quiet_path(f.module, &prefix_names)
                        && matches!(self.program.defs[def].kind, DefKind::Record(_) | DefKind::Enum(_))
                    {
                        let own_args: Vec<GenericArg> = written.iter().map(|g| self.written_generic(f, g)).collect();
                        return Some(self.static_call(f, def, Vec::new(), *last, &own_args, args, expected, span));
                    }
                    if prefix.len() == 1 {
                        let text = self.text(prefix[0].name);
                        if let Some(ty) = self.primitive_ty_by_name(&text)
                            .or_else(|| BuiltinTy::from_name(&text).map(|k| {
                                let fresh = (0..k.arity()).map(|_| self.fresh(f)).collect();
                                self.program.types.builtin(k, fresh)
                            }))
                        {
                            let own_args: Vec<GenericArg> = written.iter().map(|g| self.written_generic(f, g)).collect();
                            return Some(self.static_call_on_ty(f, ty, *last, &own_args, args, expected, span));
                        }
                    }
                }
                if path.segments.len() == 1 && BuiltinTy::from_name(&self.text(path.segments[0].name)).is_some() {
                    let given: Vec<GenericArg> = written.iter().map(|g| self.written_generic(f, g)).collect();
                    let ty = self.lower_path(&f.scope, path, Some(given), path.span);
                    let new = Ident { name: self.interner.intern("new"), span: path.span };
                    return Some(self.static_call_on_ty(f, ty, new, &[], args, expected, span));
                }
                let Some((def, given)) = self.type_apply(f, path, written) else { return Some(self.error_expr(f, span)) };
                Some(self.construct(f, def, given, args, expected, span))
            }
            ast::ExprKind::Ident(name) => {
                let name = *name;
                let callee_span = ast[callee].span;
                if let Some(node) = self.defined_call(f, callee, args, expected, span) {
                    return Some(node);
                }
                if f.find(name).is_some() || self.lookup(f.module, name).is_some() {
                    return None;
                }
                if let Some((ty, index)) = self.variant_hint(expected, name) {
                    return Some(self.variant_value(f, ty, index, args, span));
                }
                if let Some(kind) = BuiltinTy::from_name(&self.text(name)) {
                    let given = match self.without_optional(expected).map(|t| self.program.types.kind(t).clone()) {
                        Some(TyKind::Builtin { kind: k, args: exp_args }) if k == kind && exp_args.len() == kind.arity() => exp_args,
                        _ => (0..kind.arity()).map(|_| self.fresh(f)).collect(),
                    };
                    let ty = self.program.types.builtin(kind, given);
                    let new = Ident { name: self.interner.intern("new"), span: callee_span };
                    return Some(self.static_call_on_ty(f, ty, new, &[], args, expected, span));
                }
                self.prelude_call(f, Ident { name, span: callee_span }, args, expected, span)
            }
            ast::ExprKind::OptionalMember { object, name, args: type_args } => {
                let subject = self.expr(f, *object, None);
                let own_args: Vec<GenericArg> = type_args.iter().map(|g| self.written_generic(f, g)).collect();
                Some(self.optional_chain(f, subject, span, |this, f, payload| this.method_call_on(f, payload, *name, &own_args, args, None, span)))
            }
            ast::ExprKind::Member { object, name, args: type_args } => {
                let own_args: Vec<GenericArg> = type_args.iter().map(|g| self.written_generic(f, g)).collect();
                if let Some(node) = self.defined_call(f, callee, args, expected, span) {
                    return Some(node);
                }
                if let Some(Resolved::Module(_)) = self.static_target(f, *object) {
                    return None;
                }
                if let ast::ExprKind::Ident(param) = self.ast(f.module)[*object].kind
                    && f.find(param).is_none()
                    && let Some(index) = f.scope.params.iter().position(|&declared| declared == param)
                {
                    return Some(self.bound_static_call(f, index as u32, param, *name, &own_args, args, expected, span));
                }
                match self.type_object(f, *object) {
                    Some(TypeTarget::Def(def, given)) => Some(self.static_call(f, def, given, *name, &own_args, args, expected, span)),
                    Some(TypeTarget::Builtin(ty)) => Some(self.static_call_on_ty(f, ty, *name, &own_args, args, expected, span)),
                    None => Some(self.method_call(f, *object, *name, &own_args, args, expected, span)),
                }
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
    fn construct(&mut self, f: &mut FnCx, def: DefId, given: Vec<GenericArg>, args: &[ast::Arg], expected: Option<TyId>, span: Span) -> Id<Expr> {
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
            let field_ty = self.program.types.subst_generic(field.ty, &type_args);
            let value = self.default_value(f, module, default, field_ty);
            values.push(FieldValue { index: index as u32, value });
        }
        f.body.expr(ExprKind::Record { fields: values, base: None }, ty, span)
    }

    /// `BuiltinOrPrim.method(args)`.
    #[allow(clippy::too_many_arguments)]
    fn static_call_on_ty(
        &mut self,
        f: &mut FnCx,
        ty: TyId,
        name: Ident,
        own_args: &[GenericArg],
        args: &[ast::Arg],
        expected: Option<TyId>,
        span: Span,
    ) -> Id<Expr> {
        let overloads = self.methods_for_type(ty, name.name);
        if overloads.len() > 1 && self.text(name.name) == "from" {
            return self.pick_from(f, &self.show_ty(ty), overloads, args, expected, span);
        }
        let Some((method, owner_args)) = self.find_method_on_ty(f, ty, name) else {
            return self.error_expr(f, span);
        };
        if self.receiver_of(method) != Receiver::None {
            self.error(name.span, format!("`{}` is called on a value", self.text(name.name)));
            return self.error_expr(f, span);
        }
        self.call_with(f, method, &owner_args, own_args, None, Args::Written(args), expected, span)
    }

    /// `Type.method(args)`.
    #[allow(clippy::too_many_arguments)]
    fn static_call(&mut self, f: &mut FnCx, def: DefId, given: Vec<GenericArg>, name: Ident, own_args: &[GenericArg], args: &[ast::Arg], expected: Option<TyId>, span: Span) -> Id<Expr> {
        if let Some(index) = self.variant_index(def, name.name) {
            let Some(ty) = self.open_instance(f, def, given, expected, span) else { return self.error_expr(f, span) };
            return self.variant_value(f, ty, index, args, span);
        }
        let target_ty = if given.is_empty() {
            if let Some(exp) = self.expected_args(def, expected) {
                self.program.types.adt(def, exp)
            } else {
                let Some(open) = self.open_instance(f, def, Vec::new(), None, span) else { return self.error_expr(f, span) };
                open
            }
        } else {
            let Some(ty) = self.instance(def, given, None, span) else { return self.error_expr(f, span) };
            ty
        };
        let type_args = self.type_args(target_ty);
        let overloads = self.methods_named(def, name.name);
        if overloads.len() > 1 && self.text(name.name) == "from" {
            let pairs = overloads.into_iter().map(|m| (m, type_args.clone())).collect();
            let type_name = self.text(self.program.defs[def].name);
            return self.pick_from(f, &type_name, pairs, args, expected, span);
        }
        let Some((method, owner_args)) = self.find_method_on_ty(f, target_ty, name) else {
            return self.error_expr(f, span);
        };
        if self.receiver_of(method) != Receiver::None {
            self.error(name.span, format!("`{}` is called on a value", self.text(name.name)));
            return self.error_expr(f, span);
        }
        self.call_with(f, method, &owner_args, own_args, None, Args::Written(args), expected, span)
    }

    /// The arguments of `def` that an expected type of that declaration gives.
    pub(crate) fn expected_args(&self, def: DefId, expected: Option<TyId>) -> Option<Vec<GenericArg>> {
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
    fn pick_from(&mut self, f: &mut FnCx, type_name: &str, overloads: Vec<(DefId, Vec<GenericArg>)>, args: &[ast::Arg], expected: Option<TyId>, span: Span) -> Id<Expr> {
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
        for (method, type_args) in overloads {
            if !self.visible(f, method, self.program.defs[method].is_pub) {
                continue;
            }
            let param = self.program.defs[method].function().and_then(|function| function.params.first().map(|param| param.ty));
            if let Some(param) = param {
                let expected_param = self.program.types.subst_generic(param, &type_args);
                options.push((method, type_args, expected_param));
            }
        }
        let exact = options.iter().find(|(_, _, param)| *param == found).map(|(method, type_args, _)| (*method, type_args.clone()));
        let fits: Vec<_> = options
            .iter()
            .filter(|(_, _, param)| plan(self.program, found, *param).is_some())
            .map(|(method, type_args, _)| (*method, type_args.clone()))
            .collect();
        let picked = exact.or_else(|| if fits.len() == 1 { fits.into_iter().next() } else { None });
        let Some((method, type_args)) = picked else {
            let message = format!("{type_name} has no `from` for {}", self.show_ty(found));
            self.error(span, message);
            return self.error_expr(f, span);
        };
        self.call_with(f, method, &type_args, &[], None, Args::Checked(&[node]), expected, span)
    }

    /// `value.method(args)`.
    #[allow(clippy::too_many_arguments)]
    fn method_call(&mut self, f: &mut FnCx, object: Id<ast::Expr>, name: Ident, own_args: &[GenericArg], args: &[ast::Arg], expected: Option<TyId>, span: Span) -> Id<Expr> {
        let object = self.expr(f, object, None);
        self.method_call_on(f, object, name, own_args, args, expected, span)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn method_call_on(&mut self, f: &mut FnCx, object: Id<Expr>, name: Ident, own_args: &[GenericArg], args: &[ast::Arg], expected: Option<TyId>, span: Span) -> Id<Expr> {
        let ty = self.solved(f, f.body.exprs[object].ty);
        if ty == TyId::ERROR {
            return self.error_expr(f, span);
        }
        if let TyKind::Param { index, .. } = *self.program.types.kind(ty) {
            return self.bound_call(f, object, index, name, own_args, args, expected, span);
        }
        let candidates = self.methods_for_type(ty, name.name);
        if ty == TyId::ANY && self.text(name.name) == "to" && candidates.is_empty() {
            return self.any_to_call(f, object, own_args, args, expected, span);
        }
        if candidates.is_empty() && args.is_empty() && self.text(name.name) == "to_string" && self.builtin_show(f, ty) {
            return f.body.expr(ExprKind::Show(object), TyId::STRING, span);
        }
        if candidates.is_empty() && !self.has_methods(ty) {
            let message = format!("{} has no methods", self.show_ty(ty));
            self.error(name.span, message);
            return self.error_expr(f, span);
        }
        let Some((method, type_args)) = self.find_method_on_ty(f, ty, name) else { return self.error_expr(f, span) };
        if self.receiver_of(method) == Receiver::None {
            self.error(name.span, format!("`{}` is called on the type", self.text(name.name)));
            return self.error_expr(f, span);
        }
        self.call_with(f, method, &type_args, own_args, Some(object), Args::Written(args), expected, span)
    }

    fn any_to_call(&mut self, f: &mut FnCx, object: Id<Expr>, own_args: &[GenericArg], args: &[ast::Arg], expected: Option<TyId>, span: Span) -> Id<Expr> {
        if !args.is_empty() {
            self.error(span, "`to` takes no arguments");
            return self.error_expr(f, span);
        }
        let target = if own_args.is_empty() {
            match expected.and_then(|ty| self.optional_inner(ty)) {
                Some(inner) => inner,
                None => {
                    self.error(span, "write the type argument of `to`, e.g. `.to<Int>()`");
                    return self.error_expr(f, span);
                }
            }
        } else if own_args.len() == 1 {
            match own_args[0] {
                GenericArg::Type(t) => t,
                GenericArg::Const(_) => {
                    self.error(span, "expected a type argument for `to`");
                    return self.error_expr(f, span);
                }
            }
        } else {
            self.error(span, "`to` takes 1 type argument");
            return self.error_expr(f, span);
        };
        let opt_ty = self.program.types.optional(target);
        let name = self.interner.intern("__any_to");
        f.body.expr(
            ExprKind::Call { callee: Callee::Builtin { name, generic_args: vec![GenericArg::Type(target)] }, args: vec![object] },
            opt_ty,
            span,
        )
    }

    /// A method of a type parameter: one of its bounds' traits supplies it.
    #[allow(clippy::too_many_arguments)]
    fn bound_call(&mut self, f: &mut FnCx, object: Id<Expr>, index: u32, name: Ident, own_args: &[GenericArg], args: &[ast::Arg], expected: Option<TyId>, span: Span) -> Id<Expr> {
        let ty = f.body.exprs[object].ty;
        let Some((method, owner_args)) = self.bound_method(f, ty, index, name) else { return self.error_expr(f, span) };
        if self.receiver_of(method) == Receiver::None {
            self.error(name.span, format!("`{}` is called on the type", self.text(name.name)));
            return self.error_expr(f, span);
        }
        self.call_with(f, method, &owner_args, own_args, Some(object), Args::Written(args), expected, span)
    }

    /// The trait method a name stands for on a type parameter, with the trait's arguments after `Self`.
    fn bound_method(&mut self, f: &FnCx, ty: TyId, index: u32, name: Ident) -> Option<(DefId, Vec<GenericArg>)> {
        let traits = f.bounds.get(index as usize).cloned().unwrap_or_default();
        let found = traits.iter().find_map(|bound| self.methods_named(bound.def, name.name).into_iter().next().map(|method| (method, bound.args.clone())));
        let Some((method, trait_args)) = found else {
            let message = format!("{} has no method `{}`", self.show_ty(ty), self.text(name.name));
            self.error(name.span, message);
            return None;
        };
        let args = std::iter::once(ty).chain(trait_args).map(GenericArg::Type).collect();
        Some((method, args))
    }

    /// `T.name(args)` where `T` is a type parameter: a method without a receiver from one of its bounds' traits.
    #[allow(clippy::too_many_arguments)]
    fn bound_static_call(&mut self, f: &mut FnCx, index: u32, param: Symbol, name: Ident, own_args: &[GenericArg], args: &[ast::Arg], expected: Option<TyId>, span: Span) -> Id<Expr> {
        let ty = f.scope.bound.iter().find(|&&(declared, _)| declared == param).map(|&(_, concrete)| concrete).unwrap_or_else(|| self.program.types.param(index, param));
        let Some((method, owner_args)) = self.bound_method(f, ty, index, name) else { return self.error_expr(f, span) };
        if self.receiver_of(method) != Receiver::None {
            self.error(name.span, format!("`{}` is called on a value", self.text(name.name)));
            return self.error_expr(f, span);
        }
        self.call_with(f, method, &owner_args, own_args, None, Args::Written(args), expected, span)
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
        match self.type_object(f, object) {
            Some(TypeTarget::Def(def, given)) => {
                if let Some(index) = self.variant_index(def, name.name) {
                    let Some(ty) = self.open_instance(f, def, given, expected, span) else { return self.error_expr(f, span) };
                    return self.unit_variant(f, ty, index, span);
                }
                let ty = self.def_type(def, Some(given), span);
                if !self.methods_for_type(ty, name.name).is_empty() {
                    self.error(span, format!("`{}` is a method; call it", self.text(name.name)));
                } else {
                    self.error(span, format!("{} has no `{}`", self.text(self.program.defs[def].name), self.text(name.name)));
                }
                return self.error_expr(f, span);
            }
            Some(TypeTarget::Builtin(ty)) => {
                if self.find_method_on_ty(f, ty, name).is_some() {
                    self.error(span, format!("`{}` is a method; call it", self.text(name.name)));
                }
                return self.error_expr(f, span);
            }
            None => {}
        }
        let object = self.expr(f, object, None);
        self.field(f, object, name, span)
    }

    pub(crate) fn field(&mut self, f: &mut FnCx, object: Id<Expr>, name: Ident, span: Span) -> Id<Expr> {
        let ty = f.body.exprs[object].ty;
        let (def, type_args) = match self.program.types.kind(ty).clone() {
            TyKind::Error => return self.error_expr(f, span),
            TyKind::Adt { def, args } if self.program.defs[def].record().is_some() => (def, args),
            TyKind::Builtin { kind: crate::types::BuiltinTy::Pointer, .. } if self.text(name.name) == "address" => {
                return f.body.expr(ExprKind::Field { object, index: 0 }, TyId::UINT64, span);
            }
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
            if self.methods_for_type(ty, name.name).is_empty() {
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
        let field_ty = self.program.types.subst_generic(declared, &type_args);
        f.body.expr(ExprKind::Field { object, index: index as u32 }, field_ty, span)
    }

    /// `Name { field: value, field, ..base }`.
    pub(crate) fn struct_init(&mut self, f: &mut FnCx, path: &ast::Path, written: &[ast::GenericArg], inits: &[FieldInit], expected: Option<TyId>, span: Span) -> Id<Expr> {
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
        let given = written.iter().map(|g| self.written_generic(f, g)).collect();
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
    pub(crate) fn init_fields(&mut self, f: &mut FnCx, fields: &[Field], module: ModuleId, type_args: &[GenericArg], ty: TyId, inits: &[FieldInit], allow_base: bool, outside: bool, span: Span) -> Option<(Vec<FieldValue>, Option<Id<Expr>>)> {
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
            let field_ty = self.program.types.subst_generic(fields[index].ty, type_args);
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
                    let field_ty = self.program.types.subst_generic(field.ty, type_args);
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
