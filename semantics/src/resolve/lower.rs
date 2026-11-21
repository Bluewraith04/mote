use mote_base::{Id, Span, Symbol};
use mote_syntax as ast;
use mote_syntax::{ExprKind, TypeKind};

use crate::check::FnCx;
use crate::comptime;
use crate::cx::{Progress, Cx};
use crate::defs::{Bound, DefId, DefKind, ModuleId, Resolved};
use crate::types::{BuiltinTy, ConstArg, ConstExpr, ConstVal, FnParam, GenericArg, GenericArg as TypeOrConstArg, IntTy, TyId};

/// The names a type is written against: its module, generic parameters and `Self`.
#[derive(Clone)]
pub(crate) struct Scope {
    pub module: ModuleId,
    pub params: Vec<Symbol>,
    /// Parallel to `params`: `true` if that parameter is a `const` generic parameter.
    pub const_params: Vec<bool>,
    pub self_ty: Option<TyId>,
    /// Names that stand for a fixed type: a copied default method's trait parameters.
    pub bound: Vec<(Symbol, TyId)>,
}

impl Scope {
    pub(crate) fn empty(module: ModuleId) -> Self {
        Self { module, params: Vec::new(), const_params: Vec::new(), self_ty: None, bound: Vec::new() }
    }

    pub(crate) fn is_const_param(&self, index: usize) -> bool {
        self.const_params.get(index).copied().unwrap_or(false)
    }
}

pub(crate) fn primitive(name: &str) -> Option<TyId> {
    Some(match name {
        "Int" | "Int64" => TyId::INT,
        "Int8" => TyId::INT8,
        "Int16" => TyId::INT16,
        "Int32" => TyId::INT32,
        "UInt8" => TyId::UINT8,
        "UInt16" => TyId::UINT16,
        "UInt32" => TyId::UINT32,
        "UInt64" => TyId::UINT64,
        "Float" | "Float64" => TyId::FLOAT,
        "Float32" => TyId::FLOAT32,
        "Bool" => TyId::BOOL,
        "String" => TyId::STRING,
        "Null" => TyId::UNIT,
        "Any" => TyId::ANY,
        _ => return None,
    })
}

