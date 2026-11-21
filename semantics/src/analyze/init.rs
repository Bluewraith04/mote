use std::collections::{BTreeSet, HashMap, HashSet};

use mote_base::{Diagnostic, DiagnosticSink, Id, Interner};

use crate::defs::{DefId, DefKind, ModuleId, Resolved};
use crate::program::{Init, Instance, Program};
use crate::tree::{Body, Callee, Expr, ExprKind, StmtKind};
use crate::walk::reach;

/// The start-up steps: modules after their imports, a module's globals after the globals they read, then its statements.
pub(super) fn order(program: &Program, interner: &Interner, sink: &mut DiagnosticSink) -> Vec<Init> {
    let leaders: Vec<DefId> = program.defs.iter().map(|(id, _)| id).filter(|def| program.global_inits.contains_key(def)).collect();
    let leader_of = group_leaders(program, &leaders);
    let reads: HashMap<DefId, BTreeSet<DefId>> = leaders.iter().map(|&leader| (leader, global_reads(program, leader, &leader_of))).collect();
    let modules = module_order(program);
    let position: HashMap<ModuleId, usize> = modules.iter().enumerate().map(|(index, &module)| (module, index)).collect();
    let mut init = Vec::new();
    for &module in &modules {
        let units: Vec<DefId> = leaders.iter().copied().filter(|&leader| program.defs[leader].module == module).collect();
        let mut topo = Topo { program, interner, sink: &mut *sink, reads: &reads, position: &position, state: HashMap::new(), stack: Vec::new(), done: Vec::new() };
        for unit in units {
            topo.visit(unit);
        }
        init.extend(topo.done.into_iter().map(Init::Global));
        if program.top_level.contains_key(&module) {
            init.push(Init::TopLevel(module));
        }
    }
    init
}

/// The global whose body sets each global: itself, or the first name of its destructured `let`.
fn group_leaders(program: &Program, leaders: &[DefId]) -> HashMap<DefId, DefId> {
    let by_item: HashMap<_, _> = leaders.iter().map(|&leader| ((program.defs[leader].module, program.defs[leader].origin.map(|origin| origin.item)), leader)).collect();
    program
        .defs
        .iter()
        .filter(|(_, def)| matches!(def.kind, DefKind::Global(_)))
        .filter_map(|(id, def)| by_item.get(&(def.module, def.origin.map(|origin| origin.item))).map(|&leader| (id, leader)))
        .collect()
}

/// The globals, by leader, that the initializer reads directly or through the functions it reaches.
fn global_reads(program: &Program, leader: DefId, leader_of: &HashMap<DefId, DefId>) -> BTreeSet<DefId> {
    let own: HashSet<DefId> = leader_of.iter().filter(|&(_, &found)| found == leader).map(|(&member, _)| member).collect();
    let mut found = BTreeSet::new();
    let mut seen = HashSet::new();
    collect(program, &program.global_inits[&leader], Some(&own), leader_of, &mut seen, &mut found);
    found
}

fn collect(program: &Program, body: &Body, own: Option<&HashSet<DefId>>, leader_of: &HashMap<DefId, DefId>, seen: &mut HashSet<Instance>, found: &mut BTreeSet<DefId>) {
    let reached = reach(body);
    let mut skipped: HashSet<Id<Expr>> = HashSet::new();
    if let Some(own) = own {
        for &stmt in &reached.stmts {
            if let StmtKind::Assign { target, .. } = &body.stmts[stmt].kind
                && matches!(&body.exprs[*target].kind, ExprKind::Global(def) if own.contains(def))
            {
                skipped.insert(*target);
            }
        }
    }
    for id in reached.exprs {
        match &body.exprs[id].kind {
            ExprKind::Global(def) if !skipped.contains(&id) => {
                if let Some(&leader) = leader_of.get(def) {
                    found.insert(leader);
                }
            }
            ExprKind::Function(function) | ExprKind::Call { callee: Callee::Function(function), .. } => {
                if seen.insert(function.clone())
                    && let Some(next) = program.bodies.get(function)
                {
                    collect(program, next, None, leader_of, seen, found);
                }
            }
            _ => {}
        }
    }
}

/// Every module after the modules it names, the prelude first of all.
fn module_order(program: &Program) -> Vec<ModuleId> {
    let all: Vec<ModuleId> = program.modules.iter().map(|(id, _)| id).collect();
    let mut order = Vec::new();
    let mut seen = HashSet::new();
    for &module in &all {
        visit_module(program, module, &mut seen, &mut order);
    }
    order
}

fn visit_module(program: &Program, module: ModuleId, seen: &mut HashSet<ModuleId>, order: &mut Vec<ModuleId>) {
    if !seen.insert(module) {
        return;
    }
    let mut deps: BTreeSet<ModuleId> = program.modules[module]
        .scope
        .values()
        .map(|resolved| match resolved {
            Resolved::Module(other) => *other,
            Resolved::Def(def) => program.defs[*def].module,
        })
        .collect();
    deps.extend(program.prelude);
    deps.remove(&module);
    for dep in deps {
        visit_module(program, dep, seen, order);
    }
    order.push(module);
}

/// Depth-first ordering of one module's globals by what they read.
struct Topo<'a> {
    program: &'a Program,
    interner: &'a Interner,
    sink: &'a mut DiagnosticSink,
    reads: &'a HashMap<DefId, BTreeSet<DefId>>,
    position: &'a HashMap<ModuleId, usize>,
    /// 1 while on the stack, 2 once placed.
    state: HashMap<DefId, u8>,
    stack: Vec<DefId>,
    done: Vec<DefId>,
}

impl Topo<'_> {
    fn name(&self, def: DefId) -> String {
        self.interner.resolve(self.program.defs[def].name).to_string()
    }

    fn visit(&mut self, unit: DefId) {
        if self.state.contains_key(&unit) {
            return;
        }
        self.state.insert(unit, 1);
        self.stack.push(unit);
        let module = self.program.defs[unit].module;
        for &dep in &self.reads[&unit] {
            let other = self.program.defs[dep].module;
            if other != module {
                if self.position[&other] > self.position[&module] {
                    let message = format!("`{}` is read before its module is initialized", self.name(dep));
                    self.sink.emit(Diagnostic::error(message, self.program.defs[unit].span));
                }
                continue;
            }
            match self.state.get(&dep) {
                Some(1) => self.cycle(dep),
                Some(_) => {}
                None => self.visit(dep),
            }
        }
        self.stack.pop();
        self.state.insert(unit, 2);
        self.done.push(unit);
    }

    fn cycle(&mut self, start: DefId) {
        let from = self.stack.iter().position(|&def| def == start).expect("a global on the stack");
        let mut names: Vec<String> = self.stack[from..].iter().map(|&def| format!("`{}`", self.name(def))).collect();
        names.push(format!("`{}`", self.name(start)));
        let message = if names.len() == 2 { format!("{} depends on itself", names[0]) } else { format!("initialization cycle: {}", names.join(" -> ")) };
        self.sink.emit(Diagnostic::error(message, self.program.defs[start].span));
    }
}
