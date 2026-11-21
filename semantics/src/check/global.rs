use mote_base::Span;
use mote_syntax as ast;
use mote_syntax::{ItemKind, PatternKind, StmtKind};

use super::pattern::PatternCx;
use super::FnCx;
use crate::cx::{Cx, Progress};
use crate::defs::{DefId, DefKind, ModuleId};
use crate::resolve::collect::bound_names;
use crate::resolve::Scope;
use crate::tree::{Block, ExprKind, LocalKind, StmtKind as TreeStmt};
use crate::types::TyId;

impl Cx<'_> {
    /// The type of a global, checking its initializer when no annotation gives it.
    pub(crate) fn global_type(&mut self, def: DefId) -> TyId {
        match self.globals.get(&def) {
            Some(Progress::Done) => return self.global_ty(def),
            Some(Progress::Resolving) => {
                let span = self.program.defs[def].span;
                let message = format!("`{}` depends on itself", self.text(self.program.defs[def].name));
                self.error(span, message);
                return TyId::ERROR;
            }
            None => {}
        }
        self.globals.insert(def, Progress::Resolving);
        self.check_global(def);
        self.globals.insert(def, Progress::Done);
        self.global_ty(def)
    }

    fn global_ty(&self, def: DefId) -> TyId {
        match &self.program.defs[def].kind {
            DefKind::Global(global) => global.ty,
            _ => TyId::ERROR,
        }
    }

    fn check_global(&mut self, def: DefId) {
        let Some((item, None)) = self.item_of(def) else { return };
        let ItemKind::Stmt(stmt) = &item.kind else { return };
        let module = self.program.defs[def].module;
        let ast = self.ast(module);
        let StmtKind::Binding(binding) = &ast[*stmt].kind else { return };
        if !matches!(ast[binding.pattern].kind, PatternKind::Binding(_)) {
            self.check_destructured_global(def, binding, module);
            return;
        }
        let annotated = binding.ty.is_some();
        let scope = Scope { module, params: Vec::new(), const_params: Vec::new(), self_ty: None, bound: Vec::new() };
        let mut f = FnCx::new(module, scope, TyId::UNIT);
        let init = if annotated {
            let ty = self.global_ty(def);
            if let Some(written) = binding.ty {
                let written_span = ast[written].span;
                self.check_type_bounds(&f, ty, written_span);
            }
            self.expr_to(&mut f, binding.init, ty)
        } else {
            self.expr(&mut f, binding.init, None)
        };
        let ty = f.body.exprs[init].ty;
        if !annotated && let DefKind::Global(global) = &mut self.program.defs[def].kind {
            global.ty = ty;
        }
        let span = f.body.exprs[init].span;
        let ret = f.body.stmt(TreeStmt::Return(Some(init)), span);
        f.body.block = Block { stmts: vec![ret], span };
        self.program.global_inits.insert(def, f.body);
    }

    /// `let (a, b) = e` at the top level: one body binds the pattern and assigns each global.
    fn check_destructured_global(&mut self, def: DefId, binding: &ast::Binding, module: ModuleId) {
        let origin = self.program.defs[def].origin;
        let group: Vec<DefId> = self.program.defs.iter().filter(|(_, other)| other.module == module && other.origin == origin && matches!(other.kind, DefKind::Global(_))).map(|(id, _)| id).collect();
        for &member in &group {
            self.globals.insert(member, Progress::Resolving);
        }
        let scope = Scope { module, params: Vec::new(), const_params: Vec::new(), self_ty: None, bound: Vec::new() };
        let mut f = FnCx::new(module, scope, TyId::UNIT);
        let init = match binding.ty {
            Some(ty) => {
                let ty = self.written_type(&mut f, ty);
                self.expr_to(&mut f, binding.init, ty)
            }
            None => self.expr(&mut f, binding.init, None),
        };
        let ty = f.body.exprs[init].ty;
        let span = f.body.exprs[init].span;
        let mut pcx = PatternCx::new(binding.kind == ast::BindingKind::Var, LocalKind::Let);
        let pattern = self.pattern(&mut f, &mut pcx, binding.pattern, ty);
        if !self.irrefutable(&f, pattern) {
            let pattern_span = f.body.patterns[pattern].span;
            self.error(pattern_span, "this pattern can fail; use `if let` or `match`");
        }
        let mut stmts = vec![f.body.stmt(TreeStmt::Let { pattern, init }, span)];
        for &(name, local) in pcx.names() {
            let Some(&global) = group.iter().find(|&&member| self.program.defs[member].name == name) else { continue };
            let local_ty = f.body.locals[local].ty;
            if let DefKind::Global(item) = &mut self.program.defs[global].kind {
                item.ty = local_ty;
            }
            let target = f.body.expr(ExprKind::Global(global), local_ty, span);
            let value = f.body.expr(ExprKind::Local(local), local_ty, span);
            stmts.push(f.body.stmt(TreeStmt::Assign { target, op: None, value }, span));
        }
        f.body.block = Block { stmts, span };
        for &member in &group {
            self.globals.insert(member, Progress::Done);
        }
        self.program.global_inits.insert(group[0], f.body);
    }

    /// Checks the statements at the top of each module that define no global: not a `let` or `var`, or one binding no name.
    pub(crate) fn check_top_level(&mut self) {
        let modules: Vec<ModuleId> = self.program.modules.iter().map(|(id, _)| id).collect();
        for module in modules {
            let ast = self.ast(module);
            let stmts: Vec<_> = ast
                .root
                .iter()
                .filter_map(|&item| match &ast[item].kind {
                    ItemKind::Stmt(stmt) => match &ast[*stmt].kind {
                        StmtKind::Binding(binding) => {
                            let mut names = Vec::new();
                            bound_names(ast, binding.pattern, &mut names);
                            names.is_empty().then_some(*stmt)
                        }
                        _ => Some(*stmt),
                    },
                    _ => None,
                })
                .collect();
            if stmts.is_empty() {
                continue;
            }
            let scope = Scope { module, params: Vec::new(), const_params: Vec::new(), self_ty: None, bound: Vec::new() };
            let mut f = FnCx::new(module, scope, TyId::UNIT);
            f.top_level = true;
            let mut checked = Vec::new();
            for stmt in stmts {
                let stmt = self.stmt(&mut f, stmt);
                for (local, ty) in self.after_facts(&f, stmt) {
                    f.narrow(local, ty);
                }
                checked.push(stmt);
            }
            f.body.block = Block { stmts: checked, span: Span::default() };
            self.program.top_level.insert(module, f.body);
        }
    }
}
