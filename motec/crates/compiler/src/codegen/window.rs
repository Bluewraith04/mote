//! Struct locals held in registers instead of an object.

use super::abi::AbiSig;
use super::*;

pub(crate) const MAX_WINDOW_SLOTS: usize = 8;

pub(crate) enum ArgKind {
    Lit,
    Name(String),
    Other,
}

pub(crate) struct Site {
    pub key: String,
    pub let_span: Option<Span>,
    pub args: Vec<(usize, ArgKind)>,
}

#[derive(Default)]
pub(crate) struct Planned {
    pub lets: HashSet<Span>,
    pub windows: HashSet<String>,
    pub call_lets: HashMap<Span, String>,
    pub sites: Vec<Site>,
    pub unknown: HashSet<String>,
}

enum Recv {
    Struct(String),
    Other,
    Unknown,
}

struct Scan<'a> {
    window_types: &'a HashMap<String, (u16, usize)>,
    types: &'a [TypeDescriptor],
    abi: &'a HashMap<String, AbiSig>,
    checker: &'a crate::stage::TypeTable,
    owner: Option<&'a str>,
    owner_ret: bool,
    candidates: Vec<(String, Span, String)>,
    call_lets: HashMap<Span, String>,
    let_calls: HashMap<Span, Span>,
    local_types: HashMap<String, String>,
    param_windows: Vec<(String, String)>,
    sites: Vec<Site>,
    unknown: HashSet<String>,
    decls: HashMap<String, usize>,
    unfit: HashSet<String>,
    members: HashMap<String, HashSet<String>>,
}

