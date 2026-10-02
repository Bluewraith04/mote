//! Escape analysis for regions: which `let p = T {.. }` sites can live in a region.

use std::collections::{HashMap, HashSet};

use crate::ast::{stmts_yield, BinaryOp, Expr, FunctionDecl, MatchArm, ParameterKind, Stmt, TypeNode};
use crate::span::Span;

/// Why a `let`/`var` struct literal is on the heap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeapReason {
    /// The name is declared more than once in the function.
    Redeclared(String),
    /// The name is used as a whole value (stored, aliased, returned as is) at the span.
    WholeValue(String, Span),
    /// The name is passed at the span to a call that may keep it, or to one that is not analysed.
    Retained(String, Span),
    /// The name is used inside a `spawn` or lambda at the span.
    Captured(String, Span),
    /// The `var` is shared with a lambda that assigns it.
    SharedVar(String),
    /// The struct has more fields than a region object holds.
    TooManyFields(usize),
    /// The literal is not the initializer of a `let` or `var`.
    NotBound,
    /// The literal is at module level, where nothing is analysed.
    Unanalysed,
}

/// One struct literal and where it lives: registers, a region (`reason` is `None`) or the heap.
#[derive(Clone, Debug)]
pub struct MemorySite {
    pub span: Span,
    pub type_name: String,
    pub reason: Option<HeapReason>,
    /// The struct is held in registers: no memory at all.
    pub in_registers: bool,
}

/// A parameter of the body being analysed: its name and, when declared as a struct or class, that type's name.
pub struct ParamInfo {
    pub name: String,
    pub ty: Option<String>,
}

/// What the analysis knows about the whole program.
pub struct Env<'a> {
    /// Names of the value structs (a copy of one is a new object; a class is shared).
    pub value_structs: &'a HashSet<String>,
    /// Per analysed function or method (`f`, `Type::m`): for each parameter, whether the callee may keep it.
    pub sigs: &'a HashMap<String, Vec<bool>>,
}

/// The analysis of one function body.
#[derive(Default)]
pub struct Analysis {
    /// Spans of the `let`/`var` statements whose struct literal is placed in a region.
    pub placed: HashSet<Span>,
    /// The reason each other candidate statement is on the heap.
    pub heap: HashMap<Span, HeapReason>,
    /// The names of the placed variables.
    pub placed_names: HashSet<String>,
    /// The uses of placed names the analysis accepted; code generation refuses any other.
    pub safe_uses: HashSet<Span>,
}

/// The `ParamInfo`s of `f`; `self_type` is the struct or class `f` is a method of.
pub(crate) fn param_infos(f: &FunctionDecl, self_type: Option<&str>) -> Vec<ParamInfo> {
    f.params
        .iter()
        .map(|p| {
            let ty = match (&p.kind, &p.ty) {
                (Some(ParameterKind::SelfValue { .. }), _) => self_type.map(str::to_string),
                (_, Some(TypeNode::Named(n, _) | TypeNode::Generic(n, ..))) => Some(n.clone()),
                (_, Some(TypeNode::SelfType(_))) => self_type.map(str::to_string),
                _ => None,
            };
            ParamInfo { name: p.name.clone(), ty }
        })
        .collect()
}

/// Which `let`/`var` struct literals in a function body can live in a region.
///
/// A site qualifies when its name is declared once in the function and every use is safe: a field read
/// or write (`p.f`, `p.f = v`); a copy (a value struct bound, returned, yielded, stored in a literal or
/// captured by `spawn` is copied, a class is shared); a comparison; a match subject; or an argument or
/// receiver the callee provably does not keep. Anything else sends it to the collector heap.
pub(crate) fn analyze(stmts: &[Stmt], params: &[ParamInfo], env: &Env) -> Analysis {
    let mut walk = Walk::new(env, params);
    walk.stmts(stmts, Sc::default());
    let mut out = Analysis::default();
    for (name, span, ty) in walk.candidates.clone() {
        match walk.reason(&name, Some(&ty)) {
            Some(r) => {
                out.heap.insert(span, r);
            }
            None => {
                out.placed.insert(span);
                out.placed_names.insert(name.clone());
                out.safe_uses.extend(walk.uses.get(&name).into_iter().flatten().copied());
            }
        }
    }
    out
}

