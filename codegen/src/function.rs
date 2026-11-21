//! Lowers one IR function to Cranelift.

use std::collections::{HashMap, HashSet};

use cranelift_codegen::entity::EntityRef;
use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{self, types, AbiParam, InstBuilder, MemFlags, StackSlot, TrapCode, Type, Value};
use cranelift_frontend::{FunctionBuilder, Switch, Variable};
use cranelift_module::{DataId, FuncId as ClifFunc, Module as ClifModule};
use mote_abi::{fault_site_raw as raw, FaultKind, FaultTable, LinkTable, CONTEXT_POLL, CONTEXT_STACK_LIMIT};
use mote_base::{Interner, Span, Symbol};
use mote_ir::{Callee, Const, Function, FunctionKind, Op, Operand, RegId, Terminator};
use mote_ir::{FuncId, Inst, Module};
use mote_semantics::{BinaryOp, BuiltinTy, DefId, DefKind, FloatTy, Program, TyId, TyKind, UnaryOp};

use mote_runtime::helpers::{shown, PRINT_ERROR, PRINT_NEWLINE};
use crate::rows::{emitter, Emitter};
use crate::scalar::Num;
use crate::typetable::TypeMap;
use crate::values::{Addr, Repr, Temp};
use crate::CompileError;

/// What every function of a program is lowered against.
#[derive(Clone, Copy)]
pub(crate) struct Env<'e> {
    pub module: &'e Module,
    pub program: &'e Program,
    pub interner: &'e Interner,
    pub types: &'e TypeMap,
    /// The position of `Ok` among the variants of `Result`.
    pub ok_tag: Option<u32>,
    /// The module the code and data are defined in.
    pub clif: &'e dyn ClifModule,
    pub funcs: &'e HashMap<FuncId, ClifFunc>,
    pub globals: &'e HashMap<DefId, DataId>,
    /// The static string object of each text constant.
    pub strings: &'e HashMap<Symbol, DataId>,
    /// The static string object of the name of each type a `typeof` call names.
    pub type_names: &'e HashMap<TyId, DataId>,
    /// The types an `Any` can hold, for the name of the one it holds.
    pub any_members: &'e Vec<TyId>,
    /// The position in `link` of each extern function the program calls.
    pub natives: &'e HashMap<DefId, usize>,
    /// The libraries and functions the loader opens before `main`.
    pub link: &'e LinkTable,
    /// The words the loader fills with the address of each linked function.
    pub native_slots: Option<DataId>,
    /// The function that calls each blocking extern fn on behalf of a helper thread.
    pub thunks: &'e HashMap<usize, ClifFunc>,
    /// The function that runs the closure of each task type and stores its result in the handle.
    pub task_thunks: &'e HashMap<mote_abi::TypeId, ClifFunc>,
    /// The task types a `task_view` makes, whose handles may hold a converter for their result.
    pub viewed_tasks: &'e HashSet<mote_abi::TypeId>,
    pub defer_thunks: &'e HashMap<FuncId, ClifFunc>,
    /// The functions that are called through a closure, which take the closure after the result slot.
    pub closure_conv: &'e HashSet<FuncId>,
    /// The static closure object of each function in `closure_conv` that captures nothing.
    pub statics: &'e HashMap<FuncId, DataId>,
    pub imports: Imports,
}

/// The runtime entries and helpers compiled code calls.
#[derive(Clone, Copy)]
pub(crate) struct Imports {
    pub fault: ClifFunc,
    pub alloc: ClifFunc,
    pub alloc_array: ClifFunc,
    pub print: ClifFunc,
    pub show: ClifFunc,
    pub compare: ClifFunc,
    pub cut: ClifFunc,
    pub math: ClifFunc,
    pub text: ClifFunc,
    pub system: ClifFunc,
    pub value: ClifFunc,
    pub native: ClifFunc,
    pub fault_message: ClifFunc,
    pub poll: ClifFunc,
    pub fmod: ClifFunc,
    pub mem: ClifFunc,
    pub task_spawn: ClifFunc,
    pub task_await: ClifFunc,
    pub task_cancel: ClifFunc,
    pub task_ready: ClifFunc,
    pub task_message: ClifFunc,
    pub scope_begin: ClifFunc,
    pub scope_end: ClifFunc,
    pub channel_new: ClifFunc,
    pub channel_end: ClifFunc,
    pub channel_send: ClifFunc,
    pub channel_recv: ClifFunc,
    pub sender_close: ClifFunc,
    pub sender_clone: ClifFunc,
    pub receiver_close: ClifFunc,
    pub receiver_clone: ClifFunc,
    pub receiver_closed: ClifFunc,
    pub mutex_new: ClifFunc,
    pub mutex_take: ClifFunc,
    pub mutex_put: ClifFunc,
    pub once_new: ClifFunc,
    pub once_begin: ClifFunc,
    pub once_finish: ClifFunc,
    pub once_value: ClifFunc,
    pub sleep: ClifFunc,
    pub task_await_timeout: ClifFunc,
    pub channel_recv_timeout: ClifFunc,
    pub port_new: ClifFunc,
    pub port_ref: ClifFunc,
    pub defer: ClifFunc,
    pub undefer: ClifFunc,
    pub scope_end_error: ClifFunc,
    pub native_blocking: ClifFunc,
}

/// The user trap code of a block that cannot be reached.
const UNREACHABLE: TrapCode = TrapCode::unwrap_user(1);