impl Scan<'_> {
    fn decl(&mut self, name: &str) {
        *self.decls.entry(name.to_string()).or_default() += 1;
    }

    fn call_target<'e>(&self, callee: &'e Expr) -> Option<(String, Option<&'e Expr>)> {
        match callee {
            Expr::Ident(f, _) if !self.decls.contains_key(f) => self.abi.contains_key(f).then(|| (f.clone(), None)),
            Expr::MemberAccess { object, member, .. } => {
                let Recv::Struct(ty) = self.receiver(object) else { return None };
                let key = CodeGenerator::mangle_method_name(&ty, member);
                self.abi.get(&key).filter(|s| s.has_self).map(|_| (key, Some(object.as_ref())))
            }
            _ => None,
        }
    }

    fn receiver(&self, object: &Expr) -> Recv {
        let local = match object {
            Expr::Ident(n, _) => self.local_types.get(n).cloned(),
            Expr::SelfValue(_) => self.owner.map(str::to_string),
            _ => None,
        };
        if let Some(ty) = local {
            return Recv::Struct(ty);
        }
        match self.checker.of(object) {
            Some(Type::Struct { name, .. } | Type::Class { name, .. }) => Recv::Struct(name.clone()),
            Some(Type::Int | Type::Float | Type::Bool | Type::Char | Type::String | Type::Null | Type::Bytes | Type::List(_) | Type::Map(..) | Type::Set(_) | Type::Tuple(_) | Type::Stream(_) | Type::Task(_) | Type::Shared(_)) => Recv::Other,
            _ => Recv::Unknown,
        }
    }

    fn arg(&mut self, e: &Expr, key: &str, pos: usize, in_lambda: bool, out: &mut Vec<(usize, ArgKind)>) {
        let window = self.abi[key].params.get(pos).is_some_and(Option::is_some);
        let kind = match e {
            Expr::Ident(n, _) if window && !in_lambda => ArgKind::Name(n.clone()),
            Expr::SelfValue(_) if window && !in_lambda => ArgKind::Name("self".to_string()),
            Expr::StructInit { .. } if window => {
                self.expr(e, in_lambda);
                ArgKind::Lit
            }
            _ => {
                self.expr(e, in_lambda);
                ArgKind::Other
            }
        };
        if window {
            out.push((pos, kind));
        }
    }

    fn member(&mut self, name: &str, member: &str, in_lambda: bool) {
        self.members.entry(name.to_string()).or_default().insert(member.to_string());
        if in_lambda {
            self.unfit.insert(name.to_string());
        }
    }

    fn stmts(&mut self, body: &[Stmt], in_lambda: bool) {
        for s in body {
            self.stmt(s, in_lambda);
        }
    }

    fn stmt(&mut self, s: &Stmt, in_lambda: bool) {
        match s {
            Stmt::Let { name, init, span, .. } | Stmt::Var { name, init, span, .. } => {
                if let (false, Expr::Call { span: call, .. }) = (in_lambda, init) {
                    self.let_calls.insert(*call, *span);
                }
                self.expr(init, in_lambda);
                self.decl(name);
                if in_lambda {
                    return;
                }
                let local = match init {
                    Expr::StructInit { name: ty, .. } if self.window_types.contains_key(ty) => Some(ty.clone()),
                    Expr::Call { callee, .. } => self.call_target(callee).and_then(|(key, _)| {
                        let ty = self.abi[&key].ret.clone()?;
                        self.call_lets.insert(*span, key);
                        Some(ty)
                    }),
                    _ => None,
                };
                if let Some(ty) = local {
                    self.local_types.insert(name.clone(), ty.clone());
                    self.candidates.push((name.clone(), *span, ty));
                }
            }
            Stmt::TupleLet { names, init, .. } => {
                self.expr(init, in_lambda);
                for n in names {
                    self.decl(n);
                }
            }
            Stmt::Assign { target, value, .. } => {
                self.target(target, in_lambda);
                self.expr(value, in_lambda);
            }
            Stmt::CompoundAssign { target, op, value, .. } => {
                self.target(target, in_lambda);
                if let (None, Expr::MemberAccess { object, .. }) = (op.binary_op(), target) {
                    self.expr(object, in_lambda);
                }
                self.expr(value, in_lambda);
            }
            Stmt::Expr { expr, .. } | Stmt::Yield { value: expr, .. } => self.expr(expr, in_lambda),
            Stmt::Return { value: Some(Expr::Ident(..)), .. } if self.owner_ret && !in_lambda => {}
            Stmt::Return { value, .. } | Stmt::Break { value, .. } => {
                if let Some(v) = value {
                    self.expr(v, in_lambda);
                }
            }
            Stmt::If { cond, then_branch, else_branch, .. } => {
                self.expr(cond, in_lambda);
                self.stmts(then_branch, in_lambda);
                if let Some(eb) = else_branch {
                    self.stmts(eb, in_lambda);
                }
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond, in_lambda);
                self.stmts(body, in_lambda);
            }
            Stmt::ForIn { var_name, iter, body, .. } => {
                self.expr(iter, in_lambda);
                self.decl(var_name);
                self.stmts(body, in_lambda);
            }
            Stmt::Match { expr, arms, .. } => {
                self.expr(expr, in_lambda);
                self.arms(arms, in_lambda);
            }
            Stmt::ScopeBlock { body, .. } | Stmt::Block { body, .. } => self.stmts(body, in_lambda),
            Stmt::SpawnBlock { body, .. } => self.stmts(body, true),
            Stmt::WithBlock { name, init, body, .. } => {
                self.expr(init, in_lambda);
                self.decl(name);
                self.stmts(body, in_lambda);
            }
            Stmt::Continue { .. } => {}
        }
    }

    fn target(&mut self, t: &Expr, in_lambda: bool) {
        match t {
            Expr::MemberAccess { object, member, .. } => match object.as_ref() {
                Expr::Ident(n, _) => self.member(n, member, in_lambda),
                Expr::SelfValue(_) => self.member("self", member, in_lambda),
                other => self.expr(other, in_lambda),
            },
            other => self.expr(other, in_lambda),
        }
    }

    fn arms(&mut self, arms: &[MatchArm], in_lambda: bool) {
        for arm in arms {
            let mut names = Vec::new();
            pattern_names(&arm.pattern, &mut names);
            for n in names {
                self.decl(&n);
            }
            if let Some(g) = &arm.guard {
                self.expr(g, in_lambda);
            }
            self.stmts(&arm.body, in_lambda);
        }
    }

    fn expr(&mut self, e: &Expr, in_lambda: bool) {
        match e {
            Expr::Int(..) | Expr::Float(..) | Expr::Bool(..) | Expr::String(..) | Expr::Char(..) | Expr::Null(_) | Expr::StaticAccess { .. } => {}
            Expr::Ident(n, _) => {
                self.unfit.insert(n.clone());
            }
            Expr::SelfValue(_) => {
                self.unfit.insert("self".to_string());
            }
            Expr::Binary { left, right, .. } | Expr::NullCoalesce { left, right, .. } => {
                self.expr(left, in_lambda);
                self.expr(right, in_lambda);
            }
            Expr::Unary { expr, .. } | Expr::TypeTest { expr, .. } | Expr::Try { expr, .. } | Expr::Unwrap { expr, .. } => self.expr(expr, in_lambda),
            Expr::Call { callee, args, span } => {
                if let Some((key, receiver)) = self.call_target(callee) {
                    let mut site = Site { key: key.clone(), let_span: self.let_calls.get(span).copied(), args: Vec::new() };
                    let mut pos = 0;
                    if let Some(r) = receiver {
                        self.arg(r, &key, pos, in_lambda, &mut site.args);
                        pos += 1;
                    }
                    for a in args {
                        self.arg(a, &key, pos, in_lambda, &mut site.args);
                        pos += 1;
                    }
                    self.sites.push(site);
                    return;
                }
                match callee.as_ref() {
                    Expr::MemberAccess { object, member, .. } => {
                        if matches!(self.receiver(object), Recv::Unknown) {
                            self.unknown.insert(member.clone());
                        }
                        self.expr(object, in_lambda);
                    }
                    other => self.expr(other, in_lambda),
                }
                for a in args {
                    self.expr(a, in_lambda);
                }
            }
            Expr::MemberAccess { object, member, .. } => match object.as_ref() {
                Expr::Ident(n, _) => self.member(n, member, in_lambda),
                Expr::SelfValue(_) => self.member("self", member, in_lambda),
                other => self.expr(other, in_lambda),
            },
            Expr::OptionalChain { object, temp, body, .. } => {
                self.expr(object, in_lambda);
                self.decl(temp);
                self.expr(body, in_lambda);
            }
            Expr::Index { object, index, .. } => {
                self.expr(object, in_lambda);
                self.expr(index, in_lambda);
            }
            Expr::Range { start, end, .. } => {
                for x in [start, end].into_iter().flatten() {
                    self.expr(x, in_lambda);
                }
            }
            Expr::Ternary { cond, then_expr, else_expr, .. } => {
                self.expr(cond, in_lambda);
                self.expr(then_expr, in_lambda);
                self.expr(else_expr, in_lambda);
            }
            Expr::ListLiteral { elements, .. } | Expr::TupleLiteral { elements, .. } => {
                for el in elements {
                    self.expr(el, in_lambda);
                }
            }
            Expr::MapLiteral { entries, .. } => {
                for (k, v) in entries {
                    self.expr(k, in_lambda);
                    self.expr(v, in_lambda);
                }
            }
            Expr::StructInit { fields, .. } => {
                for (_, v) in fields {
                    self.expr(v, in_lambda);
                }
            }
            Expr::Lambda { params, body, .. } => {
                for p in params {
                    self.decl(&p.name);
                }
                self.stmts(body, true);
            }
            Expr::If { cond, then_branch, else_branch, .. } => {
                self.expr(cond, in_lambda);
                self.stmts(then_branch, in_lambda);
                self.stmts(else_branch, in_lambda);
            }
            Expr::Match { expr, arms, .. } => {
                self.expr(expr, in_lambda);
                self.arms(arms, in_lambda);
            }
            Expr::Spawn { body, .. } => self.stmts(body, true),
            Expr::Block { body, .. } => self.stmts(body, in_lambda),
        }
    }

    fn fit(&self, name: &str, ty: &str) -> bool {
        let (idx, _) = self.window_types[ty];
        let fields = &self.types[idx as usize].fields;
        let known = self.members.get(name).is_none_or(|ms| ms.iter().all(|m| fields.iter().any(|f| f.name.as_deref() == Some(m.as_str()))));
        self.decls.get(name) == Some(&1) && !self.unfit.contains(name) && known
    }

    fn planned(mut self) -> Planned {
        let mut out = Planned::default();
        for (name, span, ty) in &self.candidates {
            if self.fit(name, ty) {
                out.lets.insert(*span);
                out.windows.insert(name.clone());
                if let Some(key) = self.call_lets.get(span) {
                    out.call_lets.insert(*span, key.clone());
                }
            }
        }
        for (name, ty) in &self.param_windows {
            if self.fit(name, ty) {
                out.windows.insert(name.clone());
            }
        }
        out.sites = std::mem::take(&mut self.sites);
        out.unknown = std::mem::take(&mut self.unknown);
        out
    }
}