/// A function or method the callers can summarise: not generic, not variadic, not a generator.
pub(crate) fn summarizable(f: &FunctionDecl, owner_generic: bool) -> bool {
    f.generic_params.is_empty()
        && !owner_generic
        && !stmts_yield(&f.body)
        && f.params.iter().all(|p| !matches!(p.kind, Some(ParameterKind::Variadic { .. })))
}

/// For each function (`key`, declaration, owning type) whether each parameter may be kept by the callee.
///
/// A parameter is kept when the body stores, returns, captures or reassigns it, or passes it to a call
/// that keeps its own. Calls through function values, to natives and to generic functions count as keeping.
/// The dependencies between functions are solved to a fixed point, so recursion is handled.
pub(crate) fn summarize(fns: &[(String, &FunctionDecl, Option<String>)], value_structs: &HashSet<String>) -> HashMap<String, Vec<bool>> {
    let mut sigs: HashMap<String, Vec<bool>> = fns.iter().map(|(k, f, _)| (k.clone(), vec![false; f.params.len()])).collect();
    type Facts = (Vec<bool>, Vec<Vec<(String, usize)>>);
    let mut facts: Vec<Facts> = Vec::new();
    for (_, f, owner) in fns {
        let infos = param_infos(f, owner.as_deref());
        let env = Env { value_structs, sigs: &sigs };
        let mut walk = Walk::new(&env, &infos);
        walk.stmts(&f.body, Sc::default());
        let replaced = crate::codegen::CodeGenerator::replaced_params(f);
        let mut direct = Vec::new();
        let mut deps = Vec::new();
        for (i, p) in infos.iter().enumerate() {
            direct.push(replaced.contains(&i) || walk.param_kept(&p.name, p.ty.as_deref()));
            deps.push(walk.pending.iter().filter(|c| c.name == p.name).map(|c| (c.key.clone(), c.pos)).collect());
        }
        facts.push((direct, deps));
    }
    for ((key, ..), (direct, _)) in fns.iter().zip(&facts) {
        sigs.insert(key.clone(), direct.clone());
    }
    loop {
        let mut changed = false;
        for ((key, ..), (_, deps)) in fns.iter().zip(&facts) {
            for (i, ds) in deps.iter().enumerate() {
                if !sigs[key][i] && ds.iter().any(|(k, j)| sigs.get(k).is_none_or(|s| s[*j])) {
                    sigs.get_mut(key).unwrap()[i] = true;
                    changed = true;
                }
            }
        }
        if !changed {
            return sigs;
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Sc {
    lambda: bool,
    spawn: bool,
}

enum Use {
    Field,
    Copy,
    Read,
    Whole,
    Call(String, usize),
}

#[derive(Clone, Copy)]
enum Why {
    Whole,
    Captured,
}

struct Pending {
    name: String,
    span: Span,
    key: String,
    pos: usize,
    local_fn: Option<String>,
}

struct Walk<'a> {
    env: &'a Env<'a>,
    decls: HashMap<String, usize>,
    types: HashMap<String, String>,
    escaped: HashMap<String, (Span, Why)>,
    copied: HashMap<String, Span>,
    pending: Vec<Pending>,
    uses: HashMap<String, Vec<Span>>,
    candidates: Vec<(String, Span, String)>,
}

impl<'a> Walk<'a> {
    fn new(env: &'a Env<'a>, params: &[ParamInfo]) -> Self {
        let mut walk = Walk {
            env,
            decls: HashMap::new(),
            types: HashMap::new(),
            escaped: HashMap::new(),
            copied: HashMap::new(),
            pending: Vec::new(),
            uses: HashMap::new(),
            candidates: Vec::new(),
        };
        for p in params {
            walk.declare(&p.name);
            if let Some(t) = &p.ty {
                walk.types.insert(p.name.clone(), t.clone());
            }
        }
        walk
    }

    fn declare(&mut self, name: &str) {
        *self.decls.entry(name.to_string()).or_insert(0) += 1;
    }

    fn escape(&mut self, name: &str, span: Span, why: Why) {
        self.escaped.entry(name.to_string()).or_insert((span, why));
    }

    fn ok(&mut self, name: &str, span: Span) {
        self.uses.entry(name.to_string()).or_default().push(span);
    }

    fn is_value(&self, ty: Option<&str>) -> bool {
        ty.is_some_and(|t| self.env.value_structs.contains(t))
    }

    fn reason(&self, name: &str, ty: Option<&str>) -> Option<HeapReason> {
        if self.decls.get(name) != Some(&1) {
            return Some(HeapReason::Redeclared(name.to_string()));
        }
        if let Some((span, why)) = self.escaped.get(name) {
            return Some(match why {
                Why::Captured => HeapReason::Captured(name.to_string(), *span),
                Why::Whole => HeapReason::WholeValue(name.to_string(), *span),
            });
        }
        if let Some(span) = self.copied.get(name).filter(|_| !self.is_value(ty)) {
            return Some(HeapReason::WholeValue(name.to_string(), *span));
        }
        self.pending.iter().find(|c| c.name == name && self.call_keeps(c)).map(|c| HeapReason::Retained(name.to_string(), c.span))
    }

    fn call_keeps(&self, c: &Pending) -> bool {
        c.local_fn.as_ref().is_some_and(|f| self.decls.contains_key(f)) || self.env.sigs.get(&c.key).is_none_or(|s| s[c.pos])
    }

    fn param_kept(&self, name: &str, ty: Option<&str>) -> bool {
        self.reason(name, ty).is_some()
    }

    fn touch(&mut self, name: &str, span: Span, kind: Use, sc: Sc) {
        if sc.lambda {
            self.escape(name, span, Why::Captured);
        } else if sc.spawn {
            self.copied.entry(name.to_string()).or_insert(span);
        } else {
            match kind {
                Use::Field | Use::Read => self.ok(name, span),
                Use::Copy => {
                    self.copied.entry(name.to_string()).or_insert(span);
                    self.ok(name, span);
                }
                Use::Whole => self.escape(name, span, Why::Whole),
                Use::Call(key, pos) => {
                    let local_fn = (!key.contains("::")).then(|| key.clone());
                    self.pending.push(Pending { name: name.to_string(), span, key, pos, local_fn });
                    self.ok(name, span);
                }
            }
        }
    }

    fn bare(e: &Expr) -> Option<(&str, Span)> {
        match e {
            Expr::Ident(n, s) => Some((n, *s)),
            Expr::SelfValue(s) => Some(("self", *s)),
            _ => None,
        }
    }

    fn value(&mut self, e: &Expr, sc: Sc) {
        match Self::bare(e) {
            Some((n, s)) => self.touch(n, s, Use::Copy, sc),
            None => self.expr(e, sc),
        }
    }

    fn read(&mut self, e: &Expr, sc: Sc) {
        match Self::bare(e) {
            Some((n, s)) => self.touch(n, s, Use::Read, sc),
            None => self.expr(e, sc),
        }
    }

    fn stmts(&mut self, body: &[Stmt], sc: Sc) {
        for s in body {
            self.stmt(s, sc);
        }
    }

    fn arms(&mut self, arms: &[MatchArm], sc: Sc) {
        for arm in arms {
            let mut names = HashSet::new();
            crate::codegen::CodeGenerator::pattern_binding_names(&arm.pattern, &mut names);
            for n in &names {
                self.declare(n);
            }
            if let Some(g) = &arm.guard {
                self.expr(g, sc);
            }
            self.stmts(&arm.body, sc);
        }
    }

    fn stmt(&mut self, s: &Stmt, sc: Sc) {
        let nested = sc.lambda || sc.spawn;
        match s {
            Stmt::Let { name, init, span, .. } | Stmt::Var { name, init, span, .. } => {
                self.value(init, sc);
                self.declare(name);
                if let Expr::StructInit { name: ty, .. } = init {
                    self.types.insert(name.clone(), ty.clone());
                    if !nested {
                        self.candidates.push((name.clone(), *span, ty.clone()));
                    }
                }
            }
            Stmt::TupleLet { names, init, .. } => {
                self.value(init, sc);
                for n in names {
                    self.declare(n);
                }
            }
            Stmt::Assign { target, value, .. } => {
                self.target(target, sc);
                self.value(value, sc);
            }
            Stmt::CompoundAssign { target, value, .. } => {
                self.target(target, sc);
                self.expr(value, sc);
            }
            Stmt::Expr { expr, .. } => self.expr(expr, sc),
            Stmt::Return { value: Some(e), .. } => self.value(e, sc),
            Stmt::Return { value: None, .. } => {}
            Stmt::Break { value, .. } => {
                if let Some(v) = value {
                    self.expr(v, sc);
                }
            }
            Stmt::If { cond, then_branch, else_branch, .. } => {
                self.expr(cond, sc);
                self.stmts(then_branch, sc);
                if let Some(eb) = else_branch {
                    self.stmts(eb, sc);
                }
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond, sc);
                self.stmts(body, sc);
            }
            Stmt::ForIn { var_name, iter, body, .. } => {
                self.expr(iter, sc);
                self.declare(var_name);
                self.stmts(body, sc);
            }
            Stmt::Match { expr, arms, .. } => {
                self.value(expr, sc);
                self.arms(arms, sc);
            }
            Stmt::Block { body, .. } | Stmt::ScopeBlock { body, .. } => self.stmts(body, sc),
            Stmt::WithBlock { name, init, body, .. } => {
                self.expr(init, sc);
                self.declare(name);
                self.stmts(body, sc);
            }
            Stmt::SpawnBlock { body, .. } => self.stmts(body, Sc { spawn: true, ..sc }),
            Stmt::Yield { value, .. } => self.value(value, sc),
            Stmt::Continue { .. } => {}
        }
    }

    fn target(&mut self, t: &Expr, sc: Sc) {
        match t {
            Expr::MemberAccess { object, .. } => self.field_object(object, sc),
            other => self.expr(other, sc),
        }
    }

    fn field_object(&mut self, object: &Expr, sc: Sc) {
        match Self::bare(object) {
            Some((n, s)) => self.touch(n, s, Use::Field, sc),
            None => self.expr(object, sc),
        }
    }

    fn call_args(&mut self, args: &[Expr], key: Option<(&str, usize)>, sc: Sc) {
        for (i, a) in args.iter().enumerate() {
            match (Self::bare(a), key) {
                (Some((n, s)), Some((k, first))) => self.touch(n, s, Use::Call(k.to_string(), first + i), sc),
                (Some((n, s)), None) => self.touch(n, s, Use::Whole, sc),
                (None, _) => self.expr(a, sc),
            }
        }
    }

    fn call(&mut self, callee: &Expr, args: &[Expr], sc: Sc) {
        match callee {
            Expr::Ident(f, span) => {
                let known = self.env.sigs.get(f).is_some_and(|s| s.len() == args.len());
                if !known && matches!(f.as_str(), "print" | "println" | "assert_eq" | "typeof") && !self.decls.contains_key(f) {
                    for a in args {
                        self.read(a, sc);
                    }
                    return;
                }
                self.touch(f, *span, Use::Whole, sc);
                self.call_args(args, known.then_some((f.as_str(), 0)), sc);
            }
            Expr::MemberAccess { object, member, .. } => {
                let owner = Self::bare(object).and_then(|(n, _)| self.types.get(n).cloned());
                let key = owner.map(|t| format!("{t}::{member}"));
                let known = key.as_ref().filter(|k| self.env.sigs.get(*k).is_some_and(|s| s.len() == args.len() + 1));
                match (Self::bare(object), known) {
                    (Some((n, s)), Some(k)) => {
                        let k = k.clone();
                        self.touch(n, s, Use::Call(k.clone(), 0), sc);
                        self.call_args(args, Some((&k, 1)), sc);
                    }
                    (Some((n, s)), None) if member == "to_string" && args.is_empty() && key.as_ref().is_some_and(|k| !self.env.sigs.contains_key(k)) => {
                        self.touch(n, s, Use::Read, sc);
                    }
                    _ => {
                        self.expr(object, sc);
                        self.call_args(args, None, sc);
                    }
                }
            }
            other => {
                self.expr(other, sc);
                self.call_args(args, None, sc);
            }
        }
    }

    fn expr(&mut self, e: &Expr, sc: Sc) {
        match e {
            Expr::Int(..) | Expr::Float(..) | Expr::Bool(..) | Expr::String(..) | Expr::Char(..) | Expr::Null(..) => {}
            Expr::StaticAccess { .. } => {}
            Expr::Ident(name, span) => self.touch(name, *span, Use::Whole, sc),
            Expr::SelfValue(span) => self.touch("self", *span, Use::Whole, sc),
            Expr::MemberAccess { object, .. } => self.field_object(object, sc),
            Expr::OptionalChain { object, body, .. } => {
                self.expr(object, sc);
                self.expr(body, sc);
            }
            Expr::Call { callee, args, .. } => self.call(callee, args, sc),
            Expr::Binary { op: BinaryOp::Eq | BinaryOp::NotEq, left, right, .. } => {
                self.read(left, sc);
                self.read(right, sc);
            }
            Expr::Binary { left, right, .. } | Expr::NullCoalesce { left, right, .. } => {
                self.expr(left, sc);
                self.expr(right, sc);
            }
            Expr::Unary { expr, .. } | Expr::TypeTest { expr, .. } | Expr::Try { expr, .. } | Expr::Unwrap { expr, .. } => self.expr(expr, sc),
            Expr::Index { object, index, .. } => {
                self.expr(object, sc);
                self.expr(index, sc);
            }
            Expr::Range { start, end, .. } => {
                for part in [start, end].into_iter().flatten() {
                    self.expr(part, sc);
                }
            }
            Expr::Ternary { cond, then_expr, else_expr, .. } => {
                self.expr(cond, sc);
                self.value(then_expr, sc);
                self.value(else_expr, sc);
            }
            Expr::ListLiteral { elements, .. } | Expr::TupleLiteral { elements, .. } => {
                for el in elements {
                    self.value(el, sc);
                }
            }
            Expr::MapLiteral { entries, .. } => {
                for (k, v) in entries {
                    self.expr(k, sc);
                    self.value(v, sc);
                }
            }
            Expr::StructInit { fields, .. } => {
                for (_, v) in fields {
                    self.value(v, sc);
                }
            }
            Expr::Lambda { params, body, .. } => {
                for p in params {
                    self.declare(&p.name);
                }
                self.stmts(body, Sc { lambda: true, ..sc });
            }
            Expr::Spawn { body, .. } => self.stmts(body, Sc { spawn: true, ..sc }),
            Expr::If { cond, then_branch, else_branch, .. } => {
                self.expr(cond, sc);
                self.stmts(then_branch, sc);
                self.stmts(else_branch, sc);
            }
            Expr::Match { expr, arms, .. } => {
                self.value(expr, sc);
                self.arms(arms, sc);
            }
            Expr::Block { body, .. } => self.stmts(body, sc),
        }
    }
}
