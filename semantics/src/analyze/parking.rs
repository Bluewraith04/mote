use std::collections::HashSet;

use mote_base::{Diagnostic, DiagnosticSink, Id, Interner, Span};

use super::method_of;
use crate::builtins::PARKING_BUILTINS;
use crate::defs::DefId;
use crate::program::{Instance, Program};
use crate::tree::{Body, Callee, Expr, ExprKind, ForKind, StmtKind};
use crate::types::BuiltinTy;
use crate::walk::{reach, search, Reach};

const PARKING: [(BuiltinTy, &str); 9] = [
    (BuiltinTy::Task, "await"),
    (BuiltinTy::Task, "await_timeout"),
    (BuiltinTy::Sender, "send"),
    (BuiltinTy::Receiver, "recv"),
    (BuiltinTy::Receiver, "recv_timeout"),
    (BuiltinTy::Mutex, "lock"),
    (BuiltinTy::Mutex, "get"),
    (BuiltinTy::Mutex, "set"),
    (BuiltinTy::Once, "get_or_init"),
];

/// Reports each `Mutex.lock` whose body reaches an operation that parks.
pub(super) fn check(program: &Program, interner: &Interner, bodies: &[&Body], sink: &mut DiagnosticSink) {
    let scan = Scan { program, interner };
    let mut reported: HashSet<Span> = HashSet::new();
    for body in bodies {
        for id in reach(body).exprs {
            let ExprKind::Call { callee: Callee::Function(function), args } = &body.exprs[id].kind else { continue };
            if method_of(program, interner, function.def) != Some((BuiltinTy::Mutex, "lock")) {
                continue;
            }
            let Some(&arg) = args.get(1) else { continue };
            let Some(op) = scan.argument(body, arg) else { continue };
            let span = body.exprs[id].span;
            if reported.insert(span) {
                sink.emit(Diagnostic::error(format!("`lock` must not park; its body reaches `{op}`"), span));
            }
        }
    }
}

struct Scan<'a> {
    program: &'a Program,
    interner: &'a Interner,
}

impl Scan<'_> {
    /// The parking operation a closure or named function argument reaches.
    fn argument(&self, body: &Body, mut arg: Id<Expr>) -> Option<String> {
        while let ExprKind::Convert { expr, .. } = &body.exprs[arg].kind {
            arg = *expr;
        }
        let mut seen = HashSet::new();
        match &body.exprs[arg].kind {
            ExprKind::Closure(closure) => search(self.program, body, &closure.body, &mut seen, &mut |body, reached| self.probe(body, reached)),
            ExprKind::Function(function) => self.function(function, &mut seen),
            _ => None,
        }
    }

    fn function(&self, function: &Instance, seen: &mut HashSet<Instance>) -> Option<String> {
        if let Some(op) = self.op(function.def) {
            return Some(op);
        }
        if !seen.insert(function.clone()) {
            return None;
        }
        let next = self.program.bodies.get(function)?;
        search(self.program, next, &next.block, seen, &mut |body, reached| self.probe(body, reached))
    }

    /// The parking operation among the nodes a block runs itself.
    fn probe(&self, body: &Body, reached: &Reach) -> Option<String> {
        for &stmt in &reached.stmts {
            if let StmtKind::For(looped) = &body.stmts[stmt].kind {
                match looped.kind {
                    ForKind::Receiver => return Some("for over a Receiver".to_string()),
                    ForKind::Stream => return Some("for over a Stream".to_string()),
                    ForKind::Range | ForKind::Indexed => {}
                }
            }
        }
        reached.exprs.iter().find_map(|&id| match &body.exprs[id].kind {
            ExprKind::Function(function) | ExprKind::Call { callee: Callee::Function(function), .. } => self.op(function.def),
            ExprKind::Call { callee: Callee::Builtin { name, .. }, .. } => {
                let raw = self.interner.resolve(*name);
                PARKING_BUILTINS.contains(&raw).then(|| raw.to_string())
            }
            _ => None,
        })
    }

    /// What a function is called when calling it parks.
    fn op(&self, def: DefId) -> Option<String> {
        let item = &self.program.defs[def];
        if let Some((builtin, name)) = method_of(self.program, self.interner, def)
            && PARKING.iter().any(|&(parks, method)| parks == builtin && method == name)
        {
            return Some(format!("{}.{name}", builtin.name()));
        }
        let function = item.function()?;
        let parks = function.link.is_some_and(|link| link.blocking);
        parks.then(|| self.interner.resolve(item.name).to_string())
    }
}