fn pattern_names(p: &Pattern, out: &mut Vec<String>) {
    let field_names = |fields: &[FieldPattern], out: &mut Vec<String>| {
        for f in fields {
            match &f.pattern {
                Some(sub) => pattern_names(sub, out),
                None => out.push(f.name.clone()),
            }
        }
    };
    match p {
        Pattern::Wildcard(_) | Pattern::Literal(..) => {}
        Pattern::Identifier { name, subpattern, .. } => {
            out.push(name.clone());
            if let Some(sub) = subpattern {
                pattern_names(sub, out);
            }
        }
        Pattern::Tuple(subs, _) | Pattern::Or(subs, _) => {
            for s in subs {
                pattern_names(s, out);
            }
        }
        Pattern::Struct { fields, .. } => field_names(fields, out),
        Pattern::Enum { payload, .. } => match payload {
            EnumPatternPayload::None => {}
            EnumPatternPayload::Tuple(subs) => {
                for s in subs {
                    pattern_names(s, out);
                }
            }
            EnumPatternPayload::Struct { fields, .. } => field_names(fields, out),
        },
        Pattern::Type { name, .. } => out.extend(name.clone()),
        Pattern::Range { start, end, .. } => {
            pattern_names(start, out);
            pattern_names(end, out);
        }
    }
}

