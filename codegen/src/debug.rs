//! The names and source positions of compiled functions, read from the code Cranelift made.

use cranelift_codegen::CompiledCode;
use mote_base::Interner;
use mote_ir::{FuncId, FunctionKind, Module};
use mote_semantics::{DefId, Program};

/// What is known of a compiled function besides its frames.
pub(crate) struct Debug {
    pub name: String,
    pub size: u32,
    /// The offset from the function's start where a position begins, ascending, and the span start there.
    pub rows: Vec<(u32, u32)>,
}

fn def_name(program: &Program, interner: &Interner, def: DefId) -> String {
    let item = &program.defs[def];
    let name = interner.resolve(item.name);
    match item.owner {
        Some(owner) => format!("{}.{name}", interner.resolve(program.defs[owner].name)),
        None => name.to_string(),
    }
}

/// How a function is named in a backtrace.
pub(crate) fn name_of(module: &Module, program: &Program, interner: &Interner, id: FuncId) -> String {
    match &module.functions[id].kind {
        FunctionKind::Instance(instance) | FunctionKind::Native(instance) => def_name(program, interner, instance.def),
        FunctionKind::Closure { parent } => format!("{} closure", name_of(module, program, interner, *parent)),
        FunctionKind::Resume { generator } => name_of(module, program, interner, *generator),
        FunctionKind::Global(def) => format!("{} initializer", def_name(program, interner, *def)),
        FunctionKind::TopLevel(_) => "top level".to_string(),
        FunctionKind::Start => "start".to_string(),
    }
}

/// The size and the positions of a compiled function; a stretch of code with no position joins the one before it.
pub(crate) fn describe(name: String, code: &CompiledCode) -> Debug {
    let mut rows: Vec<(u32, u32)> = Vec::new();
    for found in code.buffer.get_srclocs_sorted() {
        if found.loc.is_default() {
            continue;
        }
        let span = found.loc.bits();
        match rows.last_mut() {
            Some(last) if last.0 == found.start => last.1 = span,
            Some(last) if last.1 == span => {}
            _ => rows.push((found.start, span)),
        }
    }
    Debug { name, size: code.buffer.total_size(), rows }
}
