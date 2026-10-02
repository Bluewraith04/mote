use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use crate::ast::*;
use crate::span::Span;
use crate::types::Type;

mod builtin_methods;
mod constructors;
mod generics;
use generics::CallTypes;
mod infer;
mod modes;
mod shared;
mod literals;
mod narrowing;
mod null_safe;
mod privacy;
mod traits;
mod unions;
pub(crate) mod conversions;

#[derive(Clone, Debug)]
pub struct SymbolInfo {
    pub ty: Type,
    pub is_mutable: bool,
    /// The operation that consumed this binding, `into_shared` or `spawn`; a plain reassignment clears it.
    pub is_moved: Option<&'static str>,
    pub read_only: modes::ReadOnly,
}

#[derive(Clone, Debug)]
struct FnSig {
    params: Vec<Type>,
    type_params: Vec<String>,
    bounds: Vec<(String, Vec<String>)>,
    required: usize,
    ret: Type,
    variadic: Option<Type>,
    modes: Vec<(String, bool)>,
}

#[derive(Clone, Debug)]
struct MethodSig {
    type_params: Vec<String>,
    bounds: Vec<(String, Vec<String>)>,
    params: Vec<Type>,
    required: usize,
    ret: Type,
    is_static: bool,
    variadic: Option<Type>,
    modes: Vec<(String, bool)>,
    writes_self: bool,
    is_pub: bool,
}

type PendingInstance = (String, FunctionDecl, Vec<(String, Type)>);

/// Checks a program and records the type of every expression.
pub struct TypeChecker {
    types: HashMap<String, Type>,
    mutable_classes: HashSet<String>,
    var_fields: HashSet<(String, String)>,
    module: String,
    type_module: HashMap<String, String>,
    pub_types: HashSet<String>,
    pub_fields: HashSet<(String, String)>,
    implicit_var_lambda: bool,
    imported_names: HashSet<String>,
    type_params: Vec<String>,
    generics: HashMap<String, Vec<String>>,
    traits: HashMap<String, TraitDecl>,
    param_bounds: HashMap<String, Vec<String>>,
    bounded_fns: HashMap<String, FunctionDecl>,
    param_subst: HashMap<String, Type>,
    instance_calls: HashMap<Span, String>,
    calls: HashMap<Span, CallTypes>,
    instance_seen: HashSet<String>,
    pending_instances: Vec<PendingInstance>,
    instance_done: Vec<(FunctionDecl, crate::stage::TypeTable)>,
    type_errors: RefCell<Vec<(String, Span)>>,
    declared_types: HashSet<String>,
    type_aliases: HashMap<String, TypeAliasDecl>,
    alias_stack: RefCell<Vec<String>>,
    enum_variants: HashMap<String, Vec<(String, Vec<Type>)>>,
    variant_fields: HashMap<(String, String), Vec<String>>,
    fn_sigs: HashMap<String, FnSig>,
    class_method_sigs: HashMap<(String, String), MethodSig>,
    current_self_type: Option<Type>,
    scopes: Vec<HashMap<String, SymbolInfo>>,
    spawn_boundary: Option<usize>,
    spawn_ends: Vec<(String, usize, Span)>,
    loop_boundaries: Vec<usize>,
    in_function_body: bool,
    current_fn_ret: Option<Type>,
    gen_elem: Option<Type>,
    /// The resolved type of every expression the checker visited, keyed by span; codegen resolves field indices from it.
    pub expr_types: HashMap<Span, Type>,
    any_checks: HashMap<Span, Type>,
    type_tests: HashMap<Span, Type>,
    some_lifts: HashMap<Span, u8>,
    narrowed_reads: HashMap<Span, u8>,
    narrowings: Vec<narrowing::Narrowing>,
    call_arg_spans: Vec<Span>,
    lambda_expect: Option<Type>,
    expected: Option<Type>,
    lambda_returns: Option<Vec<Type>>,
    lambda_frames: Vec<(usize, bool)>,
    mixed_literals: Vec<literals::MixedLiteral>,
    pub errors: Vec<(String, Span)>,
}

impl Default for TypeChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeChecker {
    pub fn new() -> Self {
        let mut checker = Self {
            types: HashMap::new(),
            mutable_classes: HashSet::new(),
            var_fields: HashSet::new(),
            module: String::new(),
            type_module: HashMap::new(),
            pub_types: HashSet::new(),
            pub_fields: HashSet::new(),
            implicit_var_lambda: false,
            imported_names: HashSet::new(),
            type_params: Vec::new(),
            generics: HashMap::new(),
            traits: HashMap::new(),
            param_bounds: HashMap::new(),
            bounded_fns: HashMap::new(),
            param_subst: HashMap::new(),
            instance_calls: HashMap::new(),
            calls: HashMap::new(),
            instance_seen: HashSet::new(),
            pending_instances: Vec::new(),
            instance_done: Vec::new(),
            type_errors: RefCell::new(Vec::new()),
            declared_types: HashSet::new(),
            type_aliases: HashMap::new(),
            alias_stack: RefCell::new(Vec::new()),
            enum_variants: HashMap::new(),
            variant_fields: HashMap::new(),
            fn_sigs: HashMap::new(),
            class_method_sigs: HashMap::new(),
            current_self_type: None,
            scopes: vec![HashMap::new()],
            spawn_boundary: None,
            spawn_ends: Vec::new(),
            loop_boundaries: Vec::new(),
            in_function_body: false,
            current_fn_ret: None,
            gen_elem: None,
            expr_types: HashMap::new(),
            any_checks: HashMap::new(),
            type_tests: HashMap::new(),
            some_lifts: HashMap::new(),
            narrowed_reads: HashMap::new(),
            narrowings: Vec::new(),
            call_arg_spans: Vec::new(),
            lambda_expect: None,
            expected: None,
            lambda_returns: None,
            lambda_frames: Vec::new(),
            mixed_literals: Vec::new(),
            errors: Vec::new(),
        };
        checker.register_builtins();
        checker
    }

    fn register_builtins(&mut self) {
        self.types.insert("Int".into(), Type::Int);
        self.types.insert("Float".into(), Type::Float);
        self.types.insert("Bool".into(), Type::Bool);
        self.types.insert("Char".into(), Type::Char);
        self.types.insert("String".into(), Type::String);
        self.types.insert("Null".into(), Type::Null);
        self.types.insert("__Any".into(), Type::Any);
        for decl in crate::builtin_natives::decls() {
            let generic = decl.params.iter().any(|p| matches!(&p.ty, Some(TypeNode::Named(n, _)) if n == "T"));
            let type_params = if generic { vec!["T".to_string()] } else { Vec::new() };
            let mark = self.push_type_param_names(&type_params);
            let params: Vec<Type> = decl
                .params
                .iter()
                .map(|p| p.ty.as_ref().map(|t| self.resolve_type_node(t)).unwrap_or(Type::Any))
                .collect();
            let ret_node = crate::builtin_natives::fallible_payload(decl).or(decl.return_type.as_ref());
            let ret = ret_node.map(|t| self.resolve_type_node(t)).unwrap_or(Type::Null);
            self.pop_type_params(mark);
            self.fn_sigs.insert(decl.name.clone(), FnSig { type_params, bounds: Vec::new(), required: params.len(), params, ret, variadic: None, modes: Vec::new() });
        }
    }