fn writes_only_a(word: u32) -> bool {
    use Opcode::*;
    matches!(
        Opcode::try_from((word & 0xFF) as u8),
        Ok(ADD | SUB | MUL | DIV | MOD | NEG | AND | OR | NOT | XOR | BAND | BOR | BXOR | BNOT | SHL | SHR | EQ | NE | LT | LE | GT | GE
            | GETFIELD | MOVE | LOADI | LOADK | NEWSTR | GETGLOBAL)
    )
}

impl CodeGenerator {
    pub(crate) fn plan_windows(&mut self, body: &[Stmt], params: &[String]) {
        let sig = self.current_sig.clone().unwrap_or_default();
        let named: Vec<(String, Option<String>)> = params.iter().enumerate().map(|(i, p)| (p.clone(), sig.params.get(i).cloned().flatten())).collect();
        let owner = self.current_owner.clone();
        let planned = self.scan_windows(body, &named, owner.as_deref(), sig.ret.is_some());
        self.window_lets = planned.lets;
        self.window_params = planned.windows;
    }

    pub(crate) fn scan_windows(&self, body: &[Stmt], params: &[(String, Option<String>)], owner: Option<&str>, owner_ret: bool) -> Planned {
        let mut scan = Scan {
            window_types: &self.window_types,
            types: &self.type_descriptors,
            abi: &self.abi,
            checker: &self.types,
            owner,
            owner_ret,
            candidates: Vec::new(),
            call_lets: HashMap::new(),
            let_calls: HashMap::new(),
            local_types: HashMap::new(),
            param_windows: Vec::new(),
            sites: Vec::new(),
            unknown: HashSet::new(),
            decls: HashMap::new(),
            unfit: HashSet::new(),
            members: HashMap::new(),
        };
        for (name, ty) in params {
            scan.decl(name);
            if let Some(ty) = ty {
                scan.local_types.insert(name.clone(), ty.clone());
                scan.param_windows.push((name.clone(), ty.clone()));
            }
        }
        scan.stmts(body, false);
        scan.planned()
    }

    pub(crate) fn window_slot_reg(&self, object: &Expr, member: &str) -> Option<u8> {
        let name = match object {
            Expr::Ident(name, _) => name.as_str(),
            Expr::SelfValue(_) => "self",
            _ => return None,
        };
        let w = self.reg_alloc.window(name)?;
        let slot = self.type_descriptors[w.ty as usize].fields.iter().find(|f| f.name.as_deref() == Some(member))?.slot as u8;
        Some(w.base + slot)
    }

