use std::collections::HashSet;

use mote_base::{Diagnostic, DiagnosticSink, Interner, Span};

use crate::defs::DefKind;
use crate::program::Program;
use crate::tree::{Body, Callee, ExprKind};
use crate::walk::{reach, search, Reach};

/// Reports each `spawn` that start-up code runs and whose body reads a `var` global.
pub(super) fn check(program: &Program, interner: &Interner, sink: &mut DiagnosticSink) {
    let mut reported: HashSet<Span> = HashSet::new();
    for body in start_up(program) {
        for id in reach(body).exprs {
            let ExprKind::Spawn { task, .. } = &body.exprs[id].kind else { continue };
            let found = search(program, body, &task.body, &mut HashSet::new(), &mut |body, reached| var_global(program, interner, body, reached));
            let span = body.exprs[id].span;
            if let Some(name) = found
                && reported.insert(span)
            {
                sink.emit(Diagnostic::error(format!("a task started at start-up cannot read the `var` global `{name}`"), span));
            }
        }
    }
}

/// The initializers, the top-level code and every instance they reach: the code that runs while globals can still change.
fn start_up(program: &Program) -> Vec<&Body> {
    let globals = program.defs.iter().filter_map(|(def, _)| program.global_inits.get(&def));
    let top_levels = program.modules.iter().filter_map(|(module, _)| program.top_level.get(&module));
    let mut out: Vec<&Body> = globals.chain(top_levels).collect();
    let mut seen = HashSet::new();
    let mut queue = out.clone();
    while let Some(body) = queue.pop() {
        for id in reach(body).exprs {
            if let ExprKind::Function(function) | ExprKind::Call { callee: Callee::Function(function), .. } = &body.exprs[id].kind
                && seen.insert(function.clone())
                && let Some(next) = program.bodies.get(function)
            {
                out.push(next);
                queue.push(next);
            }
        }
    }
    out
}

/// The name of a `var` global among the nodes a block runs itself.
fn var_global(program: &Program, interner: &Interner, body: &Body, reached: &Reach) -> Option<String> {
    reached.exprs.iter().find_map(|&id| match &body.exprs[id].kind {
        ExprKind::Global(def) => match &program.defs[*def].kind {
            DefKind::Global(global) if global.is_var => Some(interner.resolve(program.defs[*def].name).to_string()),
            _ => None,
        },
        _ => None,
    })
}