    pub fn check_program(&mut self, program: &Program) -> Result<(), Vec<(String, Span)>> {
        for item in &program.items {
            match item {
                Item::Struct(s) => self.declared_types.insert(s.name.clone()),
                Item::Class(c) => self.declared_types.insert(c.name.clone()),
                Item::Enum(e) => self.declared_types.insert(e.name.clone()),
                Item::Trait(t) => self.declared_types.insert(t.name.clone()),
                Item::TypeAlias(a) => {
                    if self.declared_types.contains(&a.name) || crate::types::is_builtin_type_name(&a.name) {
                        self.errors.push((format!("type `{}` is already defined", a.name), a.span));
                    }
                    self.type_aliases.insert(a.name.clone(), a.clone());
                    self.declared_types.insert(a.name.clone())
                }
                _ => false,
            };
        }
        for item in &program.items {
            if let Item::Function(f) = item
                && (self.declared_types.contains(&f.name) || crate::types::is_builtin_type_name(&f.name)) {
                    self.errors.push((format!("function `{}` has the same name as a type", f.name), f.span));
                }
        }

        for item in &program.items {
            let (name, params) = match item {
                Item::Struct(s) => (&s.name, &s.generic_params),
                Item::Class(c) => (&c.name, &c.generic_params),
                Item::Enum(e) => (&e.name, &e.generic_params),
                _ => continue,
            };
            let args = self.declare_generics(name, params);
            let shell = match item {
                Item::Struct(_) => Type::Struct { name: name.clone(), args, fields: Vec::new() },
                Item::Class(_) => Type::Class { name: name.clone(), args, fields: Vec::new() },
                _ => Type::Enum { name: name.clone(), args, variants: Vec::new() },
            };
            self.types.insert(name.clone(), shell);
        }
        for item in &program.items {
            match item {
                Item::Struct(s) => self.register_struct_type(s),
                Item::Class(c) => self.register_class_type(c),
                Item::Enum(e) => self.register_enum_type(e),
                Item::Trait(t) => self.register_trait(t),
                _ => {}
            }
        }
        for item in &program.items {
            if let Item::TypeAlias(a) = item {
                let mark = self.push_type_params(&a.generic_params);
                self.alias_stack.borrow_mut().push(a.name.clone());
                let _ = self.resolve_type_node(&a.target);
                self.alias_stack.borrow_mut().pop();
                self.pop_type_params(mark);
            }
        }

        for item in &program.items {
            if let Item::Struct(s) = item {
                self.check_struct_structural_stickiness(s);
            }
        }

        for item in &program.items {
            match item {
                Item::Function(f) => self.register_fn_sig(f),
                Item::Class(c) => self.register_method_sigs(&c.name, &c.methods),
                Item::Struct(st) => self.register_method_sigs(&st.name, &st.methods),
                Item::Enum(e) => {
                    self.check_enum_methods_shape(e);
                    self.register_method_sigs(&e.name, &e.methods)
                }
                Item::NativeFunction(n) => {
                    self.errors.push(("`native fn` is only for the compiler's own builtins".into(), n.span));
                }
                Item::Import(d) => {
                    let names = d.symbols.iter().map(|s| s.alias.clone().unwrap_or_else(|| s.name.clone()));
                    self.imported_names.extend(names);
                }
                _ => {}
            }
        }
        for item in &program.items {
            match item {
                Item::Struct(s) => self.check_declared_traits(&s.name, &s.traits),
                Item::Class(c) => self.check_declared_traits(&c.name, &c.traits),
                Item::Enum(e) => self.check_declared_traits(&e.name, &e.traits),
                _ => {}
            }
        }
        self.finish_fallible_natives();

        for item in &program.items {
            if let Item::TopLevelStmt(stmt) = item {
                let (name, ty_node, is_mut) = match stmt {
                    Stmt::Let { name, ty, .. } => (name, ty, false),
                    Stmt::Var { name, ty, .. } => (name, ty, true),
                    _ => continue,
                };
                let ty = ty_node.as_ref().map(|t| self.resolve_type_node(t)).unwrap_or(Type::Any);
                self.insert_symbol(name, ty, is_mut);
            }
        }

        for item in &program.items {
            match item {
                Item::Function(f) => self.check_function(f),
                Item::Class(c) => {
                    self.check_field_method_clash(c);
                    self.check_type_methods(&c.name, &c.methods)
                }
                Item::Struct(st) => self.check_type_methods(&st.name, &st.methods),
                Item::Enum(e) => self.check_type_methods(&e.name, &e.methods),
                Item::TopLevelStmt(stmt) => {
                    self.check_stmt(stmt);
                    if let Stmt::Let { name, is_pub: true, span, .. } | Stmt::Var { name, is_pub: true, span, .. } = stmt
                        && let Some(info) = self.lookup_symbol(name) {
                            self.check_exposed(name, &[&info.ty], *span);
                        }
                }
                _ => {}
            }
        }

        self.check_pending_instances();
        self.report_mixed_literals();
        self.flush_type_errors();
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors.clone())
        }
    }

    fn register_struct_type(&mut self, s: &StructDecl) {
        self.reject_bounds(&s.generic_params);
        let mark = self.push_type_params(&s.generic_params);
        let mut fields = Vec::new();
        for f in &s.fields {
            let ty = self.resolve_type_node(&f.ty);
            fields.push((f.name.clone(), ty));
            if f.is_mutable {
                self.var_fields.insert((s.name.clone(), f.name.clone()));
            }
        }
        self.pop_type_params(mark);
        self.note_type_privacy(&s.name, s.is_pub, &s.fields);
        self.check_pub_field_types(&s.name, &s.fields, &fields);
        let args = self.declare_generics(&s.name, &s.generic_params);
        self.types.insert(s.name.clone(), Type::Struct { name: s.name.clone(), args, fields });
    }

    fn register_class_type(&mut self, c: &ClassDecl) {
        self.reject_bounds(&c.generic_params);
        let mark = self.push_type_params(&c.generic_params);
        let mut fields = Vec::new();
        for f in &c.fields {
            let ty = self.resolve_type_node(&f.ty);
            fields.push((f.name.clone(), ty));
            if f.is_mutable {
                self.var_fields.insert((c.name.clone(), f.name.clone()));
            }
        }
        self.pop_type_params(mark);
        if c.fields.iter().any(|f| f.is_mutable) {
            self.mutable_classes.insert(c.name.clone());
        }
        self.note_type_privacy(&c.name, c.is_pub, &c.fields);
        self.check_pub_field_types(&c.name, &c.fields, &fields);
        let args = self.declare_generics(&c.name, &c.generic_params);
        self.types.insert(c.name.clone(), Type::Class { name: c.name.clone(), args, fields });
    }

    fn register_enum_type(&mut self, e: &EnumDecl) {
        self.reject_bounds(&e.generic_params);
        let mark = self.push_type_params(&e.generic_params);
        let mut variants: Vec<(String, Vec<Type>)> = Vec::new();
        for v in &e.variants {
            let payload: Vec<Type> = match &v.kind {
                EnumVariantKind::Unit { .. } => Vec::new(),
                EnumVariantKind::Tuple(tys) => {
                    tys.iter().map(|t| self.resolve_type_node(t)).collect()
                }
                EnumVariantKind::Struct(fields) => {
                    let names = fields.iter().map(|f| f.name.clone()).collect();
                    self.variant_fields.insert((e.name.clone(), v.name.clone()), names);
                    fields.iter().map(|f| self.resolve_type_node(&f.ty)).collect()
                }
            };
            variants.push((v.name.clone(), payload.clone()));
            self.enum_variants
                .entry(v.name.clone())
                .or_default()
                .push((e.name.clone(), payload));
        }
        self.pop_type_params(mark);
        self.note_type_privacy(&e.name, e.is_pub, &[]);
        let args = self.declare_generics(&e.name, &e.generic_params);
        self.types.insert(e.name.clone(), Type::Enum { name: e.name.clone(), args, variants });
    }

    fn resolve_variant(
        &mut self,
        enum_hint: Option<&str>,
        variant: &str,
        span: Span,
    ) -> Option<(String, Vec<Type>)> {
        let candidates = self.enum_variants.get(variant)?.clone();
        if let Some(hint) = enum_hint {
            return candidates.into_iter().find(|(en, _)| en == hint);
        }
        match candidates.len() {
            0 => None,
            1 => Some(candidates.into_iter().next().unwrap()),
            _ => {
                let names: Vec<&str> = candidates.iter().map(|(en, _)| en.as_str()).collect();
                self.errors.push((
                    format!(
                        "Ambiguous variant '{}' — declared by {}; qualify it as '<Enum>.{}'",
                        variant,
                        names.join(", "),
                        variant
                    ),
                    span,
                ));
                None
            }
        }
    }

    fn qualified_variant_ref(&self, e: &Expr) -> Option<(String, Span, String)> {
        match e {
            Expr::MemberAccess { object, member, span } => {
                if let Expr::Ident(name, _) = object.as_ref()
                    && matches!(self.types.get(name), Some(Type::Enum { .. })) && !self.class_method_sigs.contains_key(&(name.clone(), member.clone())) {
                        return Some((name.clone(), *span, member.clone()));
                    }
                None
            }
            Expr::StaticAccess { target: TypeNode::Named(name, _), member, span } => {
                if matches!(self.types.get(name), Some(Type::Enum { .. })) && !self.class_method_sigs.contains_key(&(name.clone(), member.clone())) {
                    Some((name.clone(), *span, member.clone()))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn check_variant_payload(&mut self, enum_name: &str, variant: &str, payload: &[Type], args: &[Type], expected: Option<&Type>, span: Span) -> Type {
        let names = self.generics.get(enum_name).cloned().unwrap_or_default();
        let mut bind = HashMap::new();
        if let (Some(exp), Some(template)) = (expected, self.types.get(enum_name)) {
            Self::unify(template, exp, &names, &mut bind);
        }
        for (p, a) in payload.iter().zip(args) {
            Self::unify(p, a, &names, &mut bind);
        }
        if payload.len() != args.len() {
            self.errors.push((
                format!("Variant '{}' takes {} value(s), but {} were given", variant, payload.len(), args.len()),
                span,
            ));
        } else {
            for (i, (got, want)) in args.iter().zip(payload).enumerate() {
                let want = Self::subst(want, &names, &bind);
                if !got.is_assignable_to(&want) {
                    self.errors.push((
                        format!("Variant '{}' value {} expects '{:?}', found '{:?}'", variant, i + 1, want, got),
                        span,
                    ));
                }
            }
        }
        let bound: Vec<Type> = names.iter().map(|n| bind.get(n).cloned().unwrap_or(Type::Hole)).collect();
        self.instantiate_type(enum_name, &bound)
    }
    fn check_struct_structural_stickiness(&mut self, s: &StructDecl) {
        let mark = self.push_type_params(&s.generic_params);
        for f in &s.fields {
            let ty = self.resolve_type_node(&f.ty);
            let issue = self.struct_field_issue(&ty, &s.name, &mut Vec::new());
            if issue == Some(true) {
                self.errors.push((
                    format!("struct `{}` contains itself through field `{}`, so it would have infinite size; make it a class", s.name, f.name),
                    f.span,
                ));
            } else if issue == Some(false) {
                self.errors.push((format!("struct `{}` field `{}` is '{:?}', which can change; make `{}` a class", s.name, f.name, ty, s.name), f.span));
            }
        }
        self.pop_type_params(mark);
    }

    fn struct_field_issue(&self, ty: &Type, owner: &str, seen: &mut Vec<String>) -> Option<bool> {
        match self.full_type(ty.clone()) {
            Type::Struct { name, fields, .. } => {
                if name == owner {
                    return Some(true);
                }
                if seen.contains(&name) {
                    return None;
                }
                seen.push(name);
                fields.iter().find_map(|(_, f)| self.struct_field_issue(f, owner, seen))
            }
            Type::Nullable(inner) => self.struct_field_issue(&inner, owner, seen),
            other => self.can_change(&other).then_some(false),
        }
    }

    fn can_change(&self, ty: &Type) -> bool {
        match ty {
            Type::List(_) | Type::Map(..) | Type::Set(_) | Type::Bytes | Type::Stream(_) => true,
            Type::Class { name, .. } => self.mutable_classes.contains(name),
            Type::Nullable(inner) => self.can_change(inner),
            Type::Tuple(ts) => ts.iter().any(|t| self.can_change(t)),
            Type::Union(crate::types::Members(ms)) => ms.iter().any(|t| self.can_change(t)),
            _ => false,
        }
    }

    fn register_fn_sig(&mut self, f: &FunctionDecl) {
        let bounds = self.collect_bounds(&f.generic_params);
        let mark = self.push_type_params(&f.generic_params);
        let params: Vec<Type> = f
            .params
            .iter()
            .filter(|p| !matches!(p.kind, Some(ParameterKind::Variadic { .. })))
            .map(|p| p.ty.as_ref().map(|t| self.resolve_type_node(t)).unwrap_or(Type::Any))
            .collect();
        let ret = f
            .return_type
            .as_ref()
            .map(|t| self.resolve_type_node(t))
            .unwrap_or(Type::Null);
        let (required, variadic) = self.required_arity(&f.params);
        self.pop_type_params(mark);
        let type_params = f.generic_params.iter().map(|p| p.name.clone()).collect();
        if !bounds.is_empty() {
            self.bounded_fns.insert(f.name.clone(), f.clone());
        }
        if f.is_pub {
            let tys: Vec<&Type> = std::iter::once(&ret).chain(&params).collect();
            self.check_exposed(&format!("fn {}", f.name), &tys, f.span);
        }
        self.fn_sigs.insert(f.name.clone(), FnSig { type_params, bounds, params, required, ret, variadic, modes: modes::param_modes(&f.params) });
    }

    fn required_arity(&mut self, params: &[Param]) -> (usize, Option<Type>) {
        let mut required = None;
        let mut variadic = None;
        let mut count = 0;
        let real_params: Vec<&Param> = params.iter().filter(|p| !matches!(p.kind, Some(ParameterKind::SelfValue { .. }))).collect();
        let last = real_params.len().saturating_sub(1);
        for (idx, p) in real_params.iter().enumerate() {
            if let Some(ParameterKind::Variadic { ty, .. }) = &p.kind {
                if idx != last {
                    self.errors.push((format!("`...{}` must be the last parameter", p.name), p.span));
                }
                if required.is_some() {
                    self.errors.push((
                        format!("`...{}` cannot follow a default parameter", p.name),
                        p.span,
                    ));
                }
                variadic = Some(ty.as_ref().map(|t| self.resolve_type_node(t)).unwrap_or(Type::Any));
                continue;
            }
            let has_default = matches!(&p.kind, Some(ParameterKind::Regular { default_value: Some(_), .. }));
            if has_default && p.is_mut {
                self.errors.push((format!("`var {}` cannot have a default", p.name), p.span));
            }
            if has_default {
                required.get_or_insert(count);
            } else if required.is_some() {
                self.errors.push((format!("parameter `{}` needs a default: a parameter after a default must have one", p.name), p.span));
            }
            count += 1;
        }
        (required.unwrap_or(count), variadic)
    }

    fn check_param_defaults(&mut self, f: &FunctionDecl) {
        for p in &f.params {
            let Some(ParameterKind::Regular { default_value: Some(default), .. }) = &p.kind else { continue };
            let want = p.ty.as_ref().map(|t| self.resolve_type_node(t)).unwrap_or(Type::Any);
            let got = self.check_expr(default);
            if !got.is_assignable_to(&want) {
                self.errors.push((format!("default for `{}` expects '{:?}', found '{:?}'", p.name, want, got), p.span));
            }
            self.note_coercion(default.span(), &got, &want);
        }
    }

    fn check_field_method_clash(&mut self, c: &ClassDecl) {
        for m in &c.methods {
            if c.fields.iter().any(|f| f.name == m.name) {
                self.errors.push((format!("`{}` is both a field and a method of `{}`", m.name, c.name), m.span));
            }
        }
    }

    fn arity_error(what: &str, required: usize, total: usize, given: usize) -> Option<String> {
        if given <= total && given >= required {
            return None;
        }
        let takes = if required == total { format!("{total}") } else { format!("{required} to {total}") };
        Some(format!("{what} takes {takes} argument(s), but {given} were given"))
    }

    fn register_method_sigs(&mut self, type_name: &str, methods: &[FunctionDecl]) {
        let class_params = self.generics.get(type_name).cloned().unwrap_or_default();
        let class_mark = self.push_type_param_names(&class_params);
        let prev_self = self.current_self_type.replace(self.types.get(type_name).cloned().unwrap_or(Type::Any));
        self.register_method_sigs_in_scope(type_name, methods);
        self.current_self_type = prev_self;
        self.pop_type_params(class_mark);
    }

    fn check_enum_methods_shape(&mut self, e: &EnumDecl) {
        for m in &e.methods {
            if e.variants.iter().any(|v| v.name == m.name) {
                self.errors.push((format!("`{}` has a variant named `{}`, so a method cannot have that name", e.name, m.name), m.span));
            }
            if modes::writes_self(&m.params) {
                self.errors.push((format!("`{}` is an enum: a value is replaced, not changed, so `var self` is not allowed; return the new value", m.name), m.span));
            }
        }
    }

    fn register_method_sigs_in_scope(&mut self, type_name: &str, methods: &[FunctionDecl]) {
        for m in methods {
            let is_static = !matches!(
                m.params.first().and_then(|p| p.kind.as_ref()),
                Some(ParameterKind::SelfValue { .. })
            );
            let bounds = self.collect_bounds(&m.generic_params);
            let mark = self.push_type_params(&m.generic_params);
            let params: Vec<Type> = m
                .params
                .iter()
                .filter(|p| !matches!(p.kind, Some(ParameterKind::SelfValue { .. }) | Some(ParameterKind::Variadic { .. })))
                .map(|p| p.ty.as_ref().map(|t| self.resolve_type_node(t)).unwrap_or(Type::Any))
                .collect();
            let ret = m
                .return_type
                .as_ref()
                .map(|t| self.resolve_type_node(t))
                .unwrap_or(Type::Null);
            let (required, variadic) = self.required_arity(&m.params);
            self.pop_type_params(mark);
            let type_params = m.generic_params.iter().map(|p| p.name.clone()).collect();
            if m.is_pub && self.pub_types.contains(type_name) {
                let tys: Vec<&Type> = std::iter::once(&ret).chain(&params).collect();
                self.check_exposed(&format!("{type_name}.{}", m.name), &tys, m.span);
            }
            self.class_method_sigs
                .insert((type_name.to_string(), m.name.clone()), MethodSig { type_params, bounds, params, required, ret, is_static, variadic, modes: modes::param_modes(&m.params), writes_self: modes::writes_self(&m.params), is_pub: m.is_pub });
        }
    }

    fn check_function(&mut self, f: &FunctionDecl) {
        let mark = self.push_type_params(&f.generic_params);
        self.check_function_in_scope(f);
        self.pop_type_params(mark);
    }

    fn check_function_in_scope(&mut self, f: &FunctionDecl) {
        let prev_ret = self.current_fn_ret.take();
        self.current_fn_ret = f
            .return_type
            .as_ref()
            .map(|t| self.resolve_type_node(t));
        let prev_gen = self.gen_elem.take();
        if stmts_yield(&f.body) {
            match self.current_fn_ret.replace(Type::Null) {
                Some(Type::Stream(elem)) => self.gen_elem = Some(*elem),
                _ => {
                    self.errors.push((
                        format!("E0704: `{}` contains `yield`, so it must be declared `-> Stream<T>`", f.name),
                        f.span,
                    ));
                    self.gen_elem = Some(Type::Any);
                }
            }
        }
        let prev_in_fn = std::mem::replace(&mut self.in_function_body, true);

        self.check_param_defaults(f);
        self.push_scope();
        for p in &f.params {
            let ty = if matches!(p.kind, Some(ParameterKind::SelfValue { .. })) {
                self.current_self_type.clone().unwrap_or(Type::Any)
            } else if let Some(ParameterKind::Variadic { ty: vty, .. }) = &p.kind {
                Type::List(Box::new(vty.as_ref().map(|t| self.resolve_type_node(t)).unwrap_or(Type::Any)))
            } else {
                p.ty.as_ref().map(|t| self.resolve_type_node(t)).unwrap_or(Type::Any)
            };
            let is_self = matches!(p.kind, Some(ParameterKind::SelfValue { .. }));
            self.insert_symbol(&p.name, ty, p.is_mut && !is_self);
            if !p.is_mut {
                self.set_read_only(&p.name, modes::ReadOnly::Param);
            }
        }
        for stmt in &f.body {
            self.check_stmt(stmt);
        }
        self.pop_scope();

        self.current_fn_ret = prev_ret;
        self.gen_elem = prev_gen;
        self.in_function_body = prev_in_fn;
    }

    fn check_type_methods(&mut self, type_name: &str, methods: &[FunctionDecl]) {
        let prev_self = self.current_self_type.replace(
            self.types.get(type_name).cloned().unwrap_or(Type::Any),
        );
        let class_params = self.generics.get(type_name).cloned().unwrap_or_default();
        let mark = self.push_type_param_names(&class_params);
        for m in methods {
            self.check_from_decl(type_name, m);
            self.check_function(m);
        }
        self.pop_type_params(mark);
        self.current_self_type = prev_self;
    }

    fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { name, ty, init, span, .. } => {
                let declared = ty.as_ref().map(|t| self.resolve_type_node(t));
                let init_ty = self.check_expr_expecting(init, declared.clone());
                let declared_ty = declared.unwrap_or_else(|| init_ty.clone());

                if !init_ty.is_assignable_to(&declared_ty) {
                    self.errors.push((
                        format!("Type mismatch: cannot initialize variable '{}' of type '{:?}' with expression of type '{:?}'", name, declared_ty, init_ty),
                        *span,
                    ));
                } else {
                    self.note_coercion(init.span(), &init_ty, &declared_ty);
                }

                self.reject_hole("let", name, &declared_ty, *span);
                self.insert_symbol(name, declared_ty.clone(), false);
                self.inherit_read_only(name, init, &declared_ty);
            }
            Stmt::Var { name, ty, init, span, .. } => {
                let declared = ty.as_ref().map(|t| self.resolve_type_node(t));
                let init_ty = self.check_expr_expecting(init, declared.clone());
                let declared_ty = declared.unwrap_or_else(|| match init_ty.clone() {
                    Type::Function { params, ret, modes, .. } => Type::Function { params, ret, sendable: false, modes },
                    other => other,
                });

                if !init_ty.is_assignable_to(&declared_ty) {
                    self.errors.push((
                        format!("Type mismatch: cannot initialize variable '{}' of type '{:?}' with expression of type '{:?}'", name, declared_ty, init_ty),
                        *span,
                    ));
                } else {
                    self.note_coercion(init.span(), &init_ty, &declared_ty);
                }

                self.reject_hole("var", name, &declared_ty, *span);
                self.insert_symbol(name, declared_ty.clone(), true);
                self.inherit_read_only(name, init, &declared_ty);
            }
            Stmt::TupleLet { names, is_mut, init, span, .. } => {
                if !self.in_function_body {
                    self.errors.push((
                        "tuple destructuring (`let (a, b) = ...`) is only supported inside a function body".into(),
                        *span,
                    ));
                }
                let init_ty = self.check_expr(init);
                let elem_tys: Vec<Type> = match &init_ty {
                    Type::Tuple(tys) if tys.len() == names.len() => tys.clone(),
                    Type::Tuple(tys) => {
                        self.errors.push((
                            format!("tuple destructuring has {} name(s), expected {}", names.len(), tys.len()),
                            *span,
                        ));
                        vec![Type::Any; names.len()]
                    }
                    Type::Any => vec![Type::Any; names.len()],
                    _ => {
                        self.errors.push((
                            format!("cannot destructure a tuple pattern from '{:?}'", init_ty),
                            *span,
                        ));
                        vec![Type::Any; names.len()]
                    }
                };
                for (name, ty) in names.iter().zip(elem_tys) {
                    if name != "_" {
                        self.reject_hole(if *is_mut { "var" } else { "let" }, name, &ty, *span);
                        self.insert_symbol(name, ty, *is_mut);
                    }
                }
            }
            Stmt::WithBlock { name, init, body, span } => {
                if !self.in_function_body {
                    self.errors.push((
                        "`with` is only supported inside a function body".into(),
                        *span,
                    ));
                }
                let init_ty = self.check_expr(init);
                if let Some(why) = self.closeable_mismatch(&init_ty) {
                    let msg = if init_ty == Type::Any {
                        format!("`with` needs a `Closeable` resource: {why}")
                    } else {
                        format!("`with` needs a `Closeable` resource: `{}` {}", Self::describe(&init_ty), why)
                    };
                    self.errors.push((msg, *span));
                }
                self.push_scope();
                self.insert_symbol(name, init_ty, false);
                for s in body { self.check_stmt(s); }
                self.pop_scope();
            }
            Stmt::Assign { target, value, span } => {
                let val_ty = self.check_expr(value);

                match target {
                    Expr::Ident(name, i_span) => {
                        if let Some((info, depth)) = self.lookup_symbol_depth(name) {
                            let val_ty = self.settle(value, &info.ty, val_ty.clone());
                            if !info.is_mutable {
                                self.errors.push((
                                    Self::assign_msg(name, &info),
                                    *i_span,
                                ));
                            }
                            if !val_ty.is_assignable_to(&info.ty) {
                                self.errors.push((
                                    format!("Type mismatch: cannot assign value of type '{:?}' to variable '{}' of type '{:?}'", val_ty, name, info.ty),
                                    *span,
                                ));
                            }
                            self.note_coercion(value.span(), &val_ty, &info.ty);
                            if depth == 0 && self.in_function_body {
                                self.errors.push((
                                    format!(
                                        "cannot assign to global '{}' outside its owning \
                                         module's top-level init — module-level state \
                                         becomes read-only once the module finishes loading",
                                        name
                                    ),
                                    *i_span,
                                ));
                            }
                            self.set_moved(name, None);
                        } else {
                            self.errors.push((format!("Undefined variable '{}'", name), *i_span));
                        }
                    }
                    Expr::MemberAccess { object, member, span: m_span } => {
                        let obj_ty = self.check_expr(object);
                        self.check_writable(target, *m_span);
                        if infer::is_scalar(&obj_ty) {
                            self.errors.push((format!("`{:?}` has no field '{}'", obj_ty, member), *m_span));
                        } else {
                            let field_ty = self.check_expr(target);
                            let val_ty = self.settle(value, &field_ty, val_ty.clone());
                            if !val_ty.is_assignable_to(&field_ty) {
                                self.errors.push((format!("field '{member}' is '{field_ty:?}', found '{val_ty:?}'"), value.span()));
                            }
                            self.note_coercion(value.span(), &val_ty, &field_ty);
                        }
                    }
                    Expr::Index { object, index, span: x_span } => {
                        let obj_ty = self.check_expr(object);
                        self.check_writable(object, *x_span);
                        let idx_ty = self.check_expr(index);
                        self.call_arg_spans = vec![index.span(), value.span()];
                        self.check_index(&obj_ty, idx_ty, Some(val_ty), *x_span);
                    }
                    Expr::SelfValue(s_span) => self.errors.push(("cannot assign to `self`; assign its fields".into(), *s_span)),
                    _ => {}
                }
            }
            Stmt::If { cond, then_branch, else_branch, .. } => {
                let cond_ty = self.check_expr(cond);
                if cond_ty != Type::Bool && cond_ty != Type::Any {
                    self.errors.push((format!("Condition in if statement must be Bool, found '{:?}'", cond_ty), cond.span()));
                }
                self.note_coercion(cond.span(), &cond_ty, &Type::Bool);
                let facts = self.facts(cond);
                self.push_scope();
                self.assume(facts.when_true.clone());
                for s in then_branch { self.check_stmt(s); }
                self.pop_scope();

                if let Some(else_b) = else_branch {
                    self.push_scope();
                    self.assume(facts.when_false.clone());
                    for s in else_b { self.check_stmt(s); }
                    self.pop_scope();
                }
                if Self::always_exits(then_branch) {
                    self.assume(facts.when_false);
                } else if else_branch.as_deref().is_some_and(Self::always_exits) {
                    self.assume(facts.when_true);
                }
            }
            Stmt::While { cond, body, .. } => {
                let cond_ty = self.check_expr(cond);
                if cond_ty != Type::Bool && cond_ty != Type::Any {
                    self.errors.push((format!("Condition in while loop must be Bool, found '{:?}'", cond_ty), cond.span()));
                }
                self.note_coercion(cond.span(), &cond_ty, &Type::Bool);
                self.loop_boundaries.push(self.scopes.len());
                self.push_scope();
                for s in body { self.check_stmt(s); }
                self.pop_scope();
                self.loop_boundaries.pop();
            }
            Stmt::ForIn { var_name, iter, body, .. } => {
                let iter_ty = self.check_expr(iter);
                if matches!(iter_ty, Type::Struct { .. } | Type::Class { .. } | Type::Enum { .. }) {
                    self.errors.push((
                        format!("cannot loop over a `{}`: loop over a range, a list, a map's keys or a Stream from a generator", Self::describe(&iter_ty)),
                        iter.span(),
                    ));
                }
                let elem_ty = match iter {
                    Expr::Range { .. } => Type::Int,
                    _ => match iter_ty {
                        Type::List(elem) | Type::Set(elem) | Type::Receiver(elem) | Type::Stream(elem) => *elem,
                        _ => Type::Any,
                    },
                };
                self.loop_boundaries.push(self.scopes.len());
                self.push_scope();
                self.bind_read_only(var_name, elem_ty);
                for s in body { self.check_stmt(s); }
                self.pop_scope();
                self.loop_boundaries.pop();
            }
            Stmt::ScopeBlock { body, .. } => {
                self.push_scope();
                for s in body { self.check_stmt(s); }
                self.pop_scope();
            }
            Stmt::SpawnBlock { body, .. } => {
                self.check_spawn_body(body);
            }
            Stmt::CompoundAssign { target, op, value, span } => {
                let target_ty = self.check_expr(target);
                match target {
                    Expr::MemberAccess { .. } => self.check_writable(target, target.span()),
                    Expr::Index { object, .. } => self.check_writable(object, target.span()),
                    _ => {}
                }
                if *op == AssignOp::NullCoalesceAssign {
                    self.coalesce_assign(target, &target_ty, value);
                } else {
                    let val_ty = self.check_expr(value);
                    if let Some(bin_op) = op.binary_op() {
                        self.arithmetic_type(bin_op, &target_ty, &val_ty, *span);
                    }
                }
                if let Expr::Ident(name, i_span) = target
                    && let Some((info, depth)) = self.lookup_symbol_depth(name) {
                        if !info.is_mutable {
                            self.errors.push((
                                Self::assign_msg(name, &info),
                                *i_span,
                            ));
                        }
                        if depth == 0 && self.in_function_body {
                            self.errors.push((
                                format!(
                                    "cannot assign to global '{}' outside its owning \
                                     module's top-level init — module-level state \
                                     becomes read-only once the module finishes loading",
                                    name
                                ),
                                *i_span,
                            ));
                        }
                    }
            }
            Stmt::Match { expr, arms, span } => {
                let subj_ty = self.check_expr(expr);
                let mut has_catchall = false;
                let mut bools_seen = 0u8;
                let mut covered: HashSet<String> = HashSet::new();
                let mut covered_types: Vec<Type> = Vec::new();

                for arm in arms {
                    if let Some((msg, sp)) = Self::unsupported_pattern(&arm.pattern) {
                        self.errors.push((msg, sp));
                    }
                    if arm.guard.is_none() {
                        self.classify_pattern(&arm.pattern, &subj_ty, &mut has_catchall, &mut bools_seen, &mut covered);
                    }

                    self.push_scope();
                    self.bind_pattern_names(&arm.pattern, &subj_ty);
                    if arm.guard.is_none() {
                        covered_types.extend(self.type_pattern_targets(&arm.pattern));
                    }
                    if let Some(guard) = &arm.guard {
                        let gt = self.check_expr(guard);
                        if gt != Type::Bool && gt != Type::Any {
                            self.errors.push((
                                format!("Match guard must be Bool, found '{:?}'", gt),
                                guard.span(),
                            ));
                        }
                        self.note_coercion(guard.span(), &gt, &Type::Bool);
                    }
                    for s in &arm.body {
                        self.check_stmt(s);
                    }
                    self.pop_scope();
                }

                let bool_exhaustive = subj_ty == Type::Bool && bools_seen == 0b11;
                let enum_exhaustive = Self::variants_of(&subj_ty).is_some_and(|vs| vs.iter().all(|(v, _)| covered.contains(v)));
                let uncovered = (!covered_types.is_empty() && subj_ty != Type::Any)
                    .then(|| self.uncovered_members(&subj_ty, &covered_types, covered.contains("None")));
                if let (false, Some(missing)) = (has_catchall, &uncovered) {
                    if let Some(m) = missing.first() {
                        let shown = if *m == Type::Null { "None".to_string() } else { format!("{m:?}") };
                        let msg = format!("Non-exhaustive `match`: `{shown}` is not covered; add an arm for it or a `_` arm");
                        self.errors.push((msg, *span));
                    }
                } else if !has_catchall && !bool_exhaustive && !enum_exhaustive {
                    let hint = if Self::variants_of(&subj_ty).is_some() {
                        "Non-exhaustive `match`: cover every variant or add a `_` arm"
                    } else {
                        "Non-exhaustive `match`: add a `_` arm"
                    };
                    self.errors.push((hint.to_string(), *span));
                }
            }
            Stmt::Break { value, .. } => {
                if let Some(v) = value {
                    self.check_expr(v);
                }
            }
            Stmt::Continue { .. } => {}
            Stmt::Yield { value, span } => {
                let got = self.check_expr(value);
                match self.gen_elem.clone() {
                    Some(elem) if !got.is_assignable_to(&elem) => {
                        self.errors.push((
                            format!("`yield` expects '{elem:?}' (the stream's element type), found '{got:?}'"),
                            *span,
                        ));
                    }
                    Some(elem) => self.note_coercion(value.span(), &got, &elem),
                    None => {
                        self.errors.push((
                            "E0704: `yield` outside a generator function's own body".into(),
                            *span,
                        ));
                    }
                }
            }
            Stmt::Block { body, .. } => {
                self.push_scope();
                for s in body { self.check_stmt(s); }
                self.pop_scope();
            }
            Stmt::Return { value, span, .. } => {
                let val_ty = value.as_ref().map(|v| self.check_expr_expecting(v, self.current_fn_ret.clone()));
                if self.current_fn_ret.is_none()
                    && let Some(returns) = self.lambda_returns.as_mut() {
                        returns.push(val_ty.clone().unwrap_or(Type::Null));
                    }
                if let Some(expected) = self.current_fn_ret.clone() {
                    match (&val_ty, &expected) {
                        (Some(actual), _) if !actual.is_assignable_to(&expected) => {
                            self.errors.push((
                                format!("Type mismatch: function returns '{:?}' but this expression is '{:?}'", expected, actual),
                                *span,
                            ));
                        }
                        (None, exp) if *exp != Type::Null && *exp != Type::Any => {
                            self.errors.push((
                                format!("Missing return value: function returns '{:?}'", expected),
                                *span,
                            ));
                        }
                        (Some(actual), exp) => {
                            let at = value.as_ref().map_or(*span, Expr::span);
                            self.note_coercion(at, actual, exp);
                        }
                        _ => {}
                    }
                }
            }
            Stmt::Expr { expr, .. } => {
                self.check_expr(expr);
            }
        }
    }

    fn check_spawn_body(&mut self, body: &[Stmt]) -> Type {
        let prev = self.spawn_boundary.replace(self.scopes.len());
        let prev_in_fn = std::mem::replace(&mut self.in_function_body, true);
        let prev_ret = self.current_fn_ret.take();
        let prev_gen = self.gen_elem.take();
        let prev_returns = self.lambda_returns.replace(Vec::new());
        let prev_ends = std::mem::take(&mut self.spawn_ends);
        self.push_scope();
        let (lead, tail) = match body.split_last() {
            Some((Stmt::Expr { expr, .. }, lead)) => (lead, Some(expr)),
            _ => (body, None),
        };
        for s in lead { self.check_stmt(s); }
        let tail_ty = tail.map(|e| self.check_expr(e));
        self.pop_scope();
        let captured = std::mem::replace(&mut self.spawn_ends, prev_ends);
        self.move_captured_ends(captured, prev);
        let returns = std::mem::replace(&mut self.lambda_returns, prev_returns).unwrap_or_default();
        self.current_fn_ret = prev_ret;
        self.gen_elem = prev_gen;
        self.spawn_boundary = prev;
        self.in_function_body = prev_in_fn;
        tail_ty.unwrap_or_else(|| Self::common_return(&returns))
    }

    fn move_captured_ends(&mut self, captured: Vec<(String, usize, Span)>, outer: Option<usize>) {
        let mut done: Vec<String> = Vec::new();
        for (name, depth, span) in captured {
            if done.contains(&name) {
                continue;
            }
            if self.loop_boundaries.last().is_some_and(|&lb| depth < lb) {
                self.errors.push((format!("`{name}` moves into `spawn` on every pass of the loop; clone it inside the loop"), span));
            }
            self.set_moved(&name, Some("spawn"));
            if outer.is_some_and(|b| depth < b) {
                self.spawn_ends.push((name.clone(), depth, span));
            }
            done.push(name);
        }
    }

    fn common_return(returns: &[Type]) -> Type {
        match returns.split_first() {
            None => Type::Null,
            Some((first, rest)) if rest.iter().all(|t| t == first) => first.clone(),
            Some(_) => Type::Any,
        }
    }

    fn check_expr(&mut self, expr: &Expr) -> Type {
        let ty = self.infer_expr(expr);
        let ty = self.full_type(ty);
        self.expr_types.insert(expr.span(), ty.clone());
        if let Expr::Call { callee, args, .. } = expr {
            self.check_call_modes(callee, args);
        }
        ty
    }

    fn check_expr_expecting(&mut self, expr: &Expr, expected: Option<Type>) -> Type {
        if matches!(expr, Expr::Lambda { .. }) {
            self.lambda_expect = expected.clone().filter(|t| matches!(t, Type::Function { .. }));
        }
        self.expected = expected.clone().filter(|t| !t.has_hole() && *t != Type::Any);
        let ty = self.check_expr(expr);
        match &expected {
            Some(exp) => self.settle(expr, exp, ty),
            None => ty,
        }
    }

    fn unsupported_pattern(pat: &Pattern) -> Option<(String, Span)> {
        match pat {
            Pattern::Tuple(subs, _) => subs.iter().find_map(Self::unsupported_pattern),
            Pattern::Struct { span, .. } => {
                Some(("Struct patterns are not supported yet".into(), *span))
            }
            Pattern::Enum { payload: EnumPatternPayload::Struct { fields, .. }, .. } => {
                fields.iter().filter_map(|f| f.pattern.as_ref()).find_map(Self::unsupported_pattern)
            }
            Pattern::Enum { payload: EnumPatternPayload::Tuple(subs), .. } => {
                subs.iter().find_map(Self::unsupported_pattern)
            }
            Pattern::Identifier { subpattern: Some(sub), .. } => Self::unsupported_pattern(sub),
            Pattern::Or(alts, _) => alts.iter().find_map(Self::unsupported_pattern),
            _ => None,
        }
    }

    fn option_of(&self, elem: Type) -> Type {
        Type::Nullable(Box::new(elem))
    }

    fn finish_fallible_natives(&mut self) {
        let error = self.error_type();
        for decl in crate::builtin_natives::decls() {
            let Some(node) = crate::builtin_natives::fallible_payload(decl) else { continue };
            let payload = self.resolve_type_node(node);
            let result = self.result_of(payload, error.clone());
            if let Some(sig) = self.fn_sigs.get_mut(&decl.name) {
                sig.ret = result;
            }
        }
    }

    fn error_type(&self) -> Type {
        self.types
            .values()
            .find(|ty| {
                matches!(ty, Type::Class { name, fields, .. }
                    if isa::type_term::display_type_name(name) == "Error"
                        && fields.iter().any(|(f, _)| f == "kind")
                        && fields.iter().any(|(f, _)| f == "message"))
            })
            .cloned()
            .unwrap_or(Type::Any)
    }

    fn result_of(&self, ok: Type, err: Type) -> Type {
        self.instantiate_by_shape(&[("Ok", 1), ("Err", 1)], &[ok, err])
    }

    fn failure_variant(ty: &Type) -> Option<&'static str> {
        match Self::variants_of(ty)? {
            vs if vs.iter().any(|(v, _)| v == "None") => Some("None"),
            vs if vs.iter().any(|(v, _)| v == "Err") => Some("Err"),
            _ => None,
        }
    }

    fn success_payload(ty: &Type) -> Option<Type> {
        Self::variants_of(ty)?
            .into_iter()
            .find(|(v, _)| v == "Some" || v == "Ok")
            .and_then(|(_, payload)| payload.first().cloned())
    }

    fn variants_of(ty: &Type) -> Option<Vec<(String, Vec<Type>)>> {
        match ty {
            Type::Enum { variants, .. } => Some(variants.clone()),
            Type::Nullable(inner) => Some(vec![("Some".into(), vec![(**inner).clone()]), ("None".into(), Vec::new())]),
            _ => None,
        }
    }

    fn pattern_variant_name(pat: &Pattern) -> Option<&str> {
        match pat {
            Pattern::Enum { variant: Some(v), .. } => Some(v),
            Pattern::Enum { variant: None, target: TypeNode::Named(head, _), .. } => Some(head),
            _ => None,
        }
    }

    fn classify_pattern(
        &mut self,
        pat: &Pattern,
        subj_ty: &Type,
        has_catchall: &mut bool,
        bools_seen: &mut u8,
        covered: &mut HashSet<String>,
    ) {
        match pat {
            Pattern::Wildcard(_) => *has_catchall = true,
            Pattern::Identifier { subpattern: None, name, .. } => {
                if let Some(variants) = Self::variants_of(subj_ty)
                    && variants.iter().any(|(v, _)| v == name) {
                        covered.insert(name.clone());
                        return;
                    }
                *has_catchall = true;
            }
            Pattern::Enum { .. } => {
                if let Some(v) = Self::pattern_variant_name(pat) {
                    covered.insert(v.to_string());
                }
            }
            Pattern::Literal(e, _) => match e.as_ref() {
                Expr::Bool(b, _) => *bools_seen |= if *b { 0b10 } else { 0b01 },
                Expr::Null(_) => {
                    covered.insert("None".to_string());
                }
                _ => {}
            },
            Pattern::Or(alts, _) => {
                for a in alts {
                    self.classify_pattern(a, subj_ty, has_catchall, bools_seen, covered);
                }
            }
            Pattern::Tuple(subs, _)
                if subs.iter().all(Self::pattern_is_irrefutable) => {
                    *has_catchall = true;
                }
            _ => {}
        }
    }

    fn pattern_is_irrefutable(pat: &Pattern) -> bool {
        match pat {
            Pattern::Wildcard(_) => true,
            Pattern::Identifier { subpattern: None, .. } => true,
            Pattern::Identifier { subpattern: Some(sub), .. } => Self::pattern_is_irrefutable(sub),
            Pattern::Tuple(subs, _) => subs.iter().all(Self::pattern_is_irrefutable),
            _ => false,
        }
    }

    fn bind_field_pattern(&mut self, f: &FieldPattern, ty: &Type) {
        match &f.pattern {
            None => self.bind_read_only(&f.name, ty.clone()),
            Some(sub) => self.bind_pattern_names(sub, ty),
        }
    }

    fn bind_pattern_names(&mut self, pat: &Pattern, ty: &Type) {
        match pat {
            Pattern::Identifier { name, subpattern, .. } => {
                let is_unit_variant = Self::variants_of(ty).is_some_and(|vs| vs.iter().any(|(v, p)| v == name && p.is_empty()));
                if !is_unit_variant {
                    self.bind_read_only(name, ty.clone());
                }
                if let Some(sub) = subpattern {
                    self.bind_pattern_names(sub, ty);
                }
            }
            Pattern::Enum { payload: EnumPatternPayload::Tuple(subs), .. } => {
                let variant = Self::pattern_variant_name(pat).map(str::to_string);
                let elem_tys: Vec<Type> = match (Self::variants_of(ty), &variant) {
                    (Some(variants), Some(v)) => variants.into_iter().find(|(vn, _)| vn == v).map(|(_, p)| p).unwrap_or_default(),
                    _ => Vec::new(),
                };
                for (i, sub) in subs.iter().enumerate() {
                    let et = elem_tys.get(i).cloned().unwrap_or(Type::Any);
                    self.bind_pattern_names(sub, &et);
                }
            }
            Pattern::Enum { payload: EnumPatternPayload::Struct { fields, has_rest }, span, .. } => {
                let Some(v) = Self::pattern_variant_name(pat).map(str::to_string) else { return };
                let Type::Enum { name: en, variants, .. } = ty else {
                    for f in fields {
                        self.bind_field_pattern(f, &Type::Any);
                    }
                    return;
                };
                let key = (en.clone(), v.clone());
                let Some(names) = self.variant_fields.get(&key).cloned() else {
                    self.errors.push((format!("`{en}.{v}` has no named fields"), *span));
                    return;
                };
                let payload = variants.iter().find(|(n, _)| *n == v).map(|(_, p)| p.clone()).unwrap_or_default();
                let given: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
                let declared: Vec<String> = if *has_rest {
                    names.iter().filter(|n| given.contains(&n.as_str())).cloned().collect()
                } else {
                    names.clone()
                };
                self.check_literal_field_names(&format!("{en}.{v}"), "pattern", given.iter().copied(), &declared, *span);
                for f in fields {
                    let t = names.iter().position(|n| *n == f.name).and_then(|i| payload.get(i).cloned()).unwrap_or(Type::Any);
                    self.bind_field_pattern(f, &t);
                }
            }
            Pattern::Or(alts, _) => {
                for a in alts {
                    self.bind_pattern_names(a, ty);
                }
            }
            Pattern::Type { name, target, span } => {
                let bound = self.check_type_pattern(target, ty, *span);
                if let Some(name) = name {
                    self.bind_read_only(name, bound);
                }
            }
            Pattern::Tuple(subs, span) => match ty {
                Type::Tuple(tys) if tys.len() == subs.len() => {
                    for (sub, t) in subs.iter().zip(tys) {
                        self.bind_pattern_names(sub, t);
                    }
                }
                Type::Tuple(tys) => {
                    self.errors.push((
                        format!("tuple pattern has {} element(s), expected {}", subs.len(), tys.len()),
                        *span,
                    ));
                    for sub in subs {
                        self.bind_pattern_names(sub, &Type::Any);
                    }
                }
                Type::Any => {
                    for sub in subs {
                        self.bind_pattern_names(sub, &Type::Any);
                    }
                }
                _ => {
                    self.errors.push((
                        format!("cannot match a tuple pattern against '{:?}'", ty),
                        *span,
                    ));
                    for sub in subs {
                        self.bind_pattern_names(sub, &Type::Any);
                    }
                }
            },
            _ => {}
        }
    }

    fn resolve_type_node(&self, node: &TypeNode) -> Type {
        match node {
            TypeNode::Int(_) => Type::Int,
            TypeNode::Float(_) => Type::Float,
            TypeNode::Bool(_) => Type::Bool,
            TypeNode::Char(_) => Type::Char,
            TypeNode::String(_) => Type::String,
            TypeNode::Null(_) => Type::Null,
            TypeNode::Tuple(elems, _) => Type::Tuple(elems.iter().map(|t| self.resolve_type_node(t)).collect()),
            TypeNode::Function(params, ret, send, _) => {
                fn bare(p: &TypeNode) -> &TypeNode {
                    match p {
                        TypeNode::VarParam(inner, _) => inner.as_ref(),
                        other => other,
                    }
                }
                let modes: Vec<bool> = params.iter().map(|p| matches!(p, TypeNode::VarParam(..))).collect();
                Type::Function {
                    params: params.iter().map(|p| self.resolve_type_node(bare(p))).collect(),
                    ret: Box::new(self.resolve_type_node(ret)),
                    sendable: *send,
                    modes: if modes.contains(&true) { modes } else { Vec::new() },
                }
            }
            TypeNode::VarParam(inner, _) => self.resolve_type_node(inner),
            TypeNode::Array(_, _, _) => Type::Any,
            TypeNode::Union(members, span) => self.resolve_union(members, *span),
            TypeNode::SelfType(_) => self.current_self_type.clone().unwrap_or(Type::Any),
            TypeNode::Named(name, _) if name == "Bytes" => Type::Bytes,
            TypeNode::Named(name, _) if self.param_subst.contains_key(name) => self.param_subst[name].clone(),
            TypeNode::Named(name, _) if self.type_params.contains(name) => Type::Param(name.clone()),
            TypeNode::Named(name, span) if name == "Option" => {
                self.type_errors.borrow_mut().push(("`Option` needs its type argument, as in `Option<T>` or `T?`".into(), *span));
                Type::Nullable(Box::new(Type::Any))
            }
            TypeNode::Named(name, span) if self.generics.contains_key(name) => {
                let params = self.generics[name].join(", ");
                let msg = format!("`{name}` needs its type arguments, as in `{name}<{params}>`");
                self.type_errors.borrow_mut().push((msg, *span));
                self.instantiate_type(name, &[])
            }
            TypeNode::Named(name, span) if self.type_aliases.contains_key(name) => self.resolve_alias(name, &[], *span),
            TypeNode::Named(name, span) => {
                if let Some(t) = self.types.get(name) {
                    return t.clone();
                }
                if !self.declared_types.contains(name) && !crate::types::is_builtin_type_name(name) {
                    self.type_errors.borrow_mut().push((shared::unknown_type(name), *span));
                    return Type::Any;
                }
                Type::Struct { name: name.clone(), args: Vec::new(), fields: vec![] }
            }
            TypeNode::Nullable(inner, _) => Type::Nullable(Box::new(self.resolve_type_node(inner))),
            TypeNode::Generic(name, args, span) if name == "Option" => {
                if args.len() != 1 {
                    let msg = format!("`Option` takes 1 type argument, but {} were given", args.len());
                    self.type_errors.borrow_mut().push((msg, *span));
                }
                Type::Nullable(Box::new(args.first().map(|a| self.resolve_type_node(a)).unwrap_or(Type::Any)))
            }
            TypeNode::Generic(name, args, span) => {
                if let Some(names) = self.generics.get(name) {
                    if args.len() != names.len() {
                        let msg = format!("`{name}` takes {} type argument(s), but {} were given", names.len(), args.len());
                        self.type_errors.borrow_mut().push((msg, *span));
                    }
                    let args: Vec<Type> = args.iter().map(|a| self.resolve_type_node(a)).collect();
                    return self.instantiate_type(name, &args);
                }
                if self.type_aliases.contains_key(name) {
                    return self.resolve_alias(name, args, *span);
                }
                match name.as_str() {
                    "List" => Type::List(Box::new(args.first().map(|a| self.resolve_type_node(a)).unwrap_or(Type::Any))),
                    "Map" => {
                        let key = args.first().map(|a| self.resolve_type_node(a)).unwrap_or(Type::Any);
                        self.check_key_type(&key, *span);
                        Type::Map(Box::new(key), Box::new(args.get(1).map(|a| self.resolve_type_node(a)).unwrap_or(Type::Any)))
                    }
                    "Set" => {
                        let key = args.first().map(|a| self.resolve_type_node(a)).unwrap_or(Type::Any);
                        self.check_key_type(&key, *span);
                        Type::Set(Box::new(key))
                    }
                    "Bytes" => Type::Bytes,
                    "Receiver" => Type::Receiver(Box::new(args.first().map(|a| self.resolve_type_node(a)).unwrap_or(Type::Any))),
                    "Sender" => Type::Sender(Box::new(args.first().map(|a| self.resolve_type_node(a)).unwrap_or(Type::Any))),
                    "Stream" => Type::Stream(Box::new(args.first().map(|a| self.resolve_type_node(a)).unwrap_or(Type::Any))),
                    "Task" => Type::Task(Box::new(args.first().map(|a| self.resolve_type_node(a)).unwrap_or(Type::Any))),
                    "Shared" => Type::Shared(Box::new(args.first().map(|a| self.resolve_type_node(a)).unwrap_or(Type::Any))),
                    "Result" => Type::Result(
                        Box::new(args.first().map(|a| self.resolve_type_node(a)).unwrap_or(Type::Any)),
                        Box::new(args.get(1).map(|a| self.resolve_type_node(a)).unwrap_or(Type::String)),
                    ),
                    other => {
                        if !self.declared_types.contains(other) && !crate::types::is_builtin_type_name(other) {
                            self.type_errors.borrow_mut().push((shared::unknown_type(other), *span));
                        }
                        Type::Any
                    }
                }
            }
        }
    }

    pub(super) fn named_field_variant(&self, name: &str) -> Option<(String, String)> {
        if let Some((en, v)) = name.rsplit_once('.') {
            let key = (en.to_string(), v.to_string());
            return self.variant_fields.contains_key(&key).then_some(key);
        }
        if self.types.contains_key(name) {
            return None;
        }
        let mut hits = self.variant_fields.keys().filter(|(_, v)| v == name);
        match (hits.next(), hits.next()) {
            (Some(key), None) => Some(key.clone()),
            _ => None,
        }
    }

    fn variant_payload(&self, key: &(String, String)) -> Vec<Type> {
        match self.types.get(&key.0) {
            Some(Type::Enum { variants, .. }) => variants.iter().find(|(v, _)| *v == key.1).map(|(_, p)| p.clone()).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    pub(super) fn check_variant_literal(&mut self, key: (String, String), fields: &[(String, Expr)], span: Span) -> Type {
        let names = self.variant_fields[&key].clone();
        let label = format!("{}.{}", key.0, key.1);
        let vals: Vec<(String, Type)> = fields.iter().map(|(n, v)| (n.clone(), self.check_expr(v))).collect();
        self.check_literal_field_names(&label, "literal", fields.iter().map(|(n, _)| n.as_str()), &names, span);
        let payload = self.variant_payload(&key);
        let generics = self.generics.get(&key.0).cloned().unwrap_or_default();
        let mut bind = HashMap::new();
        for (f, val_ty) in &vals {
            if let Some(i) = names.iter().position(|n| n == f) {
                Self::unify(&payload[i], val_ty, &generics, &mut bind);
            }
        }
        for ((f, val_ty), (_, expr)) in vals.iter().zip(fields) {
            if let Some(i) = names.iter().position(|n| n == f) {
                let want = Self::subst(&payload[i], &generics, &bind);
                if !val_ty.is_assignable_to(&want) {
                    self.errors.push((format!("Type mismatch on '{label}' field '{f}': expected '{want:?}', found '{val_ty:?}'"), span));
                }
                self.note_coercion(expr.span(), val_ty, &want);
            }
        }
        match self.types.get(&key.0) {
            Some(template) => Self::subst(template, &generics, &bind),
            None => Type::Any,
        }
    }

    pub(super) fn check_literal_field_names<'a>(&mut self, label: &str, kind: &str, given: impl Iterator<Item = &'a str>, declared: &[String], span: Span) {
        let given: Vec<&str> = given.collect();
        for (i, f) in given.iter().enumerate() {
            if !declared.iter().any(|n| n == f) {
                self.errors.push((format!("`{label}` has no field `{f}`"), span));
            } else if given[..i].contains(f) {
                self.errors.push((format!("field `{f}` is given twice in a `{label}` {kind}"), span));
            }
        }
        let missing: Vec<String> = declared.iter().filter(|n| !given.contains(&n.as_str())).map(|n| format!("`{n}`")).collect();
        if !missing.is_empty() {
            let hint = if kind == "pattern" { "; add `..` to skip them" } else { "" };
            self.errors.push((format!("`{label}` {kind} is missing field(s) {}{hint}", missing.join(", ")), span));
        }
    }

    pub(super) fn full_type(&self, ty: Type) -> Type {
        let name = match &ty {
            Type::Struct { name, fields, .. } | Type::Class { name, fields, .. } if fields.is_empty() => name,
            Type::Enum { name, variants, .. } if variants.is_empty() => name,
            Type::Nullable(inner) => return Type::Nullable(Box::new(self.full_type((**inner).clone()))),
            _ => return ty,
        };
        let args = match &ty {
            Type::Struct { args, .. } | Type::Class { args, .. } | Type::Enum { args, .. } => args.clone(),
            _ => Vec::new(),
        };
        match self.types.get(name) {
            Some(full) if std::mem::discriminant(full) == std::mem::discriminant(&ty) => {
                if self.generics.contains_key(name) { self.instantiate_type(name, &args) } else { full.clone() }
            }
            _ => ty,
        }
    }

    fn check_key_type(&self, key: &Type, span: Span) {
        if let Some(part) = key.mutable_key_part() {
            let msg = format!("a `{part:?}` can't be part of a Map or Set key, because it can change after insert; use a tuple");
            self.type_errors.borrow_mut().push((msg, span));
        }
    }

    fn resolve_alias(&self, name: &str, args: &[TypeNode], span: Span) -> Type {
        let alias = &self.type_aliases[name];
        if self.alias_stack.borrow().iter().any(|n| n == name) {
            self.type_errors.borrow_mut().push((format!("type alias `{name}` refers to itself"), span));
            return Type::Any;
        }
        if args.len() != alias.generic_params.len() {
            let msg = format!("`{name}` takes {} type argument(s), but {} were given", alias.generic_params.len(), args.len());
            self.type_errors.borrow_mut().push((msg, span));
            return Type::Any;
        }
        let map: HashMap<&str, &TypeNode> =
            alias.generic_params.iter().map(|p| p.name.as_str()).zip(args.iter()).collect();
        let target = subst_type_node(&alias.target, &map);
        self.alias_stack.borrow_mut().push(name.to_string());
        let ty = self.resolve_type_node(&target);
        self.alias_stack.borrow_mut().pop();
        ty
    }

    fn closeable_mismatch(&mut self, ty: &Type) -> Option<String> {
        match ty {
            Type::Any => Some(
                "the type of a `with` resource must be known, not `Any`".into(),
            ),
            Type::Class { name, .. } | Type::Struct { name, .. } => {
                match self.class_method_sigs.get(&(name.clone(), "close".to_string())) {
                    None => Some("missing method `close`".into()),
                    Some(sig) if sig.is_static => Some("`close` must take a `self` receiver".into()),
                    Some(sig) if !sig.params.is_empty() => Some("`close` must take no arguments besides `self`".into()),
                    Some(_) => None,
                }
            }
            _ => Some("it has no `close()` method".into()),
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
        self.drop_narrowings();
    }

    fn insert_symbol(&mut self, name: &str, ty: Type, is_mutable: bool) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(
                name.to_string(),
                SymbolInfo {
                    ty,
                    is_mutable,
                    is_moved: None,
                    read_only: modes::ReadOnly::No,
                },
            );
        }
    }

    fn set_moved(&mut self, name: &str, moved: Option<&'static str>) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(info) = scope.get_mut(name) {
                info.is_moved = moved;
                return;
            }
        }
    }

    fn lookup_symbol(&self, name: &str) -> Option<SymbolInfo> {
        for scope in self.scopes.iter().rev() {
            if let Some(info) = scope.get(name) {
                return Some(info.clone());
            }
        }
        None
    }

    fn lookup_symbol_depth(&self, name: &str) -> Option<(SymbolInfo, usize)> {
        for (i, scope) in self.scopes.iter().enumerate().rev() {
            if let Some(info) = scope.get(name) {
                return Some((info.clone(), i));
            }
        }
        None
    }
    fn is_sendable(&self, ty: &Type) -> bool {
        self.is_sendable_in(ty, &mut HashSet::new())
    }

    fn is_sendable_in(&self, ty: &Type, seen: &mut HashSet<String>) -> bool {
        match ty {
            Type::Nullable(inner) => self.is_sendable_in(inner, seen),
            Type::Union(crate::types::Members(ms)) => ms.iter().all(|m| self.is_sendable_in(m, seen)),
            Type::Class { name, fields, .. } => {
                if self.mutable_classes.contains(name) {
                    return false;
                }
                if !seen.insert(name.clone()) {
                    return true;
                }
                fields.iter().all(|(_, t)| self.is_sendable_in(t, seen))
            }
            Type::Int | Type::Float | Type::Bool | Type::Char | Type::Null => true,
            Type::String | Type::Any | Type::Receiver(_) | Type::Sender(_) | Type::Task(_) => true,
            Type::Shared(_) => true,
            Type::Function { sendable, .. } => *sendable,
            Type::Struct { fields, .. } => fields.iter().all(|(_, t)| self.is_sendable_in(t, seen)),
            _ => false,
        }
    }

    fn is_mutating_method(method: &str) -> bool {
        matches!(
            method,
            "push" | "set" | "clear" | "pop" | "remove" | "add" | "extend"
        )
    }
}

