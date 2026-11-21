//! Lowers tasks: `Spawn`, scopes, the rows that await, cancel and ask a task, and the function that runs a task.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, FuncRef, InstBuilder, MemFlags, Signature, StackSlotData, StackSlotKind, Value};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::FuncId as ClifFunc;
use mote_abi::{closure_signature, signature, HandleKind, Kind, TypeId, TypeTable, BOX_VALUE_OFFSET, CLOSURE_CODE_OFFSET, TASK_CONVERT_OFFSET, TASK_DONE, TASK_FAULTED, TASK_NUMBER_OFFSET, TASK_PENDING};
use mote_ir::{FuncId, Inst, Operand, RegId};
use mote_semantics::LangItem;

use crate::function::{abi_params, clif_type, Lowering};
use crate::scalar::Num;
use crate::values::{emit_store, internal, leaves_of, tag_type, Addr, Repr};
use crate::CompileError;

/// The `Int` a scope token is held as.
const TOKEN: Num = Num::Int { bits: 64, signed: true };

/// Where the result of a task lies in its handle.
fn payload_offset() -> i32 {
    HandleKind::Task.fixed_size() as i32
}

/// The type of the result a task type holds.
pub(crate) fn task_result(table: &TypeTable, task: TypeId) -> Option<TypeId> {
    match &table.get(task).kind {
        Kind::Handle(HandleKind::Task, args) => args.first().copied(),
        _ => None,
    }
}

/// The body of the function that runs a task: `(context, closure, handle)`, which calls the closure and stores its result in the handle.
pub(crate) fn thunk(builder: &mut FunctionBuilder, table: &TypeTable, task: TypeId) -> Result<(), CompileError> {
    let result = task_result(table, task).ok_or_else(|| internal("a task function for a type that is not a task"))?;
    let abi = closure_signature(table, &[], result);
    let first = builder.create_block();
    builder.append_block_params_for_function_params(first);
    builder.switch_to_block(first);
    let [context, closure, handle] = builder.block_params(first)[..] else { return Err(internal("a task function without three parameters")) };
    let mut args = vec![context];
    if abi.result_in_memory {
        args.push(builder.ins().iadd_imm(handle, i64::from(payload_offset())));
    }
    args.push(closure);
    let code = builder.ins().load(types::I64, MemFlags::trusted(), closure, CLOSURE_CODE_OFFSET as i32);
    let signature = builder.import_signature(Signature { params: abi_params(&abi.params), returns: abi_params(&abi.results), call_conv: CallConv::Tail });
    let call = builder.ins().call_indirect(signature, code, &args);
    if !abi.result_in_memory {
        let mut leaves = Vec::new();
        leaves_of(table, result, 0, &mut leaves);
        let results = builder.inst_results(call).to_vec();
        for ((offset, _), value) in leaves.iter().zip(results) {
            emit_store(builder, value, Addr { base: handle, offset: payload_offset() + *offset as i32 });
        }
    }
    builder.ins().return_(&[]);
    builder.seal_all_blocks();
    Ok(())
}

/// The body of the function that closes a `with` for a cancelled task: `(context, box)`, which calls the close function with the resource the box holds.
///
/// `valued` is whether the close function takes the environment of a closure, which is null here; a value it gives is dropped.
pub(crate) fn close_thunk(builder: &mut FunctionBuilder, table: &TypeTable, target: FuncRef, resource: TypeId, ret: TypeId, valued: bool) -> Result<(), CompileError> {
    let abi = signature(table, &[resource], ret);
    let first = builder.create_block();
    builder.append_block_params_for_function_params(first);
    builder.switch_to_block(first);
    let [context, held] = builder.block_params(first)[..] else { return Err(internal("a close function without two parameters")) };
    let mut args = vec![context];
    if abi.result_in_memory {
        let size = table.layout(ret).size.div_ceil(8) * 8;
        let slot = builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, size, 3));
        args.push(builder.ins().stack_addr(types::I64, slot, 0));
    }
    if valued {
        args.push(builder.ins().iconst(types::I64, 0));
    }
    if abi.args_in_memory.first().copied().unwrap_or(false) {
        args.push(builder.ins().iadd_imm(held, i64::from(BOX_VALUE_OFFSET)));
    } else {
        let mut leaves = Vec::new();
        leaves_of(table, resource, 0, &mut leaves);
        for (offset, scalar) in leaves {
            args.push(builder.ins().load(clif_type(scalar), MemFlags::trusted(), held, BOX_VALUE_OFFSET as i32 + offset as i32));
        }
    }
    builder.ins().call(target, &args);
    builder.ins().return_(&[]);
    builder.seal_all_blocks();
    Ok(())
}