/// The class of a register: nothing, or a number.
pub(crate) fn class(program: &Program, ty: TyId) -> Result<Option<Num>, CompileError> {
    Ok(match program.types.kind(ty) {
        TyKind::Unit => None,
        TyKind::Bool => Some(Num::Bool),
        TyKind::Int(int) => Some(Num::Int { bits: int.bits(), signed: int.signed() }),
        TyKind::Float(FloatTy::F32) => Some(Num::F32),
        TyKind::Float(FloatTy::F64) => Some(Num::F64),
        _ => return Err(CompileError::internal("a value that is not a number or Bool")),
    })
}

/// The type a constant has when no register says otherwise.
pub(crate) fn const_ty(constant: &Const) -> TyId {
    match constant {
        Const::Int(value) if i64::try_from(*value).is_ok() => TyId::INT,
        Const::Int(_) => TyId::UINT64,
        Const::Float(_) => TyId::FLOAT64,
        Const::Bool(_) => TyId::BOOL,
        Const::Null => TyId::UNIT,
        Const::String(_) => TyId::STRING,
    }
}

/// The name of an op's variant, such as `Concat`.
pub(crate) fn op_name(op: &Op) -> String {
    let text = format!("{op:?}");
    text.split(['(', ' ', '{']).next().unwrap_or_default().to_string()
}

pub(crate) struct Lowering<'e, 's, 'b> {
    pub(crate) b: FunctionBuilder<'b>,
    pub(crate) env: Env<'e>,
    pub(crate) faults: &'s mut FaultTable,
    pub(crate) function: &'e Function,
    pub(crate) func: FuncId,
    pub(crate) context: Value,
    pub(crate) vars: HashMap<RegId, Variable>,
    /// The slot of each register that holds a value in memory.
    pub(crate) slots: HashMap<RegId, StackSlot>,
    /// The slot that mirrors each register that holds a reference.
    ref_slots: HashMap<RegId, StackSlot>,
    pub(crate) temps: Vec<Temp>,
    pub(crate) blocks: HashMap<mote_ir::BlockId, ir::Block>,
    /// Where the caller wants a result that is passed in memory.
    pub(crate) result_addr: Option<Value>,
    /// Where the caller wants the final values of the `var` parameters.
    pub(crate) out_area: Option<Value>,
    /// The closure object the function was called through; a `Resume` function keeps its frame in it.
    pub(crate) closure: Option<Value>,
    /// The first slot of the frame, a zero word that every call site lists as a reference so that each has a stack map.
    probe: Option<StackSlot>,
}

/// What the collector needs to know about the frame of a lowered function.
pub(crate) struct Frame {
    /// The registers that hold a value in memory, by slot and type; the collector always scans them.
    pub memory: Vec<(StackSlot, mote_abi::TypeId)>,
    /// The slots that mirror the registers that hold a reference.
    pub words: Vec<StackSlot>,
}

impl<'e, 's, 'b> Lowering<'e, 's, 'b> {
    /// Lowers the function into the builder's function.
    pub fn lower(builder: FunctionBuilder<'b>, env: Env<'e>, faults: &'s mut FaultTable, id: FuncId) -> Result<Frame, CompileError> {
        let function = &env.module.functions[id];
        let mut lowering = Lowering { b: builder, env, faults, function, func: id, context: Value::from_u32(0), vars: HashMap::new(), slots: HashMap::new(), ref_slots: HashMap::new(), temps: Vec::new(), blocks: HashMap::new(), result_addr: None, out_area: None, closure: None, probe: None };
        lowering.run()?;
        lowering.b.seal_all_blocks();
        lowering.list_the_probe_at_each_call();
        let mut memory: Vec<_> = lowering.slots.iter().map(|(&reg, &slot)| (slot, lowering.reg_type(reg))).collect();
        memory.sort_by_key(|(slot, _)| slot.as_u32());
        let mut words: Vec<_> = lowering.ref_slots.values().copied().collect();
        words.sort_by_key(|slot| slot.as_u32());
        lowering.b.finalize();
        Ok(Frame { memory, words })
    }

    /// Adds the probe to the stack map of every call, which gives each call site a record even when no reference is live across it.
    fn list_the_probe_at_each_call(&mut self) {
        let Some(slot) = self.probe else { return };
        let layout = &self.b.func.layout;
        let calls: Vec<ir::Inst> = layout.blocks().flat_map(|block| layout.block_insts(block)).filter(|&inst| self.b.func.dfg.insts[inst].opcode().is_safepoint()).collect();
        for inst in calls {
            self.b.func.dfg.append_user_stack_map_entry(inst, ir::UserStackMapEntry { ty: types::I64, slot, offset: 0 });
        }
    }