impl Cx<'_> {
    pub(crate) fn lower_generic_arg(&mut self, scope: &Scope, arg: &ast::GenericArg) -> GenericArg {
        self.lower_generic_arg_with(scope, None, arg)
    }

    pub(crate) fn lower_generic_arg_with(&mut self, scope: &Scope, mut f: Option<&mut FnCx>, arg: &ast::GenericArg) -> GenericArg {
        match arg {
            ast::GenericArg::Type(ty) => {
                if let TypeKind::Named(path) = self.ast(scope.module)[*ty].kind.clone() {
                    if path.segments.len() == 1 {
                        let sym = path.segments[0].name;
                        if let Some(pos) = scope.params.iter().position(|&p| p == sym)
                            && scope.is_const_param(pos)
                        {
                            return GenericArg::Const(ConstArg::Param { index: pos as u32, name: sym });
                        }
                        if let Some(fcx) = f.as_ref()
                            && let Some((val, _)) = fcx.lookup_const(sym)
                        {
                            return GenericArg::Const(ConstArg::Val(val));
                        }
                    }
                    let names: Vec<Symbol> = path.segments.iter().map(|s| s.name).collect();
                    if let Some(Resolved::Def(def)) = self.quiet_path(scope.module, &names)
                        && matches!(self.program.defs[def].kind, DefKind::Const(_))
                    {
                        let val = self.ensure_const_def(def);
                        return GenericArg::Const(ConstArg::Val(val));
                    }
                }
                GenericArg::Type(self.lower_type_with(scope, f.as_deref_mut(), *ty))
            }
            ast::GenericArg::Expr(expr) => GenericArg::Const(self.lower_const_arg_with(scope, f, *expr)),
        }
    }

    pub(crate) fn lower_const_arg_with(&mut self, scope: &Scope, f: Option<&mut FnCx>, expr: Id<ast::Expr>) -> ConstArg {
        let ast = self.ast(scope.module);
        let node = &ast[expr];
        match &node.kind {
            ExprKind::Paren(inner) => self.lower_const_arg_with(scope, f, *inner),
            ExprKind::Int(literal) => {
                if let Ok(v) = i128::try_from(literal.value) {
                    ConstArg::Val(ConstVal::Int(v, IntTy::I64))
                } else {
                    self.error(node.span, "integer constant out of range");
                    ConstArg::Val(ConstVal::Int(0, IntTy::I64))
                }
            }
            ExprKind::Bool(b) => ConstArg::Val(ConstVal::Bool(*b)),
            ExprKind::String(s) => ConstArg::Val(ConstVal::String(*s)),
            ExprKind::Null => ConstArg::Val(ConstVal::Null),
            ExprKind::Ident(sym) => {
                let sym = *sym;
                let span = node.span;
                if let Some(pos) = scope.params.iter().position(|&p| p == sym) {
                    if scope.is_const_param(pos) {
                        return ConstArg::Param { index: pos as u32, name: sym };
                    }
                    self.error(span, format!("`{}` is a type parameter, expected a constant value", self.text(sym)));
                    return ConstArg::Val(ConstVal::Int(0, IntTy::I64));
                }
                if let Some(fcx) = f.as_ref()
                    && let Some((val, _)) = fcx.lookup_const(sym)
                {
                    return ConstArg::Val(val);
                }
                match self.lookup(scope.module, sym) {
                    Some(Resolved::Def(def)) if matches!(self.program.defs[def].kind, DefKind::Const(_)) => {
                        let val = self.ensure_const_def(def);
                        ConstArg::Val(val)
                    }
                    _ => {
                        self.error(span, format!("`{}` is not a compile-time constant", self.text(sym)));
                        ConstArg::Val(ConstVal::Int(0, IntTy::I64))
                    }
                }
            }
            _ => {
                if self.expr_uses_const_param(scope, expr) {
                    let args = (0..scope.params.len())
                        .map(|i| {
                            let name = scope.params[i];
                            if scope.is_const_param(i) {
                                GenericArg::Const(ConstArg::Param { index: i as u32, name })
                            } else {
                                GenericArg::Type(self.program.types.param(i as u32, name))
                            }
                        })
                        .collect();
                    ConstArg::Expr(ConstExpr { module: scope.module, expr, params: scope.params.clone(), args })
                } else {
                    let result = match f {
                        Some(fcx) => comptime::eval_ast_const_in_fn(self, fcx, expr),
                        None => comptime::eval_ast_const(self, scope.module, expr),
                    };
                    match result {
                        Ok(val) => ConstArg::Val(val),
                        Err(msg) => {
                            self.error(node.span, msg);
                            ConstArg::Val(ConstVal::Int(0, IntTy::I64))
                        }
                    }
                }
            }
        }
    }

    fn expr_uses_const_param(&self, scope: &Scope, expr: Id<ast::Expr>) -> bool {
        let ast = self.ast(scope.module);
        match &ast[expr].kind {
            ExprKind::Ident(sym) => scope.params.iter().enumerate().any(|(i, p)| p == sym && scope.is_const_param(i)),
            ExprKind::Paren(inner) | ExprKind::Unary { operand: inner, .. } => self.expr_uses_const_param(scope, *inner),
            ExprKind::Binary { left, right, .. } => self.expr_uses_const_param(scope, *left) || self.expr_uses_const_param(scope, *right),
            ExprKind::Tuple(els) | ExprKind::List(els) => els.iter().any(|&e| self.expr_uses_const_param(scope, e)),
            ExprKind::If { condition: ast::Condition::Expr(c), then, otherwise }
            | ExprKind::ComptimeIf { condition: ast::Condition::Expr(c), then, otherwise } => {
                self.expr_uses_const_param(scope, *c)
                    || self.expr_uses_const_param(scope, *then)
                    || match otherwise {
                        ast::ElseExpr::If(e) | ast::ElseExpr::Value(e) => self.expr_uses_const_param(scope, *e),
                    }
            }
            _ => false,
        }
    }

    pub(crate) fn lower_type(&mut self, scope: &Scope, ty: Id<ast::Type>) -> TyId {
        self.lower_type_with(scope, None, ty)
    }

    pub(crate) fn lower_type_with(&mut self, scope: &Scope, mut f: Option<&mut FnCx>, ty: Id<ast::Type>) -> TyId {
        let ast = self.ast(scope.module);
        let node = &ast[ty];
        let span = node.span;
        match &node.kind {
            TypeKind::Named(path) => self.lower_path_with(scope, f, path, None, span),
            TypeKind::Generic { path, args } => {
                let lowered: Vec<GenericArg> = args.iter().map(|arg| self.lower_generic_arg_with(scope, f.as_deref_mut(), arg)).collect();
                self.lower_path_with(scope, f, path, Some(lowered), span)
            }
            TypeKind::Builtin { name, generic_args, args } => self.check_type_builtin(scope, f, *name, generic_args, args, span),
            TypeKind::Nullable(inner) => {
                let inner = self.lower_type_with(scope, f, *inner);
                self.program.types.optional(inner)
            }
            TypeKind::Tuple(elements) => {
                let elements = elements.iter().map(|&element| self.lower_type_with(scope, f.as_deref_mut(), element)).collect();
                self.program.types.tuple(elements)
            }
            TypeKind::Array { element, len } => {
                let element = self.lower_type_with(scope, f.as_deref_mut(), *element);
                let Some(len) = len else { return self.program.types.array(element) };
                let len_span = self.ast(scope.module)[*len].span;
                let len_arg = self.lower_const_arg_with(scope, f, *len);
                match &len_arg {
                    ConstArg::Val(ConstVal::Int(v, _)) => {
                        if *v < 0 || *v > i128::from(u32::MAX) {
                            self.error(len_span, "an array length must be a non-negative 32-bit integer");
                            return TyId::ERROR;
                        }
                    }
                    ConstArg::Val(_) => {
                        self.error(len_span, "an array length is an integer");
                        return TyId::ERROR;
                    }
                    ConstArg::Param { .. } | ConstArg::Infer(_) | ConstArg::Expr(_) => {}
                }
                self.program.types.fixed_array_arg(element, len_arg)
            }
            TypeKind::Function { send, params, ret } => {
                let params = params.iter().map(|param| FnParam { ty: self.lower_type_with(scope, f.as_deref_mut(), param.ty), is_var: param.is_var }).collect();
                let ret = self.lower_type_with(scope, f, *ret);
                self.program.types.function(params, ret, *send)
            }
            TypeKind::Union(members) => {
                if let Some(&member) = members.iter().find(|&&member| matches!(ast[member].kind, TypeKind::Nullable(_))) {
                    self.error(ast[member].span, "a union member is not an optional; write `(A | B)?`");
                    return TyId::ERROR;
                }
                let members: Vec<TyId> = members.iter().map(|&member| self.lower_type_with(scope, f.as_deref_mut(), member)).collect();
                self.program.types.union(&members)
            }
            TypeKind::SelfType => scope.self_ty.unwrap_or_else(|| {
                self.error(span, "`Self` is only inside a type");
                TyId::ERROR
            }),
            TypeKind::Paren(inner) => self.lower_type_with(scope, f, *inner),
            TypeKind::Error => TyId::ERROR,
        }
    }

    /// The trait a bound or a trait list names.
    pub(super) fn trait_ref(&mut self, scope: &Scope, ty: Id<ast::Type>) -> Option<Bound> {
        let module = scope.module;
        let node = &self.ast(module)[ty];
        let (path, written): (&ast::Path, &[ast::GenericArg]) = match &node.kind {
            TypeKind::Named(path) => (path, &[]),
            TypeKind::Generic { path, args } => (path, args.as_slice()),
            _ => {
                self.error(node.span, "expected a trait");
                return None;
            }
        };
        let last = path.segments.last().expect("a path has a segment");
        let def = match self.resolve_path(module, path)? {
            Resolved::Def(def) if matches!(self.program.defs[def].kind, DefKind::Trait(_)) => def,
            _ => {
                self.error(path.span, format!("`{}` is not a trait", self.text(last.name)));
                return None;
            }
        };
        let arity = match &self.program.defs[def].kind {
            DefKind::Trait(item) => item.generics.len(),
            _ => 0,
        };
        let name = self.text(last.name);
        if written.len() != arity {
            let message = match (written.len(), arity) {
                (0, _) => format!("write the type arguments of `{name}`"),
                (_, 0) => format!("`{name}` takes no type arguments"),
                (_, 1) => format!("`{name}` takes 1 type argument"),
                (_, count) => format!("`{name}` takes {count} type arguments"),
            };
            self.error(node.span, message);
            return None;
        }
        let args = written
            .iter()
            .filter_map(|arg| match arg {
                ast::GenericArg::Type(t) => Some(self.lower_type(scope, *t)),
                ast::GenericArg::Expr(e) => {
                    self.error(self.ast(module)[*e].span, format!("trait `{name}` expects a type argument"));
                    None
                }
            })
            .collect();
        Some(Bound { def, args })
    }

    /// Follows a dotted name through modules; reports what it cannot find.
    pub(crate) fn resolve_path(&mut self, module: ModuleId, path: &ast::Path) -> Option<Resolved> {
        let first = path.segments[0];
        let Some(mut resolved) = self.lookup(module, first.name) else {
            self.error(first.span, format!("unknown name `{}`", self.text(first.name)));
            return None;
        };
        for segment in &path.segments[1..] {
            let Resolved::Module(target) = resolved else {
                self.error(segment.span, format!("`{}` does not follow a module", self.text(segment.name)));
                return None;
            };
            let Some(&next) = self.program.modules[target].exports.get(&segment.name) else {
                self.missing_name(target, *segment);
                return None;
            };
            resolved = next;
        }
        Some(resolved)
    }

    pub(crate) fn lower_path(&mut self, scope: &Scope, path: &ast::Path, args: Option<Vec<TypeOrConstArg>>, span: Span) -> TyId {
        self.lower_path_with(scope, None, path, args, span)
    }

    pub(crate) fn lower_path_with(&mut self, scope: &Scope, f: Option<&mut FnCx>, path: &ast::Path, args: Option<Vec<TypeOrConstArg>>, span: Span) -> TyId {
        let first = path.segments[0];
        let single = path.segments.len() == 1;
        if single {
            if let Some(&(_, ty)) = scope.bound.iter().find(|(name, _)| *name == first.name) {
                if args.is_some() {
                    self.error(span, format!("`{}` takes no type arguments", self.text(first.name)));
                }
                return ty;
            }
            if let Some(index) = scope.params.iter().position(|&param| param == first.name) {
                if scope.is_const_param(index) {
                    self.error(span, format!("`{}` is a `const` generic parameter, not a type", self.text(first.name)));
                    return TyId::ERROR;
                }
                if args.is_some() {
                    self.error(span, format!("`{}` takes no type arguments", self.text(first.name)));
                }
                return self.program.types.param(index as u32, first.name);
            }
            if let Some(fcx) = f.as_ref()
                && let Some((val, _)) = fcx.lookup_const(first.name)
            {
                if let ConstVal::Type(ty) = val {
                    if args.is_some() {
                        self.error(span, format!("`{}` takes no type arguments", self.text(first.name)));
                    }
                    return ty;
                }
                self.error(span, format!("`{}` is a constant value, not a type", self.text(first.name)));
                return TyId::ERROR;
            }
            if self.lookup(scope.module, first.name).is_none() {
                let first_text = self.text(first.name);
                if let Some(ty) = primitive(&first_text) {
                    if args.is_some() {
                        self.error(span, format!("`{first_text}` takes no type arguments"));
                    }
                    return ty;
                }
                if first_text == "Option" || first_text == "Array" {
                    let arg_list = args.unwrap_or_default();
                    if arg_list.len() != 1 {
                        self.error(span, format!("`{first_text}` takes 1 type argument"));
                        return TyId::ERROR;
                    }
                    let Some(inner) = arg_list[0].as_type() else {
                        self.error(span, format!("`{first_text}` expects a type argument"));
                        return TyId::ERROR;
                    };
                    return if first_text == "Option" {
                        self.program.types.optional(inner)
                    } else {
                        self.program.types.array(inner)
                    };
                }
                if let Some(kind) = BuiltinTy::from_name(&first_text) {
                    let arg_list = args.unwrap_or_default();
                    let arity = kind.arity();
                    if arg_list.len() != arity {
                        let message = match (arg_list.len(), arity) {
                            (0, _) => format!("write the type arguments of `{first_text}`"),
                            (_, 0) => format!("`{first_text}` takes no type arguments"),
                            (_, 1) => format!("`{first_text}` takes 1 type argument"),
                            (_, count) => format!("`{first_text}` takes {count} type arguments"),
                        };
                        self.error(span, message);
                        return TyId::ERROR;
                    }
                    let mut ty_args = Vec::with_capacity(arg_list.len());
                    for arg in arg_list {
                        let Some(t) = arg.as_type() else {
                            self.error(span, format!("`{first_text}` expects a type argument"));
                            return TyId::ERROR;
                        };
                        ty_args.push(t);
                    }
                    return self.program.types.builtin(kind, ty_args);
                }
            }
        }
        match self.resolve_path(scope.module, path) {
            Some(Resolved::Def(def)) => self.def_type(def, args, span),
            Some(Resolved::Module(_)) => {
                let last = path.segments.last().expect("a path has a segment");
                self.error(span, format!("`{}` is a module, not a type", self.text(last.name)));
                TyId::ERROR
            }
            None => TyId::ERROR,
        }
    }

    pub(crate) fn def_type(&mut self, def: DefId, args: Option<Vec<GenericArg>>, span: Span) -> TyId {
        let name = self.text(self.program.defs[def].name);
        if matches!(self.program.defs[def].kind, DefKind::Const(_)) {
            if let ConstVal::Type(ty) = self.ensure_const_def(def) {
                if !args.unwrap_or_default().is_empty() {
                    self.error(span, format!("`{name}` takes no type arguments"));
                }
                return ty;
            }
            self.error(span, format!("`{name}` is not a type"));
            return TyId::ERROR;
        }
        let generics = match &self.program.defs[def].kind {
            DefKind::Record(record) => record.generics.clone(),
            DefKind::Enum(en) => en.generics.clone(),
            DefKind::Alias(alias) => alias.generics.clone(),
            DefKind::Trait(_) => {
                self.error(span, format!("`{name}` is a trait, not a type"));
                return TyId::ERROR;
            }
            _ => {
                self.error(span, format!("`{name}` is not a type"));
                return TyId::ERROR;
            }
        };
        let arity = generics.len();
        let args = args.unwrap_or_default();
        if args.len() != arity {
            let message = match (args.len(), arity) {
                (0, _) => format!("write the type arguments of `{name}`"),
                (_, 0) => format!("`{name}` takes no type arguments"),
                (_, 1) => format!("`{name}` takes 1 type argument"),
                (_, count) => format!("`{name}` takes {count} type arguments"),
            };
            self.error(span, message);
            return TyId::ERROR;
        }
        for (arg, param) in args.iter().zip(&generics) {
            match (arg, param) {
                (GenericArg::Type(_), crate::defs::GenericParam::Const { .. }) => {
                    self.error(span, format!("expected a constant expression for `{}`", self.text(param.name())));
                    return TyId::ERROR;
                }
                (GenericArg::Const(_), crate::defs::GenericParam::Type { .. }) => {
                    self.error(span, format!("expected a type for `{}`", self.text(param.name())));
                    return TyId::ERROR;
                }
                _ => {}
            }
        }
        if matches!(self.program.defs[def].kind, DefKind::Alias(_)) {
            if self.aliases.get(&def) == Some(&Progress::Resolving) {
                self.error(span, format!("type alias `{name}` refers to itself"));
                return TyId::ERROR;
            }
            self.ensure_alias(def);
            let DefKind::Alias(alias) = &self.program.defs[def].kind else { unreachable!("checked above") };
            let target = alias.target;
            return self.program.types.subst_generic(target, &args);
        }
        self.program.types.adt(def, args)
    }
}
