//! Collects each module's names and resolves signatures into typed definitions.

mod attrs;
pub(crate) mod collect;
mod imports;
mod lower;
mod signatures;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use mote_base::{DiagnosticSink, Id, Interner, Symbol};
use mote_syntax as ast;

use crate::cx::Cx;
use crate::defs::Module;
use crate::program::Program;

pub use attrs::KNOWN;
pub(crate) use lower::Scope;

/// One parsed source file and where its imports lead.
pub struct ModuleInput {
    pub path: Vec<Symbol>,
    pub ast: ast::Ast,
    pub is_std: bool,
    /// The module each import item names, as an index into [`Input::modules`].
    pub imports: HashMap<Id<ast::Item>, usize>,
    /// The source text and the offset of its first byte in the source map, which doc comments point into.
    pub source: String,
    pub source_start: u32,
}

pub struct Input {
    pub modules: Vec<ModuleInput>,
    /// The module whose exports every module sees without an import.
    pub prelude: Option<usize>,
}

/// Builds the program's modules, definitions and signatures.
pub fn resolve(input: Input, interner: &mut Interner, sink: &mut DiagnosticSink) -> Program {
    let mut program = Program::new();
    let mut ids = Vec::new();
    let mut asts = Vec::new();
    let mut imports = Vec::new();
    for module in input.modules {
        ids.push(program.modules.alloc(Module { path: module.path, ast: ast::Ast::default(), is_std: module.is_std, scope: HashMap::new(), exports: HashMap::new(), source: module.source, source_start: module.source_start }));
        asts.push(module.ast);
        imports.push(module.imports);
    }
    program.prelude = input.prelude.map(|index| ids[index]);
    {
        let mut cx = Cx::new(&asts, &mut program, interner, sink);
        cx.imports = imports;
        cx.collect();
        cx.imports();
        cx.signatures();
    }
    for (id, ast) in ids.into_iter().zip(asts) {
        program.modules[id].ast = ast;
    }
    program
}