    fn run(&mut self) -> Result<(), CompileError> {
        let function = self.function;
        let start = self.b.create_block();
        self.b.append_block_params_for_function_params(start);
        self.b.switch_to_block(start);
        self.context = self.b.block_params(start)[0];
        let probe = self.b.create_sized_stack_slot(ir::StackSlotData::new(ir::StackSlotKind::ExplicitSlot, 8, 3));
        self.probe = Some(probe);
        let at = self.slot_addr(probe);
        self.zero_mem(at, 8);
        let mut frame = 0;
        for (reg_id, reg) in function.regs.iter() {
            match if reg.cell { Repr::Num(Num::Ref) } else { self.repr(reg.ty)? } {
                Repr::Nothing => {}
                Repr::Num(num) => {
                    let var = Variable::new(self.vars.len());
                    self.b.declare_var(var, num.clif());
                    self.vars.insert(reg_id, var);
                    let zero = self.zero(num);
                    self.b.def_var(var, zero);
                    if num == Num::Ref {
                        let slot = self.b.create_sized_stack_slot(ir::StackSlotData::new(ir::StackSlotKind::ExplicitSlot, 8, 3));
                        let at = self.slot_addr(slot);
                        self.zero_mem(at, 8);
                        self.ref_slots.insert(reg_id, slot);
                    }
                }
                Repr::Mem(size) => {
                    let slot = self.new_slot(size);
                    self.slots.insert(reg_id, slot);
                    let at = self.slot_addr(slot);
                    self.zero_mem(at, size);
                    frame += size;
                }
            }
        }
        self.bind_params(start)?;
        self.stack_check(frame);
        self.poll();
        for (block_id, _) in function.blocks.iter() {
            let block = self.b.create_block();
            self.blocks.insert(block_id, block);
        }
        let headers = loop_headers(function);
        let entry = self.blocks[&function.entry];
        self.b.ins().jump(entry, &[]);
        let order: Vec<_> = std::iter::once(function.entry).chain(function.blocks.iter().map(|(block, _)| block).filter(|&block| block != function.entry)).collect();
        for block_id in order {
            self.b.switch_to_block(self.blocks[&block_id]);
            if headers.contains(&block_id) {
                self.poll();
            }
            let block = &function.blocks[block_id];
            for inst in &block.insts {
                self.b.set_srcloc(ir::SourceLoc::new(inst.span.start));
                self.release_temps();
                if !self.is_quiet(inst) {
                    self.poll();
                }
                self.inst(inst)?;
            }
            self.release_temps();
            self.terminator(&block.term)?;
        }
        Ok(())
    }

    fn zero(&mut self, num: Num) -> Value {
        match num {
            Num::F32 => self.b.ins().f32const(0.0),
            Num::F64 => self.b.ins().f64const(0.0),
            other => self.b.ins().iconst(other.clif(), 0),
        }
    }

    /// Faults when the stack pointer less the bytes the frame holds in slots is below the limit the context holds.
    fn stack_check(&mut self, frame: u32) {
        let span = match &self.function.kind {
            FunctionKind::Instance(instance) => self.env.program.defs[instance.def].span,
            _ => Span::default(),
        };
        let sp = self.b.ins().get_stack_pointer(types::I64);
        let sp = if frame == 0 { sp } else { self.b.ins().iadd_imm(sp, -i64::from(frame)) };
        let limit = self.b.ins().load(types::I64, MemFlags::trusted(), self.context, CONTEXT_STACK_LIMIT as i32);
        let below = self.b.ins().icmp(IntCC::UnsignedLessThan, sp, limit);
        self.fault_if(below, FaultKind::StackDepth, span);
    }

    /// Whether the instruction never allocates and never calls a function that might, so a collection asked for can wait.
    fn is_quiet(&self, inst: &Inst) -> bool {
        let number = |ty: TyId| class(self.env.program, ty).is_ok();
        let dest_is_number = inst.dest.is_none_or(|dest| number(self.function.regs[dest].ty));
        match &inst.op {
            Op::Copy(_) | Op::Unary { .. } | Op::Cast(_) | Op::Len(_) | Op::Field { .. } | Op::TupleField { .. } | Op::IsSome(_) | Op::SomePayload(_) | Op::Unwrap(_) | Op::VariantTag(_) | Op::IsVariant { .. } | Op::VariantField { .. } | Op::IsType { .. } | Op::UnionPayload { .. } | Op::LoadCell(_) | Op::LoadGlobal(_) => true,
            Op::Binary { .. } => dest_is_number,
            Op::Index { object, .. } => matches!(self.env.program.types.kind(self.ty_of(object)), TyKind::Array(_) | TyKind::FixedArray(..) | TyKind::Builtin { kind: BuiltinTy::List, .. } | TyKind::Adt { .. }),
            Op::SetField { value, .. } | Op::SetIndex { value, .. } | Op::StoreCell { value, .. } | Op::StoreGlobal { value, .. } => number(self.ty_of(value)),
            Op::Call { callee: Callee::Function(_) | Callee::Value(_), .. } => true,
            Op::Call { callee: Callee::Intrinsic(name), .. } => {
                let name = self.env.interner.resolve(*name);
                matches!(emitter(name), Some(Emitter::Numeric | Emitter::Print | Emitter::Mem)) || matches!(name, "list_len" | "array_len" | "list_get" | "array_get" | "string_len")
            }
            _ => false,
        }
    }

    /// Enters the runtime when the task's poll word is not zero: a stop of the world or a request of the scheduler.
    fn poll(&mut self) {
        let word = self.b.ins().load(types::I64, MemFlags::trusted(), self.context, CONTEXT_POLL as i32);
        let wait = self.b.create_block();
        let go = self.b.create_block();
        self.b.set_cold_block(wait);
        self.b.ins().brif(word, wait, &[], go, &[]);
        self.b.switch_to_block(wait);
        let entry = self.func_ref(self.env.imports.poll);
        self.b.ins().call(entry, &[self.context]);
        self.b.ins().jump(go, &[]);
        self.b.switch_to_block(go);
    }

    /// Continues in a new block unless `cond` is not zero, which ends the program with a fault of the kind.
    pub(crate) fn fault_if(&mut self, cond: Value, kind: FaultKind, span: Span) {
        let fault = self.b.create_block();
        let go = self.b.create_block();
        self.b.set_cold_block(fault);
        self.b.ins().brif(cond, fault, &[], go, &[]);
        self.b.switch_to_block(fault);
        let site = raw(self.faults.add(kind, span));
        let site = self.b.ins().iconst(types::I32, i64::from(site));
        let entry = self.func_ref(self.env.imports.fault);
        self.b.ins().call(entry, &[self.context, site]);
        self.b.ins().trap(UNREACHABLE);
        self.b.switch_to_block(go);
    }

