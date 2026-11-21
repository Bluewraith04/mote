//! Finds the `std.meta.require` calls of a program that can be decided before it runs.

use std::collections::HashSet;

use mote_base::{Arena, Interner, Span};
use mote_semantics::{DefId, ModuleId, Program, TyId};

use crate::intrinsics;
use crate::ir::{Block, Callee, FuncId, Function, FunctionKind, Inst, Module, Op, Operand, RegId, Terminator};
use crate::uses::{reads, writes};

/// A `require` that does not hold.
#[derive(Clone, Debug, PartialEq)]
pub struct Unmet {
    /// The module the error is reported in: the listing type's, else the function's.
    pub module: ModuleId,
    /// The listing type's name, else the `require` call.
    pub span: Span,
    pub message: String,
    /// The type and the trait it lists, for a call in a default method copied for the listing.
    pub listing: Option<(DefId, DefId)>,
}

impl Unmet {
    /// The error text: the type, the trait and the message for a listing, else the message.
    pub fn text(&self, program: &Program, interner: &Interner) -> String {
        match self.listing {
            Some((owner, listed)) => format!("`{}` cannot list `{}`: {}", interner.resolve(program.defs[owner].name), interner.resolve(program.defs[listed].name), self.message),
            None => self.message.clone(),
        }
    }
}

/// One `require` call to decide: the function it is in, and a function that runs what leads up to the call and then the call, which faults with the message when the condition is false.
pub struct Check {
    function: FuncId,
    span: Span,
    pub thunk: FuncId,
}

impl Check {
    /// The error for a thunk that faulted with the message.
    pub fn unmet(&self, module: &Module, program: &Program, message: String) -> Option<Unmet> {
        let FunctionKind::Instance(instance) = &module.functions[self.function].kind else { return None };
        let def = &program.defs[instance.def];
        Some(match (def.owner, program.listed_defaults.get(&instance.def)) {
            (Some(owner), Some(&listed)) => Unmet { module: program.defs[owner].module, span: program.defs[owner].span, message, listing: Some((owner, listed)) },
            _ => Unmet { module: def.module, span: self.span, message, listing: None },
        })
    }
}

/// A copy of the module without start-up and with a thunk for each `require` call that is in the first block of a function and whose arguments depend on nothing but constants and calls. Any other call is left to fault when it runs.
pub fn checks(module: &Module, interner: &Interner) -> (Module, Vec<Check>) {
    let mut scratch = module.clone();
    scratch.start = None;
    scratch.entries.clear();
    let mut found = Vec::new();
    for (function, body) in module.functions.iter() {
        if !matches!(body.kind, FunctionKind::Instance(_)) {
            continue;
        }
        let entry = &body.blocks[body.entry];
        for (at, inst) in entry.insts.iter().enumerate() {
            let Op::Call { callee: Callee::Intrinsic(name), args, .. } = &inst.op else { continue };
            if interner.resolve(*name) != "require" || args.len() != 2 || !is_closed(&entry.insts[..at], args, interner) {
                continue;
            }
            let thunk = scratch.add(thunk(body, &entry.insts[..=at]));
            found.push(Check { function, span: inst.span, thunk });
        }
    }
    (scratch, found)
}

/// Whether the instructions and the two arguments read only what the instructions before them wrote, and the instructions touch nothing outside the function.
fn is_closed(before: &[Inst], args: &[Operand], interner: &Interner) -> bool {
    let mut written: HashSet<RegId> = HashSet::new();
    for inst in before {
        let allowed = match &inst.op {
            Op::LoadGlobal(_) | Op::StoreGlobal { .. } | Op::Spawn { .. } | Op::ScopeBegin | Op::ScopeEnd(_) | Op::Defer { .. } | Op::Undefer | Op::Yield(_) | Op::FrameLoad(_) | Op::FrameStore { .. } => false,
            Op::Call { callee: Callee::Function(_), .. } => true,
            Op::Call { callee: Callee::Intrinsic(name), .. } => intrinsics::lookup(interner.resolve(*name)).is_some_and(|row| !row.parks()),
            Op::Call { .. } => false,
            _ => true,
        };
        if !allowed || reads(&inst.op).iter().any(|reg| !written.contains(reg)) {
            return false;
        }
        written.extend(writes(inst));
    }
    args.iter().all(|arg| match arg {
        Operand::Reg(reg) => written.contains(reg),
        Operand::Const(_) => true,
    })
}

/// A function with no parameters that runs the instructions.
fn thunk(source: &Function, insts: &[Inst]) -> Function {
    let mut blocks = Arena::new();
    let entry = blocks.alloc(Block { insts: insts.to_vec(), term: Terminator::Return(None) });
    Function { kind: FunctionKind::Start, regs: source.regs.clone(), captures: Vec::new(), params: Vec::new(), out_params: Vec::new(), ret: TyId::UNIT, is_generator: false, frame: Vec::new(), blocks, entry }
}

