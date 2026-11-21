//! Lowers the typed bodies of a program into the register CFG.

mod array;
mod call;
mod closure;
mod data;
mod expr;
mod flow;
mod pattern;
mod place;
mod stmt;
mod task;
#[cfg(test)]
mod closure_tests;
#[cfg(test)]
mod data_tests;
#[cfg(test)]
mod flow_tests;
#[cfg(test)]
mod pattern_tests;
#[cfg(test)]
mod start_tests;
#[cfg(test)]
mod std_tests;
#[cfg(test)]
mod task_tests;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use mote_base::{Arena, Id, Interner, Span};
use mote_semantics::{Body, BuiltinTy, DefId, DefKind, ExprKind, Init, Instance, Local, PatternKind, Program, StmtKind, TyId, TyKind};

use crate::builder::FunctionBuilder;
use crate::ir::{BlockId, Callee, Const, Entry, EntryKind, FuncId, Function, FunctionKind, Module, Op, Operand, RegId, Terminator};

/// A construct the checker admits and lowering has no form for; the pipeline reports it as an internal error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsupported {
    pub function: FuncId,
    pub span: Span,
    pub what: &'static str,
}

pub struct Lowered {
    pub module: Module,
    pub unsupported: Vec<Unsupported>,
}

/// What every body being lowered shares.
struct Shared<'a> {
    program: &'a mut Program,
    interner: &'a mut Interner,
    instances: &'a HashMap<Instance, FuncId>,
    functions: &'a mut Arena<Function>,
    /// The function that stands for a native or intrinsic used as a value, made once.
    wrappers: &'a mut HashMap<Instance, FuncId>,
    unsupported: &'a mut Vec<Unsupported>,
}

/// Lowers every instance that has a body; a generator becomes a function that allocates its frame and a resume function.
pub fn lower(program: &mut Program, interner: &mut Interner) -> Lowered {
    let mut lowered = lower_bodies(program, interner);
    crate::generators::lower_generators(&mut lowered.module, program, interner);
    lowered
}

/// [`lower`] with each generator still one function that holds `Yield`.
pub(crate) fn lower_bodies(program: &mut Program, interner: &mut Interner) -> Lowered {
    let mut module = Module::new();
    let instances = program.order.clone();
    let mut returns = Vec::new();
    for instance in &instances {
        let ret = return_type(program, instance);
        returns.push(ret);
        module.add(placeholder(FunctionKind::Instance(instance.clone()), ret));
    }
    let mut unsupported = Vec::new();
    let mut wrappers = HashMap::new();
    let Module { functions, instances: ids, .. } = &mut module;
    for (instance, ret) in instances.iter().zip(returns) {
        let func = ids[instance];
        let Some(body) = program.bodies.remove(instance) else { continue };
        let shared = Shared { program, interner, instances: ids, functions: &mut *functions, wrappers: &mut wrappers, unsupported: &mut unsupported };
        let function = Lowering::new(shared, func, FunctionKind::Instance(instance.clone()), ret, &body).run();
        program.bodies.insert(instance.clone(), body);
        functions[func] = function;
    }
    let steps = start_steps(program, functions);
    for step in &steps {
        let body = match step.init {
            Init::Global(def) => program.global_inits.remove(&def),
            Init::TopLevel(module) => program.top_level.remove(&module),
        };
        let Some(body) = body else { continue };
        let shared = Shared { program, interner, instances: ids, functions: &mut *functions, wrappers: &mut wrappers, unsupported: &mut unsupported };
        let function = Lowering::new(shared, step.func, step.kind.clone(), step.ret, &body).run();
        match step.init {
            Init::Global(def) => program.global_inits.insert(def, body),
            Init::TopLevel(module) => program.top_level.insert(module, body),
        };
        functions[step.func] = function;
    }
    let start = start_function(functions, &steps);
    module.start = Some(start);
    module.entries = entries(program, interner, &instances, &module);
    Lowered { module, unsupported }
}

/// One start-up step: a global's initializer or a module's top-level statements.
struct Step {
    init: Init,
    func: FuncId,
    kind: FunctionKind,
    ret: TyId,
    /// The global that gets the value the step returns; none for a step that stores its own.
    stores: Option<DefId>,
}

/// A global's initializer ends in `return value`; a destructured `let` stores each name itself.
fn returns_value(body: &Body) -> bool {
    body.block.stmts.last().is_some_and(|&stmt| matches!(body.stmts[stmt].kind, StmtKind::Return(Some(_))))
}