    /// A reference to the function inside the function being built.
    pub(crate) fn func_ref(&mut self, id: ClifFunc) -> ir::FuncRef {
        let decl = self.env.clif.declarations().get_function_decl(id);
        let signature = self.b.func.import_signature(decl.signature.clone());
        let name = self.b.func.declare_imported_user_function(ir::UserExternalName { namespace: 0, index: id.as_u32() });
        self.b.func.import_function(ir::ExtFuncData { name: ir::ExternalName::user(name), signature, colocated: decl.linkage.is_final() })
    }

    /// The type of a register, or of a constant read alone.
    pub(crate) fn ty_of(&self, operand: &Operand) -> TyId {
        match operand {
            Operand::Reg(reg) => self.function.regs[*reg].ty,
            Operand::Const(constant) => const_ty(constant),
        }
    }

    /// The type of the operands of an op: the type of the first that is a register, else of the first constant.
    pub(crate) fn operand_ty(&self, operands: &[&Operand]) -> TyId {
        operands.iter().find_map(|operand| if let Operand::Reg(reg) = operand { Some(self.function.regs[*reg].ty) } else { None }).unwrap_or_else(|| self.ty_of(operands[0]))
    }

    /// The value of an operand read as the type, or nothing for a value with no room.
    pub(crate) fn operand(&mut self, operand: &Operand, ty: TyId) -> Result<Option<Value>, CompileError> {
        match self.repr(ty)? {
            Repr::Nothing => Ok(None),
            Repr::Num(num) => Ok(Some(self.num_operand(operand, num)?)),
            Repr::Mem(_) => Err(CompileError::internal("a value in memory here")),
        }
    }

    /// The address of the static string object of a text constant.
    fn string_address(&mut self, text: Symbol) -> Result<Value, CompileError> {
        let data = *self.env.strings.get(&text).ok_or_else(|| CompileError::internal("a text constant with no object"))?;
        let value = self.env.clif.declare_data_in_func(data, self.b.func);
        Ok(self.b.ins().symbol_value(types::I64, value))
    }

    pub(crate) fn constant(&mut self, constant: Const, num: Num) -> Result<Value, CompileError> {
        Ok(match (constant, num) {
            (Const::Null, Num::Ref) => self.b.ins().iconst(types::I64, 0),
            (Const::String(text), Num::Ref) => self.string_address(text)?,
            (Const::Bool(value), Num::Int { bits: 8, .. }) => self.b.ins().iconst(types::I8, i64::from(value)),
            (Const::Int(value), Num::Int { bits, .. }) => self.b.ins().iconst(num.clif(), sign_extended(value, bits)),
            (Const::Int(value), Num::F64) => self.b.ins().f64const(value as f64),
            (Const::Int(value), Num::F32) => self.b.ins().f32const(value as f32),
            (Const::Float(value), Num::F64) => self.b.ins().f64const(value),
            (Const::Float(value), Num::F32) => self.b.ins().f32const(value as f32),
            (Const::Bool(value), Num::Bool) => self.b.ins().iconst(types::I8, i64::from(value)),
            _ => return Err(CompileError::internal("this constant")),
        })
    }

    pub(crate) fn set(&mut self, dest: Option<RegId>, value: Option<Value>) {
        if let (Some(reg), Some(value)) = (dest, value) {
            self.define(reg, value);
        }
    }

    /// Gives a register that holds a number its new value; a reference is also written to the register's slot, which the collector scans.
    pub(crate) fn define(&mut self, reg: RegId, value: Value) {
        let Some(&var) = self.vars.get(&reg) else { return };
        self.b.def_var(var, value);
        if let Some(&slot) = self.ref_slots.get(&reg) {
            self.b.ins().stack_store(value, slot, 0);
        }
    }

    fn dest_ty(&self, dest: Option<RegId>) -> Option<TyId> {
        dest.map(|reg| self.function.regs[reg].ty)
    }