impl TypeChecker {
    /// Ends checking: packages `modules` with every expression's type and the run-time checks.
    pub fn finish(mut self, mut modules: Vec<Program>) -> crate::stage::Checked {
        let (type_args, lists) = self.resolve_calls();
        let mut types = crate::stage::TypeTable::with_instance_calls(std::mem::take(&mut self.expr_types), std::mem::take(&mut self.instance_calls))
            .with_calls(type_args, lists)
            .with_any_checks(std::mem::take(&mut self.any_checks))
            .with_type_tests(std::mem::take(&mut self.type_tests))
            .with_rewraps(self.take_rewraps());
        let mut instances = crate::ast::Program { items: Vec::new(), span: Span::default(), stable_marks: Vec::new(), test_marks: Vec::new() };
        for (decl, table) in std::mem::take(&mut self.instance_done) {
            types.add_instance(decl.name.clone(), table);
            instances.items.push(Item::Function(decl));
        }
        for program in &mut modules {
            program.items.retain(|it| !matches!(it, Item::Function(f) if self.bounded_fns.contains_key(&f.name)));
        }
        if let Some(last) = modules.last_mut() {
            last.items.append(&mut instances.items);
        }
        crate::stage::Checked { modules, types }
    }
}

fn subst_type_node(node: &TypeNode, map: &HashMap<&str, &TypeNode>) -> TypeNode {
    let sub = |n: &TypeNode| subst_type_node(n, map);
    match node {
        TypeNode::Named(n, _) if map.contains_key(n.as_str()) => map[n.as_str()].clone(),
        TypeNode::Nullable(inner, s) => TypeNode::Nullable(Box::new(sub(inner)), *s),
        TypeNode::VarParam(inner, s) => TypeNode::VarParam(Box::new(sub(inner)), *s),
        TypeNode::Generic(n, args, s) => TypeNode::Generic(n.clone(), args.iter().map(sub).collect(), *s),
        TypeNode::Tuple(elems, s) => TypeNode::Tuple(elems.iter().map(sub).collect(), *s),
        TypeNode::Union(members, s) => TypeNode::Union(members.iter().map(sub).collect(), *s),
        TypeNode::Function(params, ret, send, s) => TypeNode::Function(params.iter().map(sub).collect(), Box::new(sub(ret)), *send, *s),
        TypeNode::Array(elem, len, s) => TypeNode::Array(Box::new(sub(elem)), len.clone(), *s),
        other => other.clone(),
    }
}