impl Lowering<'_, '_, '_> {
    /// `Defer`: records the `with`, with a box that holds the resource and the function that closes it from the box.
    pub(crate) fn defer(&mut self, resource: &Operand, func: FuncId) -> Result<(), CompileError> {
        let ty = self.ty_of(resource);
        let thunk = *self.env.defer_thunks.get(&func).ok_or_else(|| internal("a with whose close has no function"))?;
        let held = self.alloc(self.env.types.boxed(ty));
        self.write(Addr { base: held, offset: BOX_VALUE_OFFSET as i32 }, self.env.types.id(ty), resource)?;
        let thunk = self.func_ref(thunk);
        let thunk = self.b.ins().func_addr(types::I64, thunk);
        self.entry(self.env.imports.defer, &[held, thunk]);
        Ok(())
    }

    /// `Undefer`: the innermost `with` has ended.
    pub(crate) fn undefer(&mut self) {
        self.entry(self.env.imports.undefer, &[]);
    }

    /// Calls an entry of the runtime with the context first.
    pub(crate) fn entry(&mut self, import: ClifFunc, args: &[Value]) -> Vec<Value> {
        let entry = self.func_ref(import);
        let mut all = vec![self.context];
        all.extend_from_slice(args);
        let call = self.b.ins().call(entry, &all);
        self.b.inst_results(call).to_vec()
    }

    /// `Spawn`: a task over a closure of the function and the captured values.
    pub(crate) fn spawn(&mut self, inst: &Inst, func: FuncId, captures: &[Operand], scope: Option<&Operand>) -> Result<(), CompileError> {
        let closure = self.closure_object(func, captures)?;
        self.start(inst, closure, scope)
    }

    fn start(&mut self, inst: &Inst, closure: Value, scope: Option<&Operand>) -> Result<(), CompileError> {
        let dest = inst.dest.ok_or_else(|| internal("a spawn with no destination"))?;
        let task = self.reg_type(dest);
        let thunk = *self.env.task_thunks.get(&task).ok_or_else(|| internal("a task type with no function"))?;
        let thunk = self.func_ref(thunk);
        let thunk = self.b.ins().func_addr(types::I64, thunk);
        let ty = self.b.ins().iconst(types::I32, i64::from(task.raw()));
        let scope = match scope {
            Some(token) => self.num_operand(token, TOKEN)?,
            None => self.b.ins().iconst(types::I64, -1),
        };
        let handle = self.entry(self.env.imports.task_spawn, &[ty, thunk, closure, scope])[0];
        self.set(inst.dest, Some(handle));
        Ok(())
    }

    pub(crate) fn scope_begin(&mut self, inst: &Inst) -> Result<(), CompileError> {
        let token = self.entry(self.env.imports.scope_begin, &[])[0];
        self.set(inst.dest, Some(token));
        Ok(())
    }

    /// `ScopeEnd`: waits for the tasks of the scope; one that faulted and nobody awaited stops the task, or, with a destination, is the `Err` the optional holds.
    pub(crate) fn scope_end(&mut self, inst: &Inst, token: &Operand) -> Result<(), CompileError> {
        let token = self.num_operand(token, TOKEN)?;
        let Some(dest) = inst.dest else {
            self.entry(self.env.imports.scope_end, &[token]);
            return Ok(());
        };
        let optional = self.env.types.table.get(self.reg_type(dest));
        if optional.is_reference() {
            return Err(CompileError::internal("a scope whose result is held by reference"));
        }
        let size = optional.layout.size;
        let field = optional.variants.get(1).and_then(|variant| variant.fields.first().copied()).ok_or_else(|| internal("an optional with no payload"))?;
        let message = self.entry(self.env.imports.scope_end_error, &[token])[0];
        let failed = self.b.ins().icmp_imm(IntCC::NotEqual, message, 0);
        let (some, none, join) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.ins().brif(failed, some, &[], none, &[]);

        self.b.switch_to_block(none);
        let at = self.reg_addr(dest)?;
        self.zero_mem(at, size);
        self.b.ins().jump(join, &[]);

        self.b.switch_to_block(some);
        let at = self.reg_addr(dest)?;
        let present = self.b.ins().iconst(types::I64, 1);
        self.store(present, at);
        let faulted = self.b.ins().iconst(types::I8, 1);
        self.write_error(dest, field.ty, field.offset, faulted, message)?;
        self.b.ins().jump(join, &[]);

        self.b.switch_to_block(join);
        Ok(())
    }

    /// The rows `task_await`, `task_cancel`, `task_is_ready` and `spawn_call`.
    pub(crate) fn task_intrinsic(&mut self, name: &str, inst: &Inst, args: &[Operand]) -> Result<(), CompileError> {
        match (name, args) {
            ("task_await", [task]) => self.task_await(inst, task),
            ("task_await_timeout", [task, ms]) => self.task_await_timeout(inst, task, ms),
            ("task_view", [task, convert]) => self.task_view(inst, task, convert),
            ("task_value", [task]) => self.task_value(inst, task),
            ("sleep", [ms]) => {
                let ms = self.num_operand(ms, TOKEN)?;
                self.entry(self.env.imports.sleep, &[ms]);
                Ok(())
            }
            ("task_cancel", [task]) => {
                let handle = self.num_operand(task, Num::Ref)?;
                self.entry(self.env.imports.task_cancel, &[handle]);
                Ok(())
            }
            ("task_is_ready", [task]) => {
                let handle = self.num_operand(task, Num::Ref)?;
                let ready = self.entry(self.env.imports.task_ready, &[handle])[0];
                let ready = self.b.ins().ireduce(types::I8, ready);
                self.set(inst.dest, Some(ready));
                Ok(())
            }
            ("spawn_call", [work]) => {
                let closure = self.num_operand(work, Num::Ref)?;
                self.start(inst, closure, None)
            }
            _ => Err(CompileError::internal(format!("the intrinsic `{name}` with these arguments"))),
        }
    }

    /// `task_view`: a handle of the dest's task type that shares the task of the first and converts its result with the closure.
    fn task_view(&mut self, inst: &Inst, task: &Operand, convert: &Operand) -> Result<(), CompileError> {
        let dest = inst.dest.ok_or_else(|| internal("a task_view with no destination"))?;
        let view = self.alloc(self.reg_type(dest));
        let source = self.num_operand(task, Num::Ref)?;
        let convert = self.num_operand(convert, Num::Ref)?;
        let number = self.load(types::I64, Addr { base: source, offset: TASK_NUMBER_OFFSET as i32 });
        self.store(number, Addr { base: view, offset: TASK_NUMBER_OFFSET as i32 });
        self.store_reference(convert, Addr { base: view, offset: TASK_CONVERT_OFFSET as i32 });
        self.set(inst.dest, Some(view));
        Ok(())
    }

    /// `task_value`: the result of a task that has returned.
    fn task_value(&mut self, inst: &Inst, task: &Operand) -> Result<(), CompileError> {
        let dest = inst.dest.ok_or_else(|| internal("a task_value with no destination"))?;
        let (task_ty, result) = (self.env.types.id(self.ty_of(task)), self.reg_type(dest));
        let handle = self.num_operand(task, Num::Ref)?;
        let at = self.temp(self.env.types.table.layout(result).size);
        self.task_result_into(handle, task_ty, result, at)?;
        self.read_into(Some(dest), at, result)
    }

    /// Copies the result of a returned task to the address: what its converter gives when the handle is a view, else the payload.
    fn task_result_into(&mut self, handle: Value, task: TypeId, result: TypeId, to: Addr) -> Result<(), CompileError> {
        let payload = Addr { base: handle, offset: payload_offset() };
        if !self.env.viewed_tasks.contains(&task) {
            return self.copy_value_at(payload, to, result);
        }
        let convert = self.load(types::I64, Addr { base: handle, offset: TASK_CONVERT_OFFSET as i32 });
        let (viewed, plain, join) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.ins().brif(convert, viewed, &[], plain, &[]);

        self.b.switch_to_block(viewed);
        self.call_converter(convert, result, to)?;
        self.b.ins().jump(join, &[]);

        self.b.switch_to_block(plain);
        self.copy_value_at(payload, to, result)?;
        self.b.ins().jump(join, &[]);

        self.b.switch_to_block(join);
        Ok(())
    }

    /// Calls the converter closure of a view, which takes nothing, and copies what it gives to the address.
    fn call_converter(&mut self, closure: Value, result: TypeId, to: Addr) -> Result<(), CompileError> {
        let types = self.env.types;
        let abi = closure_signature(&types.table, &[], result);
        let mut args = vec![self.context];
        let held = abi.result_in_memory.then(|| self.temp(types.table.layout(result).size));
        if let Some(at) = held {
            args.push(self.address(at));
        }
        args.push(closure);
        let code = self.load(types::I64, Addr { base: closure, offset: CLOSURE_CODE_OFFSET as i32 });
        let signature = self.b.import_signature(Signature { params: abi_params(&abi.params), returns: abi_params(&abi.results), call_conv: CallConv::Tail });
        let call = self.b.ins().call_indirect(signature, code, &args);
        match held {
            Some(at) => self.copy_value_at(at, to, result),
            None => {
                let mut leaves = Vec::new();
                self.leaves(result, 0, &mut leaves);
                let results = self.b.inst_results(call).to_vec();
                for ((offset, _), value) in leaves.iter().zip(results) {
                    self.store(value, to.at(*offset));
                }
                Ok(())
            }
        }
    }

    /// The position of a variant of `ErrorKind`.
    fn error_kind(&self, name: &str) -> Result<i64, CompileError> {
        let def = *self.env.program.lang.get(&LangItem::ErrorKind).ok_or_else(|| internal("no ErrorKind"))?;
        let item = self.env.program.defs[def].enum_def().ok_or_else(|| internal("an ErrorKind that is not an enum"))?;
        item.variants.iter().position(|variant| self.env.interner.resolve(variant.name) == name).map(|at| at as i64).ok_or_else(|| internal("an ErrorKind without the variant"))
    }

    /// Copies a value of the table's type from one place to another.
    pub(crate) fn copy_value_at(&mut self, from: Addr, to: Addr, id: TypeId) -> Result<(), CompileError> {
        if matches!(self.env.types.table.get(id).kind, Kind::Box(_)) {
            return Err(CompileError::internal("a task result held in a box"));
        }
        match self.repr_of(id)? {
            Repr::Nothing => {}
            Repr::Num(num) => {
                let value = self.load(num.clif(), from);
                if num == Num::Ref {
                    self.store_reference(value, to);
                } else {
                    self.store(value, to);
                }
            }
            Repr::Mem(size) => self.copy_mem(to, from, size),
        }
        Ok(())
    }

    /// `await`: `Ok` of the result when the task returned, else an `Err` of an `Error` that says why not.
    fn task_await(&mut self, inst: &Inst, task: &Operand) -> Result<(), CompileError> {
        let dest = inst.dest.ok_or_else(|| internal("an await with no destination"))?;
        let task_ty = self.env.types.id(self.ty_of(task));
        let handle = self.num_operand(task, Num::Ref)?;
        let state = self.entry(self.env.imports.task_await, &[handle])[0];
        self.await_result(dest, self.reg_type(dest), 0, task_ty, handle, state)
    }

    /// `await_timeout`: null when the time ran out, else what `await` gives.
    fn task_await_timeout(&mut self, inst: &Inst, task: &Operand, ms: &Operand) -> Result<(), CompileError> {
        let dest = inst.dest.ok_or_else(|| internal("an await_timeout with no destination"))?;
        let optional = self.env.types.table.get(self.reg_type(dest));
        if optional.is_reference() {
            return Err(CompileError::internal("an await_timeout whose result is held by reference"));
        }
        let size = optional.layout.size;
        let field = optional.variants.get(1).and_then(|variant| variant.fields.first().copied()).ok_or_else(|| internal("an optional with no payload"))?;
        let task_ty = self.env.types.id(self.ty_of(task));
        let handle = self.num_operand(task, Num::Ref)?;
        let ms = self.num_operand(ms, TOKEN)?;
        let state = self.entry(self.env.imports.task_await_timeout, &[handle, ms])[0];
        let timed_out = self.b.ins().icmp_imm(IntCC::Equal, state, TASK_PENDING as i64);
        let (none, some, join) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.ins().brif(timed_out, none, &[], some, &[]);

        self.b.switch_to_block(none);
        let at = self.reg_addr(dest)?;
        self.zero_mem(at, size);
        self.b.ins().jump(join, &[]);

        self.b.switch_to_block(some);
        let at = self.reg_addr(dest)?;
        let present = self.b.ins().iconst(types::I64, 1);
        self.store(present, at);
        self.await_result(dest, field.ty, field.offset, task_ty, handle, state)?;
        self.b.ins().jump(join, &[]);

        self.b.switch_to_block(join);
        Ok(())
    }

    /// Writes `Ok` of the task's result, or an `Err` that says why not, as the `Result` type at the offset of the register, for the state the task ended in.
    fn await_result(&mut self, dest: RegId, result: TypeId, base: u32, task: TypeId, handle: Value, state: Value) -> Result<(), CompileError> {
        let types = self.env.types;
        let info = types.table.get(result);
        if !matches!(info.kind, Kind::Enum(_)) {
            return Err(CompileError::internal("an await whose result is not a Result"));
        }
        let ok = self.env.ok_tag.ok_or_else(|| internal("no Ok variant"))?;
        let ok_field = info.variants[ok as usize].fields.first().copied();
        let tag = tag_type(info.tag_size);

        let done = self.b.ins().icmp_imm(IntCC::Equal, state, TASK_DONE as i64);
        let (returned, failed, join) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.ins().brif(done, returned, &[], failed, &[]);

        self.b.switch_to_block(returned);
        let at = self.reg_addr(dest)?.at(base);
        let ok_tag = self.b.ins().iconst(tag, i64::from(ok));
        self.store(ok_tag, at);
        if let Some(field) = ok_field {
            self.task_result_into(handle, task, field.ty, at.at(field.offset))?;
        }
        self.b.ins().jump(join, &[]);

        self.b.switch_to_block(failed);
        let message = self.entry(self.env.imports.task_message, &[handle])[0];
        let faulted = self.b.ins().icmp_imm(IntCC::Equal, state, TASK_FAULTED as i64);
        self.write_error(dest, result, base, faulted, message)?;
        self.b.ins().jump(join, &[]);

        self.b.switch_to_block(join);
        Ok(())
    }

    /// Writes an `Err` of an `Error` with the message as the `Result` type at the offset of the register; its kind is `Other` when `faulted` is not zero and `Interrupted` otherwise.
    fn write_error(&mut self, dest: RegId, result: TypeId, base: u32, faulted: Value, message: Value) -> Result<(), CompileError> {
        let types = self.env.types;
        let info = types.table.get(result);
        let ok = self.env.ok_tag.ok_or_else(|| internal("no Ok variant"))?;
        let err = (0..info.variants.len() as u32).find(|&at| at != ok).ok_or_else(|| internal("no Err variant"))?;
        let err_field = info.variants[err as usize].fields.first().copied().ok_or_else(|| internal("an Err without a payload"))?;
        let shape = types.table.get(err_field.ty).object.as_ref().ok_or_else(|| internal("an error that is not an object"))?;
        let (kind_field, message_field) = match (shape.fields.first().copied(), shape.fields.get(1).copied()) {
            (Some(kind), Some(message)) => (kind, message),
            _ => return Err(internal("an Error without a kind and a message")),
        };
        let kind_tag = tag_type(types.table.get(kind_field.ty).tag_size);
        let (other, interrupted) = (self.error_kind("Other")?, self.error_kind("Interrupted")?);
        let tag = tag_type(info.tag_size);
        let object = self.alloc(err_field.ty);
        let (by_fault, by_cancel) = (self.b.ins().iconst(kind_tag, other), self.b.ins().iconst(kind_tag, interrupted));
        let kind = self.b.ins().select(faulted, by_fault, by_cancel);
        self.store(kind, Addr { base: object, offset: kind_field.offset as i32 });
        self.store_reference(message, Addr { base: object, offset: message_field.offset as i32 });
        let at = self.reg_addr(dest)?.at(base);
        let err_tag = self.b.ins().iconst(tag, i64::from(err));
        self.store(err_tag, at);
        self.store_reference(object, at.at(err_field.offset));
        Ok(())
    }
}