    fn inst(&mut self, inst: &Inst) -> Result<(), CompileError> {
        let dest = inst.dest;
        let dest_ty = self.dest_ty(dest);
        match &inst.op {
            Op::Copy(operand) => {
                let dest = dest.ok_or_else(|| CompileError::internal("a copy with no destination"))?;
                self.copy_into(dest, operand)?;
            }
            Op::Unary { op, operand } => {
                let ty = match (operand, dest_ty) {
                    (Operand::Const(_), Some(ty)) => ty,
                    _ => self.ty_of(operand),
                };
                let num = class(self.env.program, ty)?.ok_or_else(|| CompileError::internal("a unary operation on Null"))?;
                let value = self.operand(operand, ty)?.expect("a number has a value");
                let result = self.unary(*op, value, num, inst.span)?;
                self.set(dest, Some(result));
            }
            Op::Binary { op, left, right } => {
                if let Some(result) = self.null_compare(*op, left, right)? {
                    self.set(dest, Some(result));
                    return Ok(());
                }
                if let Some(result) = self.optional_compare(*op, left, right)? {
                    self.set(dest, Some(result));
                    return Ok(());
                }
                if let Some(result) = self.reference_binary(*op, left, right)? {
                    self.set(dest, Some(result));
                    return Ok(());
                }
                if let Some(result) = self.value_equality(*op, left, right)? {
                    self.set(dest, Some(result));
                    return Ok(());
                }
                let compares = matches!(op, BinaryOp::Eq | BinaryOp::NotEq | BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq);
                if matches!(op, BinaryOp::Eq | BinaryOp::NotEq) && self.operand_ty(&[left, right]) == TyId::UNIT {
                    let result = self.b.ins().iconst(types::I8, i64::from(*op == BinaryOp::Eq));
                    self.set(dest, Some(result));
                    return Ok(());
                }
                let ty = match (left, right, dest_ty) {
                    (Operand::Const(_), Operand::Const(_), Some(ty)) if !compares => ty,
                    _ => self.operand_ty(&[left, right]),
                };
                let num = class(self.env.program, ty)?.ok_or_else(|| CompileError::internal("a binary operation on Null"))?;
                let a = self.operand(left, ty)?.expect("a number has a value");
                let b = self.operand(right, ty)?.expect("a number has a value");
                let result = self.binary(*op, a, b, num, inst.span)?;
                self.set(dest, Some(result));
            }
            Op::Cast(operand) => {
                let to = dest_ty.ok_or_else(|| CompileError::internal("a cast with no destination"))?;
                let from_ty = self.ty_of(operand);
                if let (TyKind::Adt { def, .. }, Some(target)) = (self.env.program.types.kind(from_ty), class(self.env.program, to)?)
                    && let Some(item) = self.env.program.defs[*def].enum_def()
                {
                    let constants = item.variants.iter().map(|variant| variant.discriminant).collect::<Option<Vec<_>>>().ok_or_else(|| CompileError::internal("a cast of an enum without constants"))?;
                    let result = self.enum_constant(operand, &constants, target)?;
                    self.set(dest, Some(result));
                    return Ok(());
                }
                let (Some(from), Some(target)) = (class(self.env.program, from_ty)?, class(self.env.program, to)?) else { return Err(CompileError::internal("a cast of Null")) };
                let value = self.operand(operand, from_ty)?.expect("a number has a value");
                let result = self.cast(value, from, target)?;
                self.set(dest, Some(result));
            }
            Op::Call { callee, args, outs } => self.call(inst, callee, args, outs)?,
            Op::Closure { func, captures } => self.closure(inst, *func, captures)?,
            Op::Stream { resume, init } => self.new_stream(inst, *resume, init)?,
            Op::FrameLoad(slot) => self.frame_load(inst, *slot)?,
            Op::FrameStore { slot, value } => self.frame_store(*slot, value)?,
            Op::Spawn { func, captures, scope } => self.spawn(inst, *func, captures, scope.as_ref())?,
            Op::ScopeBegin => self.scope_begin(inst)?,
            Op::ScopeEnd(token) => self.scope_end(inst, token)?,
            Op::Defer { resource, func } => self.defer(resource, *func)?,
            Op::Undefer => self.undefer(),
            Op::NewCell(value) => self.new_cell(inst, value)?,
            Op::LoadCell(cell) => self.load_cell(inst, cell)?,
            Op::StoreCell { cell, value } => self.store_cell(cell, value)?,
            Op::LoadGlobal(def) => {
                let (address, id) = self.global(*def)?;
                self.read_into(dest, Addr { base: address, offset: 0 }, id)?;
            }
            Op::StoreGlobal { def, value } => {
                let (address, id) = self.global(*def)?;
                self.write(Addr { base: address, offset: 0 }, id, value)?;
            }
            _ => self.value_op(inst)?,
        }
        Ok(())
    }

    /// The address of a global's cell and the table's type of its value.
    fn global(&mut self, def: DefId) -> Result<(Value, mote_abi::TypeId), CompileError> {
        let DefKind::Global(global) = &self.env.program.defs[def].kind else { return Err(CompileError::internal("a global that is not a global")) };
        let id = self.env.types.id(global.ty);
        let data = *self.env.globals.get(&def).ok_or_else(|| CompileError::internal("a global with no cell"))?;
        let value = self.env.clif.declare_data_in_func(data, self.b.func);
        let address = self.b.ins().symbol_value(types::I64, value);
        Ok((address, id))
    }

    fn call(&mut self, inst: &Inst, callee: &Callee, args: &[Operand], outs: &[RegId]) -> Result<(), CompileError> {
        match callee {
            Callee::Function(target) => self.call_function(inst, *target, args, outs),
            Callee::Value(closure) => self.call_value(inst, closure, args, outs),
            Callee::Native(def) => self.native_call(inst, *def, args, outs),
            Callee::Intrinsic(name) => {
                let name = self.env.interner.resolve(*name).to_string();
                self.intrinsic(&name, inst, args, outs)
            }
        }
    }

    fn intrinsic(&mut self, name: &str, inst: &Inst, args: &[Operand], outs: &[RegId]) -> Result<(), CompileError> {
        if !outs.is_empty() && !matches!(name, "list_push" | "list_pop" | "list_set" | "list_clear" | "list_set_slice" | "array_set" | "stream_next") {
            return Err(CompileError::internal(format!("the intrinsic `{name}` with a `var` argument")));
        }
        match emitter(name) {
            Some(Emitter::List) => self.list_intrinsic(name, inst, args, outs),
            Some(Emitter::Numeric) => self.numeric_intrinsic(name, inst, args),
            Some(Emitter::Text) => self.text_intrinsic(name, inst, args),
            Some(Emitter::Collect) => self.collection_intrinsic(name, inst, args, outs),
            Some(Emitter::Print) => self.print_intrinsic(name, args),
            Some(Emitter::System) => self.system_intrinsic(name, inst, args),
            Some(Emitter::Task) => self.task_intrinsic(name, inst, args),
            Some(Emitter::Channel) => self.channel_intrinsic(name, inst, args),
            Some(Emitter::Lock) => self.lock_intrinsic(name, inst, args),
            Some(Emitter::Atomic) => self.atomic_intrinsic(name, inst, args),
            Some(Emitter::Stream) => self.stream_intrinsic(name, inst, args, outs),
            Some(Emitter::Mem) => self.mem_intrinsic(name, inst, args),
            None => Err(CompileError::internal(format!("the intrinsic `{name}`"))),
        }
    }

