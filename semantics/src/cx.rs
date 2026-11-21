use std::collections::{HashMap, HashSet};

use mote_base::{Diagnostic, DiagnosticSink, Id, Interner, Span, Symbol};
use mote_syntax as ast;

use crate::defs::{DefId, ModuleId, Resolved};
use crate::program::Program;
use crate::types::TyId;

/// Whether a declaration's type is still being worked out.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Progress {
    Resolving,
    Done,
}

/// What name resolution and checking share: syntax trees, the program under construction and diagnostics.
pub(crate) struct Cx<'a> {
    pub asts: &'a [ast::Ast],
    pub ids: Vec<ModuleId>,
    pub imports: Vec<HashMap<Id<ast::Item>, usize>>,
    pub prelude: Option<ModuleId>,
    pub program: &'a mut Program,
    pub interner: &'a mut Interner,
    pub sink: &'a mut DiagnosticSink,
    /// Where each name of a module was first bound.
    pub bound_at: Vec<HashMap<Symbol, Span>>,
    /// Imports left out because they close a cycle, by module index.
    pub cut: HashSet<(usize, Id<ast::Item>)>,
    pub aliases: HashMap<DefId, Progress>,
    /// Globals whose initializer is being or has been checked.
    pub globals: HashMap<DefId, Progress>,
    pub from: Symbol,
    pub self_name: Symbol,
    /// The types a copied default method reads its trait's parameters as.
    pub trait_names: HashMap<DefId, Vec<(Symbol, TyId)>>,
}

impl<'a> Cx<'a> {
    pub fn new(asts: &'a [ast::Ast], program: &'a mut Program, interner: &'a mut Interner, sink: &'a mut DiagnosticSink) -> Self {
        let ids: Vec<ModuleId> = program.modules.iter().map(|(id, _)| id).collect();
        let from = interner.intern("from");
        let self_name = interner.intern("Self");
        Self {
            asts,
            prelude: program.prelude,
            imports: vec![HashMap::new(); ids.len()],
            bound_at: vec![HashMap::new(); ids.len()],
            ids,
            program,
            interner,
            sink,
            cut: HashSet::new(),
            aliases: HashMap::new(),
            globals: HashMap::new(),
            from,
            self_name,
            trait_names: HashMap::new(),
        }
    }

    pub fn ast(&self, module: ModuleId) -> &'a ast::Ast {
        let asts: &'a [ast::Ast] = self.asts;
        &asts[module.index()]
    }

    pub fn error(&mut self, span: Span, message: impl Into<String>) {
        self.sink.emit(Diagnostic::error(message, span));
    }

    pub fn text(&self, name: Symbol) -> String {
        self.interner.resolve(name).to_string()
    }

    pub fn module_name(&self, module: ModuleId) -> String {
        let path = &self.program.modules[module].path;
        if path.is_empty() {
            return "the module".to_string();
        }
        path.iter().map(|&name| self.interner.resolve(name)).collect::<Vec<_>>().join(".")
    }

    /// What a name means in a module: its own scope, then the prelude.
    pub fn lookup(&self, module: ModuleId, name: Symbol) -> Option<Resolved> {
        if let Some(&resolved) = self.program.modules[module].scope.get(&name) {
            return Some(resolved);
        }
        let prelude = self.prelude?;
        self.program.modules[prelude].exports.get(&name).copied()
    }

    /// Binds a name in a module's scope and, when `public`, its exports.
    pub fn bind(&mut self, module: ModuleId, name: Symbol, resolved: Resolved, span: Span, public: bool) {
        let index = module.index();
        match self.program.modules[module].scope.get(&name).copied() {
            Some(existing) if existing == resolved => {}
            Some(_) => {
                let first = self.bound_at[index][&name];
                let message = format!("`{}` is defined twice", self.text(name));
                self.sink.emit(Diagnostic::error(message, span).label(first, "first defined here"));
                return;
            }
            None => {
                self.program.modules[module].scope.insert(name, resolved);
                self.bound_at[index].insert(name, span);
            }
        }
        if public {
            self.program.modules[module].exports.insert(name, resolved);
        }
    }
}