/// A function for each start-up step with a body, in the order they run.
fn start_steps(program: &Program, functions: &mut Arena<Function>) -> Vec<Step> {
    let mut steps = Vec::new();
    for &init in &program.init {
        let (kind, ret, stores) = match init {
            Init::Global(def) => {
                let Some(body) = program.global_inits.get(&def) else { continue };
                let ty = match &program.defs[def].kind {
                    DefKind::Global(global) => global.ty,
                    _ => TyId::ERROR,
                };
                let stores = returns_value(body).then_some(def);
                (FunctionKind::Global(def), if stores.is_some() { ty } else { TyId::UNIT }, stores)
            }
            Init::TopLevel(id) => {
                if !program.top_level.contains_key(&id) {
                    continue;
                }
                (FunctionKind::TopLevel(id), TyId::UNIT, None)
            }
        };
        let func = functions.alloc(placeholder(kind.clone(), ret));
        steps.push(Step { init, func, kind, ret, stores });
    }
    steps
}

/// The function that runs every step, storing the globals the steps return.
fn start_function(functions: &mut Arena<Function>, steps: &[Step]) -> FuncId {
    let span = Span::default();
    let mut b = FunctionBuilder::new(FunctionKind::Start, TyId::UNIT);
    for step in steps {
        let call = Op::Call { callee: Callee::Function(step.func), args: Vec::new(), outs: Vec::new() };
        match step.stores {
            Some(def) => {
                let value = b.value(step.ret, call, span);
                b.emit(None, Op::StoreGlobal { def, value }, span);
            }
            None => b.emit(None, call, span),
        }
    }
    b.terminate(Terminator::Return(None));
    functions.alloc(b.finish().expect("a single closed block"))
}

/// `main` (the first plain function of that name outside std) and every `test` block, in the order the program needed them.
fn entries(program: &Program, interner: &Interner, instances: &[Instance], module: &Module) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut has_main = false;
    for instance in instances {
        let def = &program.defs[instance.def];
        let Some(&func) = module.instances.get(instance) else { continue };
        match &def.kind {
            DefKind::Test => entries.push(Entry { kind: EntryKind::Test(def.name), func }),
            DefKind::Function(_) if !has_main && def.owner.is_none() && instance.args.is_empty() && interner.resolve(def.name) == "main" && !program.modules[def.module].is_std => {
                has_main = true;
                entries.push(Entry { kind: EntryKind::Main, func });
            }
            _ => {}
        }
    }
    entries
}

/// A function that only ends, to hold an id until its body is lowered.
fn placeholder(kind: FunctionKind, ret: TyId) -> Function {
    let mut builder = FunctionBuilder::new(kind, ret);
    builder.terminate(Terminator::Unreachable);
    builder.finish().expect("a single closed block")
}

/// The `let` a closure is bound to when the closure captures that same local, as a nested `fn` that calls itself does.
fn self_capture(body: &Body, pattern: Id<mote_semantics::Pattern>, init: Id<mote_semantics::Expr>) -> Option<Id<Local>> {
    let PatternKind::Bind(local) = body.patterns[pattern].kind else { return None };
    match &body.exprs[init].kind {
        ExprKind::Closure(closure) if closure.captures.contains(&local) => Some(local),
        _ => None,
    }
}

/// The locals that live in a cell: every `var` a closure captures and every nested `fn` that captures itself.
fn celled_locals(body: &Body) -> HashSet<Id<Local>> {
    let mut cells = HashSet::new();
    for (_, expr) in body.exprs.iter() {
        if let ExprKind::Closure(closure) = &expr.kind {
            cells.extend(closure.captures.iter().copied().filter(|&local| body.locals[local].is_var));
        }
    }
    for (_, stmt) in body.stmts.iter() {
        if let StmtKind::Let { pattern, init } = &stmt.kind {
            cells.extend(self_capture(body, *pattern, *init));
        }
    }
    cells
}

fn return_type(program: &mut Program, instance: &Instance) -> TyId {
    if let Some(&ret) = program.instance_returns.get(instance) {
        return ret;
    }
    let declared = match program.defs[instance.def].function() {
        Some(function) => function.ret,
        None => return TyId::UNIT,
    };
    program.types.subst_generic(declared, &instance.args)
}