    fn print_intrinsic(&mut self, name: &str, args: &[Operand]) -> Result<(), CompileError> {
        let [arg] = args else { return Err(CompileError::internal("print with other than one argument")) };
        let flags = u32::from(name.ends_with("println")) * PRINT_NEWLINE as u32 + u32::from(name.starts_with('e')) * PRINT_ERROR as u32;
        if !self.is_plain(arg) {
            return self.print_value(arg, flags);
        }
        let (bits, kind) = self.shown_bits(arg)?;
        let kind = self.b.ins().iconst(types::I32, i64::from(kind));
        let flags = self.b.ins().iconst(types::I32, i64::from(flags));
        let print = self.func_ref(self.env.imports.print);
        self.b.ins().call(print, &[self.context, bits, kind, flags]);
        Ok(())
    }

    /// A number, `Bool` or string operand as the 64 bits and the kind that `mote_print` and `mote_show` read.
    pub(crate) fn shown_bits(&mut self, arg: &Operand) -> Result<(Value, u32), CompileError> {
        let ty = match arg {
            Operand::Reg(reg) => self.function.regs[*reg].ty,
            Operand::Const(Const::String(_)) => TyId::STRING,
            Operand::Const(Const::Int(_) | Const::Float(_) | Const::Bool(_)) => self.ty_of(arg),
            Operand::Const(Const::Null) => return Err(CompileError::internal("print of Null")),
        };
        if self.env.program.types.kind(ty) == &TyKind::String {
            let reference = self.num_operand(arg, Num::Ref)?;
            return Ok((reference, shown::STRING));
        }
        let num = class(self.env.program, ty)?.ok_or_else(|| CompileError::internal("print of Null"))?;
        let value = self.operand(arg, ty)?.expect("a number has a value");
        Ok(match num {
            Num::Int { bits, signed } => {
                let wide = if bits == 64 { value } else if signed { self.b.ins().sextend(types::I64, value) } else { self.b.ins().uextend(types::I64, value) };
                (wide, if signed { shown::SIGNED } else { shown::UNSIGNED })
            }
            Num::Bool => (self.b.ins().uextend(types::I64, value), shown::BOOL),
            Num::F64 => (self.b.ins().bitcast(types::I64, MemFlags::new(), value), shown::FLOAT),
            Num::F32 => {
                let wide = self.b.ins().fpromote(types::F64, value);
                (self.b.ins().bitcast(types::I64, MemFlags::new(), wide), shown::FLOAT32)
            }
            Num::Ref => return Err(CompileError::internal("print of an object")),
        })
    }

    fn terminator(&mut self, term: &Terminator) -> Result<(), CompileError> {
        match term {
            Terminator::Jump(target) => {
                let block = self.blocks[target];
                self.b.ins().jump(block, &[]);
            }
            Terminator::Branch { condition, then, otherwise } => {
                let value = self.operand(condition, TyId::BOOL)?.expect("a condition has a value");
                let (then, otherwise) = (self.blocks[then], self.blocks[otherwise]);
                self.b.ins().brif(value, then, &[], otherwise, &[]);
            }
            Terminator::Switch { value, cases, default } => {
                let ty = self.operand_ty(&[value]);
                let num = class(self.env.program, ty)?.ok_or_else(|| CompileError::internal("a switch on Null"))?;
                let scrutinee = self.operand(value, ty)?.expect("a number has a value");
                let bits = match num {
                    Num::Int { bits, .. } => bits,
                    Num::Bool => 8,
                    _ => return Err(CompileError::internal("a switch on a Float")),
                };
                let mut switch = Switch::new();
                for (case, target) in cases {
                    let pattern = (*case as u128) & if bits == 128 { u128::MAX } else { (1u128 << bits) - 1 };
                    switch.set_entry(pattern, self.blocks[target]);
                }
                switch.emit(&mut self.b, scrutinee, self.blocks[default]);
            }
            Terminator::Return(value) => self.return_value(value.as_ref())?,
            Terminator::Unreachable => {
                self.b.ins().trap(UNREACHABLE);
            }
        }
        Ok(())
    }

    fn unary(&mut self, op: UnaryOp, value: Value, num: Num, span: Span) -> Result<Value, CompileError> {
        Ok(match (op, num) {
            (UnaryOp::Not, Num::Bool) => self.b.ins().bxor_imm(value, 1),
            (UnaryOp::Neg, Num::F32 | Num::F64) => self.b.ins().fneg(value),
            (UnaryOp::BitNot, Num::Int { .. }) => self.b.ins().bnot(value),
            (UnaryOp::Neg, Num::Int { bits, signed }) => self.negate(value, bits, signed, span),
            _ => return Err(CompileError::internal("this unary operation")),
        })
    }

    fn negate(&mut self, value: Value, bits: u32, signed: bool, span: Span) -> Value {
        if !signed {
            let nonzero = self.b.ins().icmp_imm(IntCC::NotEqual, value, 0);
            self.fault_if(nonzero, FaultKind::IntegerOverflow, span);
            return value;
        }
        let ty = Num::Int { bits, signed }.clif();
        let zero = self.b.ins().iconst(ty, 0);
        self.arithmetic(BinaryOp::Sub, zero, value, bits, true, span)
    }

