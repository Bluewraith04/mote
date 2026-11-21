use std::collections::HashSet;

use mote_base::{Diagnostic, DiagnosticSink, Id, Interner, Span};

use crate::program::Program;
use crate::tree::{ArmBody, Block, Body, Condition, Else, Expr, ExprKind, IfStmt, Local, Match, Pattern, Stmt, StmtKind};
use crate::types::{BuiltinTy, TyKind};
use crate::walk::{operands, pattern_locals};

/// The channel ends a `spawn` took, on some path to here; `None` where control cannot arrive.
type State = Option<HashSet<Id<Local>>>;

/// Reports each use of a `Sender` or `Receiver` after a `spawn` captured it.
pub(super) fn check(program: &Program, interner: &Interner, bodies: &[&Body], sink: &mut DiagnosticSink) {
    let mut reported: HashSet<Span> = HashSet::new();
    for body in bodies {
        let mut flow = Flow { program, interner, body, state: Some(HashSet::new()), frames: Vec::new(), errors: Vec::new() };
        flow.block(&body.block);
        for (span, message) in flow.errors {
            if reported.insert(span) {
                sink.emit(Diagnostic::error(message, span));
            }
        }
    }
}

fn join(a: State, b: State) -> State {
    match (a, b) {
        (Some(mut a), Some(b)) => {
            a.extend(b);
            Some(a)
        }
        (a, None) => a,
        (None, b) => b,
    }
}

/// The states that left a loop by `break` and came back by `continue`.
struct Frame {
    breaks: State,
    continues: State,
}

struct Flow<'a> {
    program: &'a Program,
    interner: &'a Interner,
    body: &'a Body,
    state: State,
    frames: Vec<Frame>,
    errors: Vec<(Span, String)>,
}

impl<'a> Flow<'a> {
    fn tracked(&self, local: Id<Local>) -> bool {
        matches!(self.program.types.kind(self.body.locals[local].ty), TyKind::Builtin { kind: BuiltinTy::Sender | BuiltinTy::Receiver, .. })
    }

    fn use_local(&mut self, local: Id<Local>, span: Span) {
        if self.state.as_ref().is_some_and(|moved| moved.contains(&local)) {
            let name = self.interner.resolve(self.body.locals[local].name);
            let message = format!("`{name}` was moved into a task");
            if !self.errors.iter().any(|(seen, _)| *seen == span) {
                self.errors.push((span, message));
            }
        }
    }

    fn revive(&mut self, pattern: Id<Pattern>) {
        let mut bound = Vec::new();
        pattern_locals(self.body, pattern, &mut bound);
        if let Some(moved) = &mut self.state {
            for local in bound {
                moved.remove(&local);
            }
        }
    }

    fn block(&mut self, block: &Block) {
        for &stmt in &block.stmts {
            self.stmt(stmt);
        }
    }

    /// A closure or task body, which runs on its own flow.
    fn within(&mut self, block: &Block) {
        let state = self.state.replace(HashSet::new());
        let frames = std::mem::take(&mut self.frames);
        self.block(block);
        self.state = state;
        self.frames = frames;
    }

    fn condition(&mut self, condition: &Condition) {
        match condition {
            Condition::Expr(expr) => self.expr(*expr),
            Condition::Let { pattern, value } => {
                self.expr(*value);
                self.revive(*pattern);
            }
        }
    }

