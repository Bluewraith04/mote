mod flow;
#[cfg(test)]
mod tests;
mod typed;

use mote_base::Interner;
use mote_semantics::Program;

use crate::ir::{BlockId, Callee, FuncId, Function, FunctionKind, Inst, Module, Op, Operand, RegId, Terminator};

/// A malformed function or call found by [`verify`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyError {
    pub function: FuncId,
    pub message: String,
}

/// Checks that every block, register and function a function names exists, and that each op, call and terminator fits the types of its registers.
pub fn verify(module: &Module, program: &Program, interner: &Interner) -> Vec<VerifyError> {
    let mut errors = Vec::new();
    for (id, function) in module.functions.iter() {
        let mut check = Check { module, program, interner, function, id, errors: &mut errors, inst: None };
        check.run();
    }
    errors
}

struct Check<'a> {
    module: &'a Module,
    program: &'a Program,
    interner: &'a Interner,
    function: &'a Function,
    id: FuncId,
    errors: &'a mut Vec<VerifyError>,
    /// The instruction being checked, named in the messages of its type errors.
    inst: Option<&'a Inst>,
}

impl<'a> Check<'a> {
    fn fail(&mut self, message: String) {
        self.errors.push(VerifyError { function: self.id, message });
    }

    fn run(&mut self) {
        if self.function.blocks.get(self.function.entry).is_none() {
            self.fail("entry block is missing".to_string());
        }
        for &reg in self.function.params.iter().chain(&self.function.captures) {
            self.reg(reg);
        }
        for &position in &self.function.out_params {
            if position >= self.function.params.len() {
                self.fail(format!("out parameter {position} is not a parameter"));
            }
        }
        self.typed_function();
        let function: &'a Function = self.function;
        for (_, block) in function.blocks.iter() {
            for inst in &block.insts {
                if let Some(dest) = inst.dest {
                    self.reg(dest);
                }
                self.op(&inst.op);
                self.inst = Some(inst);
                self.typed_inst(inst);
                self.inst = None;
            }
            self.term(&block.term);
            self.typed_term(&block.term);
        }
        self.flow();
    }

    fn reg(&mut self, reg: RegId) {
        if self.function.regs.get(reg).is_none() {
            self.fail(format!("r{} does not exist", reg.index()));
        }
    }

    fn operand(&mut self, operand: &Operand) {
        if let Operand::Reg(reg) = operand {
            self.reg(*reg);
        }
    }

    fn operands<'o>(&mut self, operands: impl IntoIterator<Item = &'o Operand>) {
        for operand in operands {
            self.operand(operand);
        }
    }

    fn block(&mut self, block: BlockId) {
        if self.function.blocks.get(block).is_none() {
            self.fail(format!("b{} does not exist", block.index()));
        }
    }

    fn func(&mut self, func: FuncId) -> Option<&Function> {
        let found = self.module.functions.get(func);
        if found.is_none() {
            self.fail(format!("f{} does not exist", func.index()));
        }
        found
    }

    fn op(&mut self, op: &Op) {
        match op {
            Op::Copy(a) | Op::Len(a) | Op::VariantTag(a) | Op::Cast(a) | Op::Show(a) | Op::NewCell(a) | Op::LoadCell(a) | Op::IsSome(a) | Op::SomePayload(a) | Op::Unwrap(a) | Op::ScopeEnd(a) | Op::Yield(a) | Op::Detach(a) => self.operand(a),
            Op::Unary { operand, .. } | Op::Convert { operand, .. } | Op::IsType { operand, .. } | Op::UnionPayload { operand, .. } => self.operand(operand),
            Op::Binary { left, right, .. } => self.operands([left, right]),
            Op::Concat(items) | Op::Tuple(items) | Op::List(items) | Op::Array(items) | Op::Record(items) => self.operands(items),
            Op::Variant { fields, .. } => self.operands(fields),
            Op::ArrayRepeat { value, len } => self.operands([value, len]),
            Op::Range { start, end, .. } => self.operands([start, end]),
            Op::Field { object, .. } | Op::TupleField { object, .. } | Op::IsVariant { object, .. } | Op::VariantField { object, .. } => self.operand(object),
            Op::SetField { object, value, .. } => self.operands([object, value]),
            Op::Index { object, index } => self.operands([object, index]),
            Op::SetIndex { object, index, value } => self.operands([object, index, value]),
            Op::Slice { object, start, end, step, .. } => self.operands([object].into_iter().chain(start).chain(end).chain(step)),
            Op::SetSlice { object, start, end, value, .. } => self.operands([object, value].into_iter().chain(start).chain(end)),
            Op::StoreCell { cell, value } => self.operands([cell, value]),
            Op::LoadGlobal(_) | Op::ScopeBegin | Op::Undefer => {}
            Op::FrameLoad(slot) | Op::FrameStore { slot, .. } => {
                if let Op::FrameStore { value, .. } = op {
                    self.operand(value);
                }
                if *slot as usize >= self.function.frame.len() {
                    self.fail(format!("frame slot {slot} does not exist"));
                }
            }
            Op::Stream { resume, init } => {
                self.operands(init);
                let Some(target) = self.func(*resume) else { return };
                let slots = target.frame.len();
                if matches!(target.kind, FunctionKind::Resume { .. }) && init.len() + 1 > slots {
                    self.fail(format!("f{} has {slots} frame slots, found {} initial values", resume.index(), init.len()));
                }
            }
            Op::Defer { resource, func } => {
                self.operand(resource);
                let params = self.func(*func).map(|target| target.params.len());
                if let Some(count) = params.filter(|&count| count != 1) {
                    self.fail(format!("f{} closes {count} values, not 1", func.index()));
                }
            }
            Op::StoreGlobal { value, .. } => self.operand(value),
            Op::Call { callee, args, outs } => {
                self.operands(args);
                for &out in outs {
                    self.reg(out);
                }
                match callee {
                    Callee::Function(func) => {
                        let Some(target) = self.func(*func) else { return };
                        let (params, out_params) = (target.params.len(), target.out_params.len());
                        if args.len() != params {
                            self.fail(format!("f{} takes {params} arguments, found {}", func.index(), args.len()));
                        }
                        if outs.len() != out_params {
                            self.fail(format!("f{} copies back {out_params} values, found {}", func.index(), outs.len()));
                        }
                    }
                    Callee::Value(operand) => self.operand(operand),
                    Callee::Intrinsic(_) | Callee::Native(_) => {}
                }
            }
            Op::Closure { func, captures } => self.carrier(*func, captures),
            Op::Spawn { func, captures, scope } => {
                self.carrier(*func, captures);
                if let Some(scope) = scope {
                    self.operand(scope);
                }
            }
        }
    }

    fn carrier(&mut self, func: FuncId, captures: &[Operand]) {
        self.operands(captures);
        let Some(target) = self.func(func) else { return };
        let count = target.captures.len();
        if captures.len() != count {
            self.fail(format!("f{} carries {count} captures, found {}", func.index(), captures.len()));
        }
    }

    fn term(&mut self, term: &Terminator) {
        match term {
            Terminator::Branch { condition, .. } => self.operand(condition),
            Terminator::Switch { value, .. } => self.operand(value),
            Terminator::Return(Some(value)) => self.operand(value),
            Terminator::Jump(_) | Terminator::Return(None) | Terminator::Unreachable => {}
        }
        for target in term.targets() {
            self.block(target);
        }
    }
}