    fn binary(&mut self, op: BinaryOp, a: Value, b: Value, num: Num, span: Span) -> Result<Value, CompileError> {
        use BinaryOp::*;
        Ok(match num {
            Num::Bool => match op {
                And | BitAnd => self.b.ins().band(a, b),
                Or | BitOr => self.b.ins().bor(a, b),
                BitXor => self.b.ins().bxor(a, b),
                Eq => self.b.ins().icmp(IntCC::Equal, a, b),
                NotEq => self.b.ins().icmp(IntCC::NotEqual, a, b),
                _ => return Err(CompileError::internal("this Bool operation")),
            },
            Num::F32 | Num::F64 => match op {
                Add => self.b.ins().fadd(a, b),
                Sub => self.b.ins().fsub(a, b),
                Mul => self.b.ins().fmul(a, b),
                Div => self.b.ins().fdiv(a, b),
                Rem => self.float_rem(a, b, num),
                Eq => self.b.ins().fcmp(FloatCC::Equal, a, b),
                NotEq => self.b.ins().fcmp(FloatCC::NotEqual, a, b),
                Lt => self.b.ins().fcmp(FloatCC::LessThan, a, b),
                LtEq => self.b.ins().fcmp(FloatCC::LessThanOrEqual, a, b),
                Gt => self.b.ins().fcmp(FloatCC::GreaterThan, a, b),
                GtEq => self.b.ins().fcmp(FloatCC::GreaterThanOrEqual, a, b),
                _ => return Err(CompileError::internal("this Float operation")),
            },
            Num::Int { bits, signed } => match op {
                Add | Sub | Mul | Div | Rem => self.arithmetic(op, a, b, bits, signed, span),
                BitAnd => self.b.ins().band(a, b),
                BitOr => self.b.ins().bor(a, b),
                BitXor => self.b.ins().bxor(a, b),
                Shl | Shr => self.shift(op, a, b, bits, signed, span),
                Eq => self.b.ins().icmp(IntCC::Equal, a, b),
                NotEq => self.b.ins().icmp(IntCC::NotEqual, a, b),
                Lt => self.b.ins().icmp(if signed { IntCC::SignedLessThan } else { IntCC::UnsignedLessThan }, a, b),
                LtEq => self.b.ins().icmp(if signed { IntCC::SignedLessThanOrEqual } else { IntCC::UnsignedLessThanOrEqual }, a, b),
                Gt => self.b.ins().icmp(if signed { IntCC::SignedGreaterThan } else { IntCC::UnsignedGreaterThan }, a, b),
                GtEq => self.b.ins().icmp(if signed { IntCC::SignedGreaterThanOrEqual } else { IntCC::UnsignedGreaterThanOrEqual }, a, b),
                _ => return Err(CompileError::internal("this integer operation")),
            },
            Num::Ref => match op {
                Eq => self.b.ins().icmp(IntCC::Equal, a, b),
                NotEq => self.b.ins().icmp(IntCC::NotEqual, a, b),
                _ => return Err(CompileError::internal("this reference operation")),
            },
        })
    }

    /// The remainder of two floats, computed as `Float` and cut back for a `Float32`.
    fn float_rem(&mut self, a: Value, b: Value, num: Num) -> Value {
        let (a, b) = if num == Num::F32 { (self.b.ins().fpromote(types::F64, a), self.b.ins().fpromote(types::F64, b)) } else { (a, b) };
        let fmod = self.func_ref(self.env.imports.fmod);
        let call = self.b.ins().call(fmod, &[a, b]);
        let result = self.b.inst_results(call)[0];
        if num == Num::F32 { self.b.ins().fdemote(types::F32, result) } else { result }
    }

    /// `+ - * / %` on integers, faulting on overflow and division by zero.
    fn arithmetic(&mut self, op: BinaryOp, a: Value, b: Value, bits: u32, signed: bool, span: Span) -> Value {
        use BinaryOp::*;
        let ty = Num::Int { bits, signed }.clif();
        if matches!(op, Div | Rem) {
            let zero = self.b.ins().icmp_imm(IntCC::Equal, b, 0);
            self.fault_if(zero, FaultKind::DivideByZero, span);
        }
        if bits == 64 {
            let flagged = match (op, signed) {
                (Add, true) => Some(self.b.ins().sadd_overflow(a, b)),
                (Add, false) => Some(self.b.ins().uadd_overflow(a, b)),
                (Sub, true) => Some(self.b.ins().ssub_overflow(a, b)),
                (Sub, false) => Some(self.b.ins().usub_overflow(a, b)),
                (Mul, true) => Some(self.b.ins().smul_overflow(a, b)),
                (Mul, false) => Some(self.b.ins().umul_overflow(a, b)),
                _ => None,
            };
            if let Some((result, flag)) = flagged {
                self.fault_if(flag, FaultKind::IntegerOverflow, span);
                return result;
            }
            return match (op, signed) {
                (Div, true) => {
                    let least = self.b.ins().icmp_imm(IntCC::Equal, a, i64::MIN);
                    let minus_one = self.b.ins().icmp_imm(IntCC::Equal, b, -1);
                    let both = self.b.ins().band(least, minus_one);
                    self.fault_if(both, FaultKind::IntegerOverflow, span);
                    self.b.ins().sdiv(a, b)
                }
                (Div, false) => self.b.ins().udiv(a, b),
                (_, true) => self.b.ins().srem(a, b),
                (_, false) => self.b.ins().urem(a, b),
            };
        }
        let wide = |lowering: &mut Self, value: Value| if signed { lowering.b.ins().sextend(types::I64, value) } else { lowering.b.ins().uextend(types::I64, value) };
        let (x, y) = (wide(self, a), wide(self, b));
        let result = match (op, signed) {
            (Add, _) => self.b.ins().iadd(x, y),
            (Sub, _) => self.b.ins().isub(x, y),
            (Mul, _) => self.b.ins().imul(x, y),
            (Div, true) => self.b.ins().sdiv(x, y),
            (Div, false) => self.b.ins().udiv(x, y),
            (_, true) => self.b.ins().srem(x, y),
            (_, false) => self.b.ins().urem(x, y),
        };
        let narrow = self.b.ins().ireduce(ty, result);
        let back = wide(self, narrow);
        let lost = self.b.ins().icmp(IntCC::NotEqual, back, result);
        self.fault_if(lost, FaultKind::IntegerOverflow, span);
        narrow
    }

