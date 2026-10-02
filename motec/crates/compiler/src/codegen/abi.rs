//! Structs passed and returned in registers: which functions and methods take or give a struct as its fields.

use super::window::{ArgKind, Planned};
use super::*;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AbiSig {
    pub ret: Option<String>,
    pub params: Vec<Option<String>>,
    pub has_self: bool,
}

impl AbiSig {
    fn is_empty(&self) -> bool {
        self.ret.is_none() && self.params.iter().all(Option::is_none)
    }
}

struct Def<'a> {
    key: String,
    f: &'a FunctionDecl,
    owner: Option<&'a str>,
    generic_owner: bool,
}

#[derive(Default)]
struct Facts {
    total: HashMap<String, usize>,
    ret_ok: HashMap<String, usize>,
    arg_ok: HashMap<(String, usize), usize>,
    unfit: HashSet<(String, usize)>,
    unknown: HashSet<String>,
}

impl Facts {
    fn add(&mut self, planned: &Planned, real: bool) {
        for site in &planned.sites {
            *self.total.entry(site.key.clone()).or_default() += 1;
            if !real {
                continue;
            }
            if site.let_span.is_some_and(|s| planned.call_lets.contains_key(&s)) {
                *self.ret_ok.entry(site.key.clone()).or_default() += 1;
            }
            for (pos, kind) in &site.args {
                let good = match kind {
                    ArgKind::Lit => true,
                    ArgKind::Name(n) => planned.windows.contains(n),
                    ArgKind::Other => false,
                };
                if good {
                    *self.arg_ok.entry((site.key.clone(), *pos)).or_default() += 1;
                }
            }
        }
        self.unknown.extend(planned.unknown.iter().cloned());
    }
}

impl CodeGenerator {
    pub(crate) fn plan_window_abi(&mut self, modules: &[&[Item]]) {
        let mut defs: Vec<Def> = Vec::new();
        let mut loose: Vec<&[Stmt]> = Vec::new();
        for item in modules.iter().flat_map(|m| m.iter()) {
            match item {
                Item::Function(f) => defs.push(Def { key: f.name.clone(), f, owner: None, generic_owner: false }),
                Item::Struct(s) => defs.extend(s.methods.iter().map(|m| Def { key: Self::mangle_method_name(&s.name, &m.name), f: m, owner: Some(&s.name), generic_owner: !s.generic_params.is_empty() })),
                Item::Class(c) => defs.extend(c.methods.iter().map(|m| Def { key: Self::mangle_method_name(&c.name, &m.name), f: m, owner: Some(&c.name), generic_owner: !c.generic_params.is_empty() })),
                Item::Enum(e) => defs.extend(e.methods.iter().map(|m| Def { key: Self::mangle_method_name(&e.name, &m.name), f: m, owner: Some(&e.name), generic_owner: !e.generic_params.is_empty() })),
                Item::Trait(t) => loose.extend(t.members.iter().filter_map(|m| m.default_body.as_deref())),
                Item::TopLevelStmt(s) => loose.push(std::slice::from_ref(s)),
                _ => {}
            }
        }
        self.abi.clear();
        for d in &defs {
            if let Some(sig) = self.signature(d) {
                self.abi.insert(d.key.clone(), sig);
            }
        }
        self.drop_escaping(&defs, &loose);
        loop {
            let mut facts = Facts::default();
            for d in &defs {
                let sig = self.abi.get(&d.key);
                let params: Vec<(String, Option<String>)> =
                    d.f.params.iter().enumerate().map(|(j, p)| (p.name.clone(), sig.and_then(|s| s.params.get(j).cloned().flatten()))).collect();
                let planned = self.scan_windows(&d.f.body, &params, d.owner, sig.is_some_and(|s| s.ret.is_some()));
                for (j, (name, ty)) in params.iter().enumerate() {
                    if ty.is_some() && !planned.windows.contains(name) {
                        facts.unfit.insert((d.key.clone(), j));
                    }
                }
                facts.add(&planned, true);
            }
            for body in &loose {
                facts.add(&self.scan_windows(body, &[], None, false), false);
            }
            if !self.prune_abi(&facts) {
                break;
            }
        }
    }

    fn prune_abi(&mut self, facts: &Facts) -> bool {
        let mut changed = false;
        for (key, sig) in self.abi.iter_mut() {
            let total = facts.total.get(key).copied().unwrap_or(0);
            let unknown = key.rsplit_once("::").is_some_and(|(_, m)| facts.unknown.contains(m));
            let dead = total == 0 || unknown;
            if sig.ret.is_some() && (dead || facts.ret_ok.get(key).copied().unwrap_or(0) != total) {
                sig.ret = None;
                changed = true;
            }
            for (j, p) in sig.params.iter_mut().enumerate() {
                let at = (key.clone(), j);
                if p.is_some() && (dead || facts.unfit.contains(&at) || facts.arg_ok.get(&at).copied().unwrap_or(0) != total) {
                    *p = None;
                    changed = true;
                }
            }
        }
        self.abi.retain(|_, s| !s.is_empty());
        changed
    }