/// One body being lowered.
struct Lowering<'a> {
    program: &'a mut Program,
    interner: &'a mut Interner,
    instances: &'a HashMap<Instance, FuncId>,
    body: &'a Body,
    func: FuncId,
    ret: TyId,
    b: FunctionBuilder,
    functions: &'a mut Arena<Function>,
    wrappers: &'a mut HashMap<Instance, FuncId>,
    regs: HashMap<Id<Local>, RegId>,
    /// The locals that live in a cell.
    cells: HashSet<Id<Local>>,
    /// The `var` parameters that live in a cell, as (parameter, cell); the parameter gets the cell's value back on return.
    cell_params: Vec<(RegId, RegId)>,
    /// The (break, continue, cleanups open at the loop) of the open loops, innermost last.
    loops: Vec<(BlockId, BlockId, usize)>,
    /// What every exit from the code being lowered must undo, innermost last.
    cleanups: Vec<Cleanup>,
    /// The tokens of the open `scope`s, innermost last.
    scopes: Vec<Operand>,
    /// Whether the function is a `spawn` body whose result holds arrays, which `return` detaches.
    detach_result: bool,
    unsupported: &'a mut Vec<Unsupported>,
}

/// Work done on every way out of a `scope` or a `with`.
#[derive(Clone)]
enum Cleanup {
    /// Waits for the tasks of the scope.
    Scope(Operand),
    /// Calls `close` on the resource.
    Close { resource: Operand, func: FuncId },
}

impl<'a> Lowering<'a> {
    fn new(shared: Shared<'a>, func: FuncId, kind: FunctionKind, ret: TyId, body: &'a Body) -> Self {
        let Shared { program, interner, instances, functions, wrappers, unsupported } = shared;
        let mut b = FunctionBuilder::new(kind, ret);
        if body.is_generator {
            b.set_generator();
        }
        let cells = celled_locals(body);
        Self { program, interner, instances, body, func, ret, b, functions, wrappers, regs: HashMap::new(), cells, cell_params: Vec::new(), loops: Vec::new(), cleanups: Vec::new(), scopes: Vec::new(), detach_result: false, unsupported }
    }

    fn run(mut self) -> Function {
        let body = self.body;
        self.enter(&body.params);
        self.block(&body.block);
        self.end_body(body.is_generator)
    }

    /// Declares the parameters; one that a closure captures moves into a cell.
    fn enter(&mut self, params: &[Id<Local>]) {
        for &param in params {
            let local = &self.body.locals[param];
            let reg = self.b.param(local.ty, Some(local.name), local.is_var);
            if self.cells.contains(&param) {
                let cell = self.b.cell(local.ty, Some(local.name));
                self.b.emit(Some(cell), Op::NewCell(Operand::Reg(reg)), local.span);
                self.regs.insert(param, cell);
                if local.is_var {
                    self.cell_params.push((reg, cell));
                }
            } else {
                self.regs.insert(param, reg);
            }
        }
    }

    /// Ends a body that falls off its last statement and builds the function; a generator then just ends.
    fn end_body(&mut self, returns_unit: bool) -> Function {
        if !self.b.is_terminated() {
            let term = if self.ret == TyId::UNIT || returns_unit { Terminator::Return(None) } else { Terminator::Unreachable };
            self.emit_cell_params(Span::default());
            self.b.terminate(term);
        }
        let b = std::mem::replace(&mut self.b, FunctionBuilder::new(FunctionKind::Start, TyId::UNIT));
        b.finish().unwrap_or_else(|open| panic!("b{} of f{} has no terminator", open.index(), self.func.index()))
    }

    /// Copies each captured `var` parameter back out of its cell before the function returns.
    fn emit_cell_params(&mut self, span: Span) {
        for (param, cell) in self.cell_params.clone() {
            self.b.emit(Some(param), Op::LoadCell(Operand::Reg(cell)), span);
        }
    }

    /// Records a construct with no form and stands a `null` in for its value.
    fn unsupported(&mut self, span: Span, what: &'static str) -> Operand {
        self.unsupported.push(Unsupported { function: self.func, span, what });
        Operand::Const(Const::Null)
    }

    /// Whether the type is a `Map` or a `Set`, whose indexing and literals run std code.
    fn is_collection(&self, ty: TyId) -> bool {
        matches!(self.program.types.kind(ty), TyKind::Adt { def, .. } if self.program.std.map == Some(*def) || self.program.std.set == Some(*def))
    }

    /// Whether writing through a value of the type changes what other holders see.
    fn is_reference(&self, ty: TyId) -> bool {
        match self.program.types.kind(ty) {
            TyKind::Array(_) | TyKind::Builtin { kind: BuiltinTy::List, .. } => true,
            TyKind::Adt { def, .. } => {
                if self.program.std.map == Some(*def) || self.program.std.set == Some(*def) {
                    return true;
                }
                matches!(&self.program.defs[*def].kind, DefKind::Record(record) if record.is_class)
            }
            _ => false,
        }
    }
}