    fn shift(&mut self, op: BinaryOp, a: Value, b: Value, bits: u32, signed: bool, span: Span) -> Value {
        let out = self.b.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, b, i64::from(bits));
        self.fault_if(out, FaultKind::ShiftOutOfRange, span);
        match (op, signed) {
            (BinaryOp::Shl, _) => self.b.ins().ishl(a, b),
            (_, true) => self.b.ins().sshr(a, b),
            (_, false) => self.b.ins().ushr(a, b),
        }
    }

    /// `value as T` between numbers: integers wrap, floats cut toward zero and saturate with `NaN` as 0.
    pub(crate) fn cast(&mut self, value: Value, from: Num, to: Num) -> Result<Value, CompileError> {
        Ok(match (from, to) {
            (Num::Int { bits: from_bits, signed }, Num::Int { bits: to_bits, .. }) => {
                let target = to.clif();
                if to_bits < from_bits {
                    self.b.ins().ireduce(target, value)
                } else if to_bits == from_bits {
                    value
                } else if signed {
                    self.b.ins().sextend(target, value)
                } else {
                    self.b.ins().uextend(target, value)
                }
            }
            (Num::Bool, Num::Int { bits, .. }) => {
                if bits == 8 { value } else { self.b.ins().uextend(to.clif(), value) }
            }
            (Num::Int { bits, signed }, Num::F32 | Num::F64) => {
                let wide = if bits == 64 { value } else if signed { self.b.ins().sextend(types::I64, value) } else { self.b.ins().uextend(types::I64, value) };
                let float = if signed { self.b.ins().fcvt_from_sint(types::F64, wide) } else { self.b.ins().fcvt_from_uint(types::F64, wide) };
                if to == Num::F32 { self.b.ins().fdemote(types::F32, float) } else { float }
            }
            (Num::F32 | Num::F64, Num::Int { bits, signed }) => {
                let float = if from == Num::F32 { self.b.ins().fpromote(types::F64, value) } else { value };
                if bits == 64 && !signed {
                    self.b.ins().fcvt_to_uint_sat(types::I64, float)
                } else {
                    let wide = self.b.ins().fcvt_to_sint_sat(types::I64, float);
                    if bits == 64 {
                        wide
                    } else {
                        let (low, high) = if signed { (-(1i64 << (bits - 1)), (1i64 << (bits - 1)) - 1) } else { (0, (1i64 << bits) - 1) };
                        let low = self.b.ins().iconst(types::I64, low);
                        let high = self.b.ins().iconst(types::I64, high);
                        let clamped = self.b.ins().smax(wide, low);
                        let clamped = self.b.ins().smin(clamped, high);
                        self.b.ins().ireduce(to.clif(), clamped)
                    }
                }
            }
            (Num::F32, Num::F64) => self.b.ins().fpromote(types::F64, value),
            (Num::F64, Num::F32) => self.b.ins().fdemote(types::F32, value),
            (Num::F32, Num::F32) | (Num::F64, Num::F64) => value,
            _ => return Err(CompileError::internal("this cast")),
        })
    }
}

/// The low `bits` of a value, as an immediate that extends its sign.
pub(crate) fn sign_extended(value: i128, bits: u32) -> i64 {
    let shift = 64 - bits;
    (((value as u64) << shift) as i64) >> shift
}

/// The Cranelift parameter list of a signature built from the ABI scalars.
pub(crate) fn abi_params(scalars: &[mote_abi::Scalar]) -> Vec<AbiParam> {
    scalars.iter().map(|&scalar| AbiParam::new(clif_type(scalar))).collect()
}

pub(crate) fn clif_type(scalar: mote_abi::Scalar) -> Type {
    use mote_abi::Scalar;
    match scalar {
        Scalar::I8 => types::I8,
        Scalar::I16 => types::I16,
        Scalar::I32 => types::I32,
        Scalar::I64 | Scalar::Ref => types::I64,
        Scalar::F32 => types::F32,
        Scalar::F64 => types::F64,
    }
}

/// The blocks a jump returns to from further down a depth-first walk from the entry, which are the heads of the loops.
fn loop_headers(function: &Function) -> HashSet<mote_ir::BlockId> {
    let mut open = HashSet::new();
    let mut done = HashSet::new();
    let mut headers = HashSet::new();
    let mut stack = vec![(function.entry, function.blocks[function.entry].term.targets(), 0)];
    open.insert(function.entry);
    while let Some((block, targets, next)) = stack.last_mut() {
        let block = *block;
        match targets.get(*next).copied() {
            None => {
                open.remove(&block);
                done.insert(block);
                stack.pop();
            }
            Some(target) => {
                *next += 1;
                if open.contains(&target) {
                    headers.insert(target);
                } else if !done.contains(&target) {
                    open.insert(target);
                    stack.push((target, function.blocks[target].term.targets(), 0));
                }
            }
        }
    }
    headers
}