    fn stmt(&mut self, id: Id<Stmt>) {
        if self.state.is_none() {
            return;
        }
        let body = self.body;
        match &body.stmts[id].kind {
            StmtKind::Let { pattern, init } => {
                self.expr(*init);
                self.revive(*pattern);
            }
            StmtKind::Assign { target, op, value } => {
                self.expr(*value);
                match (&body.exprs[*target].kind, op) {
                    (ExprKind::Local(local), None) => {
                        if let Some(moved) = &mut self.state {
                            moved.remove(local);
                        }
                    }
                    _ => self.expr(*target),
                }
            }
            StmtKind::If(branch) => self.if_stmt(branch),
            StmtKind::While { condition, body: inner } => self.while_loop(condition, inner),
            StmtKind::For(looped) => {
                self.expr(looped.iterable);
                self.for_loop(looped.pattern, &looped.body);
            }
            StmtKind::Match(matched) => self.matched(matched),
            StmtKind::Scope(block) => self.block(block),
            StmtKind::With { init, body: inner, .. } => {
                self.expr(*init);
                self.block(inner);
            }
            StmtKind::Return(value) => {
                if let Some(value) = value {
                    self.expr(*value);
                }
                self.state = None;
            }
            StmtKind::Break => {
                let state = self.state.take();
                if let Some(frame) = self.frames.last_mut() {
                    frame.breaks = join(frame.breaks.take(), state);
                }
            }
            StmtKind::Continue => {
                let state = self.state.take();
                if let Some(frame) = self.frames.last_mut() {
                    frame.continues = join(frame.continues.take(), state);
                }
            }
            StmtKind::Yield(value) | StmtKind::Expr(value) => self.expr(*value),
            StmtKind::Const { .. }
            | StmtKind::ComptimeIf(_)
            | StmtKind::ComptimeFor { .. }
            | StmtKind::ComptimeBlock(_)
            | StmtKind::Error => {}
        }
    }

    fn if_stmt(&mut self, branch: &IfStmt) {
        self.condition(&branch.condition);
        let after = self.state.clone();
        self.block(&branch.then);
        let then = self.state.take();
        self.state = after;
        match &branch.otherwise {
            Some(Else::Block(block)) => self.block(block),
            Some(Else::If(other)) => self.stmt(*other),
            None => {}
        }
        self.state = join(then, self.state.take());
    }

    fn while_loop(&mut self, condition: &Condition, inner: &Block) {
        let mut head = self.state.clone();
        loop {
            self.state = head.clone();
            self.condition(condition);
            let after = self.state.clone();
            self.frames.push(Frame { breaks: None, continues: None });
            self.block(inner);
            let frame = self.frames.pop().expect("a loop frame");
            let back = join(self.state.take(), frame.continues);
            let next = join(head.clone(), back);
            if next == head {
                self.state = join(after, frame.breaks);
                return;
            }
            head = next;
        }
    }

    fn for_loop(&mut self, pattern: Id<Pattern>, inner: &Block) {
        let mut head = self.state.clone();
        loop {
            self.state = head.clone();
            self.revive(pattern);
            self.frames.push(Frame { breaks: None, continues: None });
            self.block(inner);
            let frame = self.frames.pop().expect("a loop frame");
            let back = join(self.state.take(), frame.continues);
            let next = join(head.clone(), back);
            if next == head {
                self.state = join(head, frame.breaks);
                return;
            }
            head = next;
        }
    }

    fn matched(&mut self, matched: &Match) {
        self.expr(matched.subject);
        let start = self.state.clone();
        let mut out: State = None;
        for arm in &matched.arms {
            self.state = start.clone();
            self.revive(arm.pattern);
            if let Some(guard) = arm.guard {
                self.expr(guard);
            }
            match &arm.body {
                ArmBody::Value(value) => self.expr(*value),
                ArmBody::Block(block) => self.block(block),
            }
            out = join(out, self.state.take());
        }
        self.state = if matched.arms.is_empty() { start } else { out };
    }

    fn expr(&mut self, id: Id<Expr>) {
        if self.state.is_none() {
            return;
        }
        let body = self.body;
        let expr = &body.exprs[id];
        match &expr.kind {
            ExprKind::Local(local) => self.use_local(*local, expr.span),
            ExprKind::If { condition, then, otherwise } => {
                self.condition(condition);
                let after = self.state.clone();
                self.expr(*then);
                let first = self.state.take();
                self.state = after;
                self.expr(*otherwise);
                self.state = join(first, self.state.take());
            }
            ExprKind::Match(matched) => self.matched(matched),
            ExprKind::Closure(closure) => {
                for &captured in &closure.captures {
                    self.use_local(captured, expr.span);
                }
                self.within(&closure.body);
            }
            ExprKind::Spawn { task, .. } => {
                for &captured in &task.captures {
                    self.use_local(captured, expr.span);
                }
                for &captured in &task.captures {
                    if self.tracked(captured)
                        && let Some(moved) = &mut self.state
                    {
                        moved.insert(captured);
                    }
                }
                self.within(&task.body);
            }
            kind => {
                for operand in operands(kind) {
                    self.expr(operand);
                }
            }
        }
    }
}