    pub(crate) fn compile_window_let(&mut self, name: &str, init: &Expr) -> Result<(), (String, Span)> {
        match init {
            Expr::StructInit { name: type_name, fields, span, .. } => {
                let (type_idx, slots) = self.window_types[type_name];
                let base = self.build_window(type_name, fields, *span)?;
                self.reg_alloc.bind_window(name, base, slots, type_idx);
                Ok(())
            }
            _ => self.compile_window_call_let(name, init),
        }
    }

    fn build_window(&mut self, type_name: &str, fields: &[(String, Expr)], span: Span) -> Result<u8, (String, Span)> {
        let base = self.reg_alloc.alloc_block(self.window_types[type_name].1);
        self.fill_window(base, type_name, fields, span)?;
        Ok(base)
    }

    pub(crate) fn fill_window(&mut self, base: u8, type_name: &str, fields: &[(String, Expr)], span: Span) -> Result<(), (String, Span)> {
        let (type_idx, slots) = self.window_types[type_name];
        let mut set = vec![false; slots];
        for (i, (field, value)) in fields.iter().enumerate() {
            let slot = self.field_slot(type_idx, field, i) as usize;
            if slot >= slots {
                return Err((format!("Struct '{type_name}' has no field '{field}'"), span));
            }
            self.emit_into(base + slot as u8, value)?;
            set[slot] = true;
        }
        for (i, _) in set.iter().enumerate().filter(|(_, done)| !**done) {
            self.emit_load_null(base + i as u8);
        }
        Ok(())
    }

    fn compile_window_call_let(&mut self, name: &str, init: &Expr) -> Result<(), (String, Span)> {
        let Expr::Call { callee, span, .. } = init else { unreachable!("a window `let` has a struct literal or a call") };
        let (key, keep) = match callee.as_ref() {
            Expr::Ident(f, _) => (f.clone(), *span),
            Expr::MemberAccess { object, member, span: m_span } => (Self::mangle_method_name(self.static_class_name(object).unwrap_or_default(), member), *m_span),
            _ => unreachable!("a window `let` calls a function or method"),
        };
        let ty = self.abi.get(&key).and_then(|s| s.ret.clone());
        let plain = self.types.any_check(*span).is_none() && self.types.rewrap(*span) == 0;
        if plain && ty.is_some() {
            self.keep_wr = Some(keep);
        }
        self.wr_result = None;
        let r = self.compile_value(init)?;
        self.keep_wr = None;
        match (self.wr_result.take(), ty) {
            (Some(base), Some(ty)) => {
                let (type_idx, slots) = self.window_types[&ty];
                self.reg_alloc.bind_window(name, base, slots, type_idx);
            }
            _ => self.reg_alloc.bind_var(name, r),
        }
        Ok(())
    }

    pub(crate) fn emit_window_arg(&mut self, arg: &Expr, ty: &str, base: u8) -> Result<(), (String, Span)> {
        let slots = self.window_types[ty].1;
        if let Expr::StructInit { name, fields, span, .. } = arg
            && name == ty {
                if self.report_memory {
                    self.note_window_site(arg);
                }
                return self.fill_window(base, ty, fields, *span);
            }
        let window = match arg {
            Expr::Ident(n, _) => self.reg_alloc.window(n),
            Expr::SelfValue(_) => self.reg_alloc.window("self"),
            _ => None,
        };
        if let Some(w) = window {
            for k in 0..slots {
                self.current_insts.push(encode_r2(Opcode::MOVE, base + k as u8, w.base + k as u8));
            }
            return Ok(());
        }
        let obj = self.compile_expr(arg)?;
        for k in 0..slots {
            self.current_insts.push(encode_r3(Opcode::GETFIELD, base + k as u8, obj, k as u8));
        }
        self.reg_alloc.free_temp(obj);
        Ok(())
    }

