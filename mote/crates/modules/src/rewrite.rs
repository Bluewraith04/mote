//! Rewrites one module's AST against its [`ModuleScope`]: mangles top-level definitions and resolves every cross-module reference to a mangled name.
//! Locals shadow global symbols of the same name and are never rewritten.

use std::collections::HashSet;

use compiler::ast::*;

use crate::symbols::ModuleScope;

/// Rewrite every item in `program` in place.
pub(crate) fn rewrite_program(program: &mut Program, scope: &ModuleScope) -> Result<(), String> {
    let mut rw = Rewriter {
        scope,
        locals: vec![HashSet::new()],
    };
    for item in &mut program.items {
        rw.item(item)?;
    }
    Ok(())
}

struct Rewriter<'a> {
    scope: &'a ModuleScope,
    locals: Vec<HashSet<String>>,
}

impl Rewriter<'_> {
    fn push_scope(&mut self) {
        self.locals.push(HashSet::new());
    }

    fn pop_scope(&mut self) {
        self.locals.pop();
    }

    fn bind_local(&mut self, name: &str) {
        if let Some(top) = self.locals.last_mut() {
            top.insert(name.to_string());
        }
    }

    fn is_local(&self, name: &str) -> bool {
        self.locals.iter().any(|s| s.contains(name))
    }

    fn alias_of(&self, object: &Expr) -> Option<&crate::symbols::AliasTarget> {
        fn dotted(e: &Expr) -> Option<String> {
            match e {
                Expr::Ident(name, _) => Some(name.clone()),
                Expr::MemberAccess { object, member, .. } => Some(format!("{}.{member}", dotted(object)?)),
                _ => None,
            }
        }
        let path = dotted(object)?;
        let head = path.split('.').next()?;
        if self.is_local(head) {
            return None;
        }
        self.scope.alias(&path)
    }

    fn item(&mut self, item: &mut Item) -> Result<(), String> {
        match item {
            Item::Function(f) => self.function(f, true),
            Item::NativeFunction(f) => {
                f.name = self.scope.mangled(&f.name);
                for p in &mut f.params {
                    if let Some(ty) = &mut p.ty {
                        self.ty(ty)?;
                    }
                }
                if let Some(rt) = &mut f.return_type {
                    self.ty(rt)?;
                }
                Ok(())
            }
            Item::Struct(s) => {
                s.name = self.scope.mangled(&s.name);
                for t in &mut s.traits {
                    self.ty(t)?;
                }
                for field in &mut s.fields {
                    self.ty(&mut field.ty)?;
                }
                for m in &mut s.methods {
                    self.function(m, false)?;
                }
                Ok(())
            }
            Item::Class(c) => {
                c.name = self.scope.mangled(&c.name);
                for t in &mut c.traits {
                    self.ty(t)?;
                }
                for field in &mut c.fields {
                    self.ty(&mut field.ty)?;
                }
                for m in &mut c.methods {
                    self.function(m, false)?;
                }
                Ok(())
            }
            Item::Enum(e) => {
                e.name = self.scope.mangled(&e.name);
                for t in &mut e.traits {
                    self.ty(t)?;
                }
                for v in &mut e.variants {
                    match &mut v.kind {
                        EnumVariantKind::Unit { discriminant } => {
                            if let Some(d) = discriminant {
                                self.expr(d)?;
                            }
                        }
                        EnumVariantKind::Tuple(tys) => {
                            for t in tys {
                                self.ty(t)?;
                            }
                        }
                        EnumVariantKind::Struct(fields) => {
                            for fd in fields {
                                self.ty(&mut fd.ty)?;
                            }
                        }
                    }
                }
                for m in &mut e.methods {
                    self.function(m, false)?;
                }
                Ok(())
            }
            Item::Trait(t) => {
                t.name = self.scope.mangled(&t.name);
                for member in &mut t.members {
                    for p in &mut member.params {
                        if let Some(ty) = &mut p.ty {
                            self.ty(ty)?;
                        }
                    }
                    if let Some(rt) = &mut member.return_type {
                        self.ty(rt)?;
                    }
                    if let Some(body) = &mut member.default_body {
                        self.block(body)?;
                    }
                }
                Ok(())
            }
            Item::TypeAlias(a) => {
                a.name = self.scope.mangled(&a.name);
                self.ty(&mut a.target)
            }
            Item::Import(_) => Ok(()),
            Item::TopLevelStmt(s) => self.toplevel_stmt(s),
        }
    }

    fn toplevel_stmt(&mut self, stmt: &mut Stmt) -> Result<(), String> {
        match stmt {
            Stmt::Let { name, ty, init, .. } | Stmt::Var { name, ty, init, .. } => {
                if let Some(t) = ty {
                    self.ty(t)?;
                }
                self.expr(init)?;
                if let Some(mangled) = self.scope.resolve_value(name) {
                    *name = mangled.to_string();
                }
                Ok(())
            }
            other => self.stmt(other),
        }
    }

    fn function(&mut self, f: &mut FunctionDecl, mangle_name: bool) -> Result<(), String> {
        if mangle_name {
            f.name = self.scope.mangled(&f.name);
        }
        for gp in &mut f.generic_params {
            for b in &mut gp.bounds {
                self.ty(b)?;
            }
        }
        for p in &mut f.params {
            if let Some(ty) = &mut p.ty {
                self.ty(ty)?;
            }
        }
        for p in &mut f.params {
            if let Some(ParameterKind::Regular { default_value: Some(default), .. }) = &mut p.kind {
                self.expr(default)?;
            }
        }
        if let Some(rt) = &mut f.return_type {
            self.ty(rt)?;
        }
        self.push_scope();
        for p in &f.params {
            self.bind_local(&p.name);
        }
        let body_res = self.block_no_scope(&mut f.body);
        self.pop_scope();
        body_res
    }

    fn block(&mut self, body: &mut [Stmt]) -> Result<(), String> {
        self.push_scope();
        let res = self.block_no_scope(body);
        self.pop_scope();
        res
    }

    fn block_no_scope(&mut self, body: &mut [Stmt]) -> Result<(), String> {
        for s in body {
            self.stmt(s)?;
        }
        Ok(())
    }

    fn stmt(&mut self, stmt: &mut Stmt) -> Result<(), String> {
        match stmt {
            Stmt::Let { name, ty, init, .. } | Stmt::Var { name, ty, init, .. } => {
                if let Some(t) = ty {
                    self.ty(t)?;
                }
                self.expr(init)?;
                self.bind_local(name);
                Ok(())
            }
            Stmt::TupleLet { names, init, .. } => {
                self.expr(init)?;
                for n in names {
                    self.bind_local(n);
                }
                Ok(())
            }
            Stmt::Assign { target, value, .. } => {
                self.expr(target)?;
                self.expr(value)
            }
            Stmt::CompoundAssign { target, value, .. } => {
                self.expr(target)?;
                self.expr(value)
            }
            Stmt::Expr { expr, .. } => self.expr(expr),
            Stmt::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                self.expr(cond)?;
                self.block(then_branch)?;
                if let Some(eb) = else_branch {
                    self.block(eb)?;
                }
                Ok(())
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond)?;
                self.block(body)
            }
            Stmt::ForIn {
                var_name,
                iter,
                body,
                ..
            } => {
                self.expr(iter)?;
                self.push_scope();
                self.bind_local(var_name);
                let res = self.block_no_scope(body);
                self.pop_scope();
                res
            }
            Stmt::Match { expr, arms, .. } => {
                self.expr(expr)?;
                for arm in arms {
                    self.match_arm(arm)?;
                }
                Ok(())
            }
            | Stmt::ScopeBlock { body, .. }
            | Stmt::SpawnBlock { body, .. }
            | Stmt::Block { body, .. } => self.block(body),
            Stmt::WithBlock { name, init, body, .. } => {
                self.expr(init)?;
                self.push_scope();
                self.bind_local(name);
                let res = self.block_no_scope(body);
                self.pop_scope();
                res
            }
            Stmt::Return { value, .. } | Stmt::Break { value, .. } => {
                if let Some(v) = value {
                    self.expr(v)?;
                }
                Ok(())
            }
            Stmt::Yield { value, .. } => self.expr(value),
            Stmt::Continue { .. } => Ok(()),
        }
    }

    fn match_arm(&mut self, arm: &mut MatchArm) -> Result<(), String> {
        self.push_scope();
        let mut bindings = HashSet::new();
        collect_pattern_bindings(&arm.pattern, &mut bindings);
        for b in &bindings {
            self.bind_local(b);
        }
        let res = (|| {
            self.pattern(&mut arm.pattern)?;
            if let Some(g) = &mut arm.guard {
                self.expr(g)?;
            }
            self.block_no_scope(&mut arm.body)
        })();
        self.pop_scope();
        res
    }

    fn expr(&mut self, expr: &mut Expr) -> Result<(), String> {
        match expr {
            Expr::Ident(name, span) => {
                if !self.is_local(name) {
                    if let Some(m) = self
                        .scope
                        .resolve_value(name)
                        .or_else(|| self.scope.resolve_type(name))
                    {
                        *expr = Expr::Ident(m.to_string(), *span);
                    }
                }
                Ok(())
            }
            Expr::Call { callee, args, span, .. } => {
                if let Expr::MemberAccess { object, member, .. } = callee.as_mut() {
                    if let Some(target) = self.alias_of(object) {
                        match target.resolve(member) {
                            Some(m) => {
                                **callee = Expr::Ident(m, *span);
                            }
                            None => {
                                return Err(format!(
                                    "module '{}' has no exported symbol '{}'",
                                    target.module.display_path(), member
                                ));
                            }
                        }
                        for arg in args.iter_mut() {
                            self.expr(arg)?;
                        }
                        return Ok(());
                    }
                }
                self.expr(callee)?;
                for arg in args.iter_mut() {
                    self.expr(arg)?;
                }
                Ok(())
            }
            Expr::MemberAccess { object, member, span } => {
                if let Some(target) = self.alias_of(object) {
                    return match target.resolve(member) {
                        Some(m) => {
                            *expr = Expr::Ident(m, *span);
                            Ok(())
                        }
                        None => Err(format!(
                            "module '{}' has no exported symbol '{}' (line {})",
                            target.module.display_path(), member, span.line
                        )),
                    };
                }
                self.expr(object)
            }
            Expr::StructInit {
                name,
                target_type,
                fields,
                span,
            } => {
                let enum_variant = name.split_once('.').filter(|(head, _)| self.scope.alias(head).is_none());
                if let Some((head, variant)) = enum_variant {
                    let en = self.scope.resolve_type(head).map(str::to_string).unwrap_or_else(|| head.to_string());
                    *name = format!("{en}.{variant}");
                } else if let Some((ty, variant)) = name.rsplit_once('.').filter(|(ty, _)| ty.contains('.')) {
                    if let Some(m) = self.type_name(ty, span.line)? {
                        *name = format!("{m}.{variant}");
                    }
                } else if name.contains('.') {
                    if let Some(m) = self.type_name(name, span.line)? {
                        *name = m;
                    }
                } else if let Some(m) = self
                    .scope
                    .resolve_type(name)
                    .or_else(|| self.scope.resolve_value(name))
                {
                    *name = m.to_string();
                }
                if let Some(t) = target_type {
                    self.ty(t)?;
                }
                for (_, v) in fields.iter_mut() {
                    self.expr(v)?;
                }
                Ok(())
            }
            Expr::Binary { left, right, .. } => {
                self.expr(left)?;
                self.expr(right)
            }
            Expr::Unary { expr, .. }
            | Expr::Try { expr, .. }
            | Expr::Unwrap { expr, .. } => self.expr(expr),
            Expr::OptionalChain { object, body, .. } => {
                self.expr(object)?;
                self.expr(body)
            }
            Expr::Spawn { body, .. } => self.block(body),
            Expr::TypeTest { expr, target_type, .. } => {
                self.expr(expr)?;
                self.ty(target_type)
            }
            Expr::StaticAccess { target, member, .. } => {
                if let (TypeNode::Generic(name, _, _), "new") = (&mut *target, member.as_str()) {
                    if !self.is_local(name) && self.scope.resolve_type(name).is_none() {
                        if let Some(m) = self.scope.resolve_value(name) {
                            *name = m.to_string();
                        }
                    }
                }
                self.ty(target)
            }
            Expr::Index { object, index, .. } => {
                self.expr(object)?;
                self.expr(index)
            }
            Expr::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.expr(s)?;
                }
                if let Some(e) = end {
                    self.expr(e)?;
                }
                Ok(())
            }
            Expr::Ternary {
                cond,
                then_expr,
                else_expr,
                ..
            } => {
                self.expr(cond)?;
                self.expr(then_expr)?;
                self.expr(else_expr)
            }
            Expr::NullCoalesce { left, right, .. } => {
                self.expr(left)?;
                self.expr(right)
            }
            Expr::ListLiteral { elements, .. } | Expr::TupleLiteral { elements, .. } => {
                for e in elements {
                    self.expr(e)?;
                }
                Ok(())
            }
            Expr::MapLiteral { entries, .. } => {
                for (k, v) in entries {
                    self.expr(k)?;
                    self.expr(v)?;
                }
                Ok(())
            }
            Expr::Lambda {
                params,
                return_type,
                body,
                ..
            } => {
                for p in params.iter_mut() {
                    if let Some(ty) = &mut p.ty {
                        self.ty(ty)?;
                    }
                }
                if let Some(rt) = return_type {
                    self.ty(rt)?;
                }
                self.push_scope();
                for p in params.iter() {
                    self.bind_local(&p.name);
                }
                let res = self.block_no_scope(body);
                self.pop_scope();
                res
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                self.expr(cond)?;
                self.block(then_branch)?;
                self.block(else_branch)
            }
            Expr::Match { expr, arms, .. } => {
                self.expr(expr)?;
                for arm in arms {
                    self.match_arm(arm)?;
                }
                Ok(())
            }
            Expr::Block { body, .. } => self.block(body),
            Expr::Int(..)
            | Expr::Float(..)
            | Expr::Bool(..)
            | Expr::String(..)
            | Expr::Char(..)
            | Expr::Null(_)
            | Expr::SelfValue(_) => Ok(()),
        }
    }

    fn type_name(&self, name: &str, line: usize) -> Result<Option<String>, String> {
        let Some((first_alias, first_member)) = name.split_once('.') else {
            return Ok(self.scope.resolve_type(name).map(str::to_string));
        };
        let split = name.rmatch_indices('.').map(|(i, _)| (&name[..i], &name[i + 1..])).find(|(a, _)| self.scope.alias(a).is_some());
        let (alias, member) = split.unwrap_or((first_alias, first_member));
        let Some(target) = self.scope.alias(alias) else {
            if self.scope.resolve_type(alias).is_some() || matches!(alias, "Option" | "Result") {
                return Err(format!("`{name}` is an enum variant, not a type; test for it with `match` (line {line})"));
            }
            return Err(format!("`{alias}` in type `{name}` is not a module alias (line {line})"));
        };
        target.resolve(member).map(Some).ok_or_else(|| {
            format!("module '{}' has no exported type '{member}' (line {line})", target.module.display_path())
        })
    }

    fn ty(&mut self, node: &mut TypeNode) -> Result<(), String> {
        match node {
            TypeNode::Named(name, span) => {
                if let Some(m) = self.type_name(name, span.line)? {
                    *node = TypeNode::Named(m, *span);
                }
                Ok(())
            }
            TypeNode::Generic(name, args, span) => {
                if let Some(m) = self.type_name(name, span.line)? {
                    *name = m;
                }
                for a in args {
                    self.ty(a)?;
                }
                Ok(())
            }
            TypeNode::Nullable(inner, _) | TypeNode::VarParam(inner, _) => self.ty(inner),
            TypeNode::Tuple(items, _) | TypeNode::Union(items, _) => {
                for i in items {
                    self.ty(i)?;
                }
                Ok(())
            }
            TypeNode::Function(params, ret, _, _) => {
                for p in params {
                    self.ty(p)?;
                }
                self.ty(ret)
            }
            TypeNode::Array(inner, size, _) => {
                self.ty(inner)?;
                if let Some(s) = size {
                    self.expr(s)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn pattern(&mut self, pat: &mut Pattern) -> Result<(), String> {
        match pat {
            Pattern::Literal(expr, _) => self.expr(expr),
            Pattern::Identifier {
                subpattern: Some(sub),
                ..
            } => self.pattern(sub),
            Pattern::Type { target, .. } => self.ty(target),
            Pattern::Tuple(pats, _) | Pattern::Or(pats, _) => {
                for p in pats {
                    self.pattern(p)?;
                }
                Ok(())
            }
            Pattern::Struct { target, fields, .. } => {
                self.ty(target)?;
                for f in fields {
                    if let Some(p) = &mut f.pattern {
                        self.pattern(p)?;
                    }
                }
                Ok(())
            }
            Pattern::Enum { target, payload, .. } => {
                self.ty(target)?;
                match payload {
                    EnumPatternPayload::Tuple(pats) => {
                        for p in pats {
                            self.pattern(p)?;
                        }
                    }
                    EnumPatternPayload::Struct { fields, .. } => {
                        for f in fields {
                            if let Some(p) = &mut f.pattern {
                                self.pattern(p)?;
                            }
                        }
                    }
                    EnumPatternPayload::None => {}
                }
                Ok(())
            }
            Pattern::Range { start, end, .. } => {
                self.pattern(start)?;
                self.pattern(end)
            }
            Pattern::Wildcard(_) | Pattern::Identifier { subpattern: None, .. } => Ok(()),
        }
    }
}

fn collect_pattern_bindings(pat: &Pattern, out: &mut HashSet<String>) {
    match pat {
        Pattern::Identifier { name, subpattern, .. } => {
            out.insert(name.clone());
            if let Some(sub) = subpattern {
                collect_pattern_bindings(sub, out);
            }
        }
        Pattern::Tuple(pats, _) | Pattern::Or(pats, _) => {
            for p in pats {
                collect_pattern_bindings(p, out);
            }
        }
        Pattern::Struct { fields, .. } => {
            for f in fields {
                match &f.pattern {
                    Some(p) => collect_pattern_bindings(p, out),
                    None => {
                        out.insert(f.name.clone());
                    }
                }
            }
        }
        Pattern::Enum { payload, .. } => match payload {
            EnumPatternPayload::Tuple(pats) => {
                for p in pats {
                    collect_pattern_bindings(p, out);
                }
            }
            EnumPatternPayload::Struct { fields, .. } => {
                for f in fields {
                    match &f.pattern {
                        Some(p) => collect_pattern_bindings(p, out),
                        None => {
                            out.insert(f.name.clone());
                        }
                    }
                }
            }
            EnumPatternPayload::None => {}
        },
        Pattern::Type { name, .. } => {
            out.extend(name.clone());
        }
        Pattern::Range { .. } | Pattern::Literal(..) | Pattern::Wildcard(_) => {}
    }
}
