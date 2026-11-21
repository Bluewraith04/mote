use std::collections::HashSet;

use mote_base::Id;

use crate::program::{Instance, Program};
use crate::tree::{ArmBody, Block, Body, Callee, Condition, Else, Expr, ExprKind, Local, Match, Pattern, PatternKind, Stmt, StmtKind};

/// The nodes a block reaches, closures and `spawn` bodies included, in an arbitrary order.
#[derive(Default)]
pub(crate) struct Reach {
    pub exprs: Vec<Id<Expr>>,
    pub stmts: Vec<Id<Stmt>>,
}

pub(crate) fn reach(body: &Body) -> Reach {
    let mut out = Reach::default();
    Walker { body, out: &mut out, tasks: true }.block(&body.block);
    out
}

/// The nodes a block runs itself: a `spawn` body runs elsewhere and is left out.
pub(crate) fn reach_own(body: &Body, block: &Block) -> Reach {
    let mut out = Reach::default();
    Walker { body, out: &mut out, tasks: false }.block(block);
    out
}

/// The sub-expressions of a node in evaluation order, leaving out what holds blocks or branches: `If`, `Match`, closures and `spawn`.
pub(crate) fn operands(kind: &ExprKind) -> Vec<Id<Expr>> {
    match kind {
        ExprKind::Concat(items) | ExprKind::Tuple(items) | ExprKind::List(items) | ExprKind::Array(items) => items.clone(),
        ExprKind::Show(inner) | ExprKind::Cast(inner) | ExprKind::Unwrap(inner) | ExprKind::Propagate(inner) => vec![*inner],
        ExprKind::ArrayRepeat { value, len } => vec![*value, *len],
        ExprKind::Map(entries) => entries.iter().flat_map(|&(key, value)| [key, value]).collect(),
        ExprKind::Record { fields, base } => fields.iter().map(|field| field.value).chain(*base).collect(),
        ExprKind::Variant { fields, .. } => fields.iter().map(|field| field.value).collect(),
        ExprKind::Call { callee, args } => match callee {
            crate::tree::Callee::Value(value) => std::iter::once(*value).chain(args.iter().copied()).collect(),
            _ => args.clone(),
        },
        ExprKind::Field { object, .. } | ExprKind::TupleField { object, .. } => vec![*object],
        ExprKind::SetField { object, value, .. } => vec![*object, *value],
        ExprKind::Index { object, index } => vec![*object, *index],
        ExprKind::Slice { object, start, end, step, .. } => std::iter::once(*object).chain(*start).chain(*end).chain(*step).collect(),
        ExprKind::Unary { operand, .. } => vec![*operand],
        ExprKind::Binary { left, right, .. } => vec![*left, *right],
        ExprKind::Is { expr, .. } | ExprKind::Convert { expr, .. } => vec![*expr],
        ExprKind::Range { start, end, .. } => vec![*start, *end],
        _ => Vec::new(),
    }
}

/// Runs `probe` over what a block runs, then over each function it names or calls, once each.
pub(crate) fn search<T>(program: &Program, body: &Body, block: &Block, seen: &mut HashSet<Instance>, probe: &mut impl FnMut(&Body, &Reach) -> Option<T>) -> Option<T> {
    let reached = reach_own(body, block);
    if let Some(found) = probe(body, &reached) {
        return Some(found);
    }
    for &id in &reached.exprs {
        if let ExprKind::Function(function) | ExprKind::Call { callee: Callee::Function(function), .. } = &body.exprs[id].kind
            && seen.insert(function.clone())
            && let Some(next) = program.bodies.get(function)
            && let Some(found) = search(program, next, &next.block, seen, probe)
        {
            return Some(found);
        }
    }
    None
}

/// The locals a pattern binds.
pub(crate) fn pattern_locals(body: &Body, pattern: Id<Pattern>, out: &mut Vec<Id<Local>>) {
    match &body.patterns[pattern].kind {
        PatternKind::Bind(local) => out.push(*local),
        PatternKind::At { local, pattern } => {
            out.push(*local);
            pattern_locals(body, *pattern, out);
        }
        PatternKind::Or(items) | PatternKind::Tuple(items) => items.iter().for_each(|&item| pattern_locals(body, item, out)),
        PatternKind::OptionalSome(inner) => pattern_locals(body, *inner, out),
        PatternKind::Variant { fields, .. } | PatternKind::Record { fields } => fields.iter().for_each(|field| pattern_locals(body, field.pattern, out)),
        PatternKind::Type { local: Some(local), .. } => out.push(*local),
        _ => {}
    }
}

struct Walker<'a> {
    body: &'a Body,
    out: &'a mut Reach,
    tasks: bool,
}

impl Walker<'_> {
    fn block(&mut self, block: &Block) {
        for &stmt in &block.stmts {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, id: Id<Stmt>) {
        self.out.stmts.push(id);
        match &self.body.stmts[id].kind {
            StmtKind::Let { init, .. } => self.expr(*init),
            StmtKind::Assign { target, value, .. } => {
                self.expr(*value);
                self.expr(*target);
            }
            StmtKind::Const { init, .. } => self.expr(*init),
            StmtKind::If(branch) | StmtKind::ComptimeIf(branch) => {
                self.condition(&branch.condition);
                self.block(&branch.then);
                match &branch.otherwise {
                    Some(Else::Block(block)) => self.block(block),
                    Some(Else::If(other)) => self.stmt(*other),
                    None => {}
                }
            }
            StmtKind::While { condition, body } => {
                self.condition(condition);
                self.block(body);
            }
            StmtKind::For(looped) => {
                self.expr(looped.iterable);
                self.block(&looped.body);
            }
            StmtKind::ComptimeFor { iterable, body, .. } => {
                self.expr(*iterable);
                self.block(body);
            }
            StmtKind::Match(matched) => self.matched(matched),
            StmtKind::Scope(block) | StmtKind::ComptimeBlock(block) => self.block(block),
            StmtKind::With { init, body, .. } => {
                self.expr(*init);
                self.block(body);
            }
            StmtKind::Return(Some(value)) | StmtKind::Yield(value) | StmtKind::Expr(value) => self.expr(*value),
            StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue | StmtKind::Error => {}
        }
    }

    fn condition(&mut self, condition: &Condition) {
        match condition {
            Condition::Expr(expr) => self.expr(*expr),
            Condition::Let { value, .. } => self.expr(*value),
        }
    }

    fn matched(&mut self, matched: &Match) {
        self.expr(matched.subject);
        for arm in &matched.arms {
            if let Some(guard) = arm.guard {
                self.expr(guard);
            }
            match &arm.body {
                ArmBody::Value(value) => self.expr(*value),
                ArmBody::Block(block) => self.block(block),
            }
        }
    }

    fn expr(&mut self, id: Id<Expr>) {
        self.out.exprs.push(id);
        match &self.body.exprs[id].kind {
            ExprKind::If { condition, then, otherwise } => {
                self.condition(condition);
                self.expr(*then);
                self.expr(*otherwise);
            }
            ExprKind::Comptime(block) => self.block(block),
            ExprKind::ComptimeIf { condition, then, otherwise } => {
                self.expr(*condition);
                self.expr(*then);
                self.expr(*otherwise);
            }
            ExprKind::Match(matched) => self.matched(matched),
            ExprKind::Closure(closure) => self.block(&closure.body),
            ExprKind::Spawn { task, .. } => {
                if self.tasks {
                    self.block(&task.body);
                }
            }
            kind => {
                for operand in operands(kind) {
                    self.expr(operand);
                }
            }
        }
    }
}