    fn drop_escaping(&mut self, defs: &[Def], loose: &[&[Stmt]]) {
        let mut idents: HashMap<String, usize> = HashMap::new();
        let mut calls: HashMap<String, usize> = HashMap::new();
        let mut count = |node: cells::Node| match node {
            cells::Node::Expr(Expr::Call { callee, .. }) => {
                if let Expr::Ident(n, _) = callee.as_ref() {
                    *calls.entry(n.clone()).or_default() += 1;
                }
            }
            cells::Node::Expr(Expr::Ident(n, _)) => *idents.entry(n.clone()).or_default() += 1,
            _ => {}
        };
        for d in defs {
            cells::walk_stmts(&d.f.body, &mut count);
        }
        loose.iter().for_each(|b| cells::walk_stmts(b, &mut count));
        self.abi.retain(|key, _| key.contains("::") || idents.get(key) == calls.get(key));
    }

    fn signature(&self, d: &Def) -> Option<AbiSig> {
        let f = d.f;
        let plain = f.generic_params.is_empty()
            && !d.generic_owner
            && !self.hidden_params.contains_key(&d.key)
            && !self.variadic_fns.contains_key(&d.key)
            && !self.default_fns.contains_key(&d.key)
            && d.key != "main"
            && !stmts_yield(&f.body)
            && f.params.iter().all(|p| !p.is_mut && matches!(p.kind, Some(ParameterKind::Regular { .. } | ParameterKind::SelfValue { is_var: false })));
        if !plain {
            return None;
        }
        let in_registers = |n: &str| self.window_types.contains_key(n).then(|| n.to_string());
        let params = f
            .params
            .iter()
            .map(|p| match (&p.kind, &p.ty) {
                (Some(ParameterKind::SelfValue { .. }), _) => d.owner.and_then(in_registers),
                (_, Some(TypeNode::Named(n, _))) => in_registers(n),
                _ => None,
            })
            .collect();
        let ret = match &f.return_type {
            Some(TypeNode::Named(n, _)) if Self::returns_literals(&f.body) => in_registers(n),
            _ => None,
        };
        let has_self = matches!(f.params.first().and_then(|p| p.kind.as_ref()), Some(ParameterKind::SelfValue { .. }));
        let sig = AbiSig { ret, params, has_self };
        (!sig.is_empty()).then_some(sig)
    }

    fn returns_literals(body: &[Stmt]) -> bool {
        let (mut any, mut all) = (false, true);
        cells::walk_stmts(body, &mut |node| {
            if let cells::Node::Stmt(Stmt::Return { value, .. }) = node {
                any = true;
                all &= matches!(value, Some(Expr::StructInit { .. } | Expr::Ident(..)));
            }
        });
        any && all
    }

    pub(crate) fn param_width(&self, sig: &AbiSig, j: usize) -> usize {
        sig.params.get(j).and_then(Option::as_ref).map_or(1, |ty| self.window_types[ty].1)
    }

    pub(crate) fn abi_width(&self, sig: &AbiSig, count: usize) -> usize {
        (0..count).map(|j| self.param_width(sig, j)).sum()
    }

    pub(crate) fn abi_params(&self, f: &FunctionDecl, sig: &AbiSig) -> Vec<String> {
        let mut names = Vec::new();
        for (j, p) in f.params.iter().enumerate() {
            match sig.params.get(j).and_then(Option::as_ref) {
                Some(ty) => names.extend((0..self.window_types[ty].1).map(|k| format!("{}${k}", p.name))),
                None => names.push(p.name.clone()),
            }
        }
        names
    }

    pub(crate) fn bind_window_params(&mut self, f: &FunctionDecl) {
        let Some(sig) = self.current_sig.clone() else { return };
        let mut reg = 0usize;
        for (j, p) in f.params.iter().enumerate() {
            let Some(ty) = sig.params.get(j).cloned().flatten() else {
                reg += 1;
                continue;
            };
            let (type_idx, slots) = self.window_types[&ty];
            if self.window_params.contains(&p.name) {
                self.reg_alloc.bind_window(&p.name, reg as u8, slots, type_idx);
            } else {
                let obj = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_ri(Opcode::NEWOBJ, obj, type_idx));
                for k in 0..slots {
                    self.current_insts.push(encode_r3(Opcode::SETFIELD, obj, k as u8, (reg + k) as u8));
                }
                self.reg_alloc.bind_var(&p.name, obj);
            }
            reg += slots;
        }
    }
}
