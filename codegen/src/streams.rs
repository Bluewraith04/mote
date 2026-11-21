//! Lowers streams: a `Stream` is a closure object over a resume function, and its frame is the object's fields.

use cranelift_codegen::ir::{types, InstBuilder, Signature, Value};
use cranelift_codegen::isa::CallConv;
use mote_abi::{closure_signature, CLOSURE_CODE_OFFSET};
use mote_ir::{FuncId, Inst, Operand, RegId};

use crate::function::{abi_params, Lowering};
use crate::scalar::Num;
use crate::values::{internal, Addr};
use crate::CompileError;

impl Lowering<'_, '_, '_> {
    /// A stream over a new frame: slot 0 is zero, the next slots hold the initial values.
    pub(crate) fn new_stream(&mut self, inst: &Inst, resume: FuncId, init: &[Operand]) -> Result<(), CompileError> {
        let id = self.env.types.closure(resume);
        let object = self.alloc(id);
        let code = self.func_ref(*self.env.funcs.get(&resume).ok_or_else(|| internal("a stream over a function that was not compiled"))?);
        let code = self.b.ins().func_addr(types::I64, code);
        self.store(code, Addr { base: object, offset: CLOSURE_CODE_OFFSET as i32 });
        for (slot, value) in (1..).zip(init) {
            self.store_slot(object, resume, slot, value)?;
        }
        self.set(inst.dest, Some(object));
        Ok(())
    }

    /// Stores a value or a cell in a slot of the frame object of the resume function.
    fn store_slot(&mut self, object: Value, resume: FuncId, slot: u32, value: &Operand) -> Result<(), CompileError> {
        let id = self.env.types.closure(resume);
        let fields = self.env.types.table.get(id).object.as_ref().map(|shape| shape.fields.clone()).unwrap_or_default();
        let field = fields.get(slot as usize).ok_or_else(|| internal("a frame slot that does not exist"))?;
        let at = Addr { base: object, offset: field.offset as i32 };
        if self.env.module.functions[resume].frame[slot as usize].cell {
            let cell = self.num_operand(value, Num::Ref)?;
            self.store_reference(cell, at);
            Ok(())
        } else {
            self.write(at, field.ty, value)
        }
    }

    /// The frame object of the resume function being lowered.
    fn frame_object(&self) -> Result<Value, CompileError> {
        self.closure.ok_or_else(|| internal("a frame access outside a resume function"))
    }

    pub(crate) fn frame_load(&mut self, inst: &Inst, slot: u32) -> Result<(), CompileError> {
        let object = self.frame_object()?;
        let id = self.env.types.closure(self.func);
        let field = self.env.types.table.get(id).object.as_ref().and_then(|shape| shape.fields.get(slot as usize).copied()).ok_or_else(|| internal("a frame slot that does not exist"))?;
        let at = Addr { base: object, offset: field.offset as i32 };
        if self.function.frame[slot as usize].cell {
            let cell = self.load(types::I64, at);
            let dest = Self::dest_reg(inst)?;
            self.define(dest, cell);
            Ok(())
        } else {
            self.read_into(inst.dest, at, field.ty)
        }
    }

    pub(crate) fn frame_store(&mut self, slot: u32, value: &Operand) -> Result<(), CompileError> {
        let object = self.frame_object()?;
        self.store_slot(object, self.func, slot, value)
    }

    /// `stream_next`: calls the resume function of the stream with the stream as its closure; the stream is its own final value.
    pub(crate) fn stream_intrinsic(&mut self, name: &str, inst: &Inst, args: &[Operand], outs: &[RegId]) -> Result<(), CompileError> {
        let [stream] = args else { return Err(CompileError::internal(format!("the intrinsic `{name}` with these arguments"))) };
        let object = self.num_operand(stream, Num::Ref)?;
        let types = self.env.types;
        let ret = self.reg_type(Self::dest_reg(inst)?);
        let abi = closure_signature(&types.table, &[], ret);
        let signature = Signature { params: abi_params(&abi.params), returns: abi_params(&abi.results), call_conv: CallConv::Tail };
        let mut values = vec![self.context];
        let result_at = if abi.result_in_memory {
            let at = self.temp(types.table.layout(ret).size);
            values.push(self.address(at));
            Some(at)
        } else {
            None
        };
        values.push(object);
        let code = self.load(types::I64, Addr { base: object, offset: CLOSURE_CODE_OFFSET as i32 });
        let signature = self.b.import_signature(signature);
        let call = self.b.ins().call_indirect(signature, code, &values);
        let results = self.b.inst_results(call).to_vec();
        let dest = Self::dest_reg(inst)?;
        match result_at {
            Some(at) => self.read_into(Some(dest), at, ret)?,
            None => self.store_leaves(dest, ret, &results)?,
        }
        if let Some(&out) = outs.first() {
            self.set(Some(out), Some(object));
        }
        Ok(())
    }
}