    pub(crate) fn finish_wr_call(&mut self, ty_name: &str, dest: u8, block: usize, span: Span) -> u8 {
        let (type_idx, slots) = self.window_types[ty_name];
        let rest = dest.wrapping_add(slots as u8);
        if self.keep_wr == Some(span) {
            self.keep_wr = None;
            self.wr_result = Some(dest);
            self.reg_alloc.free_block(rest, block.saturating_sub(slots));
            return dest;
        }
        let obj = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::NEWOBJ, obj, type_idx));
        for i in 0..slots {
            self.current_insts.push(encode_r3(Opcode::SETFIELD, obj, i as u8, dest + i as u8));
        }
        self.current_insts.push(encode_r2(Opcode::MOVE, dest, obj));
        self.reg_alloc.free_temp(obj);
        self.reg_alloc.free_block(dest.wrapping_add(1), block.max(slots) - 1);
        dest
    }

    pub(crate) fn compile_wr_return(&mut self, ty_name: &str, val: &Expr) -> Result<(), (String, Span)> {
        let (_, slots) = self.window_types[ty_name];
        let window = match val {
            Expr::Ident(n, _) => self.reg_alloc.window(n),
            _ => None,
        };
        let (base, owned) = match (val, window) {
            (_, Some(w)) => (w.base, false),
            (Expr::StructInit { name, fields, span, .. }, None) if name == ty_name => {
                if self.report_memory {
                    self.note_window_site(val);
                }
                (self.build_window(name, fields, *span)?, true)
            }
            _ => {
                let obj = self.compile_returned(val)?;
                let base = self.reg_alloc.alloc_block(slots);
                for i in 0..slots {
                    self.current_insts.push(encode_r3(Opcode::GETFIELD, base + i as u8, obj, i as u8));
                }
                self.reg_alloc.free_temp(obj);
                (base, true)
            }
        };
        self.exit_regions_above(0);
        self.exit_withs_above(0);
        self.current_insts.push(encode_r2(Opcode::RETN, base, slots as u8));
        if owned {
            self.reg_alloc.free_block(base, slots);
        }
        Ok(())
    }

    pub(crate) fn note_window_site(&mut self, init: &Expr) {
        let Expr::StructInit { name, span, .. } = init else { return };
        self.reported_inits.insert(*span);
        self.memory_sites.push(crate::regions::MemorySite { span: *span, type_name: name.clone(), reason: None, in_registers: true });
    }

    pub(crate) fn emit_into(&mut self, dest: u8, e: &Expr) -> Result<(), (String, Span)> {
        let before = self.current_insts.len();
        let r = self.compile_value(e)?;
        let simple = matches!(
            e,
            Expr::Int(..) | Expr::Float(..) | Expr::Bool(..) | Expr::Char(..) | Expr::Null(_) | Expr::String(..) | Expr::Ident(..)
                | Expr::Unary { .. } | Expr::MemberAccess { .. }
        ) || matches!(e, Expr::Binary { op, .. } if !matches!(op, BinaryOp::And | BinaryOp::Or));
        let plain = self.types.any_check(e.span()).is_none() && self.types.rewrap(e.span()) == 0;
        let last = self.current_insts.last().copied();
        match last {
            Some(word) if simple && plain && self.current_insts.len() > before && writes_only_a(word) && ((word >> 8) & 0xFF) as u8 == r => {
                *self.current_insts.last_mut().unwrap() = (word & !0xFF00) | ((dest as u32) << 8);
            }
            _ => self.current_insts.push(encode_r2(Opcode::MOVE, dest, r)),
        }
        self.reg_alloc.free_temp(r);
        Ok(())
    }

    pub(crate) fn try_window_assign(&mut self, target: &Expr, value: &Expr) -> Result<bool, (String, Span)> {
        let Expr::MemberAccess { object, member, .. } = target else { return Ok(false) };
        let Some(reg) = self.window_slot_reg(object, member) else { return Ok(false) };
        self.emit_into(reg, value)?;
        Ok(true)
    }

    pub(crate) fn try_window_compound(&mut self, target: &Expr, opcode: Opcode, symbol: &str, guard: bool, value: &Expr) -> Result<bool, (String, Span)> {
        let Expr::MemberAccess { object, member, .. } = target else { return Ok(false) };
        let Some(reg) = self.window_slot_reg(object, member) else { return Ok(false) };
        let val_reg = self.compile_expr(value)?;
        if guard {
            self.emit_operand_guard("__operand_check", symbol, &[reg, val_reg]);
        }
        self.current_insts.push(encode_r3(opcode, reg, reg, val_reg));
        self.reg_alloc.free_temp(val_reg);
        Ok(true)
    }

    pub(crate) fn try_window_read(&mut self, object: &Expr, member: &str) -> Option<u8> {
        let reg = self.window_slot_reg(object, member)?;
        let dest = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r2(Opcode::MOVE, dest, reg));
        Some(dest)
    }
}
