//! Lowers closures, the cells `var`s are captured in, and calls through a function value.

use cranelift_codegen::ir::{types, InstBuilder, Signature, Value};
use cranelift_codegen::isa::CallConv;
use mote_abi::{closure_signature, passing, Passing, TypeId, BOX_VALUE_OFFSET, CLOSURE_CODE_OFFSET};
use mote_ir::{Const, FuncId, Inst, Operand, RegId};
use mote_semantics::TyKind;

use crate::function::{abi_params, Lowering};
use crate::scalar::Num;
use crate::values::{internal, Addr};
use crate::CompileError;

impl Lowering<'_, '_, '_> {
    /// A function value over the captured values: a static object when it captures nothing.
    pub(crate) fn closure(&mut self, inst: &Inst, func: FuncId, captures: &[Operand]) -> Result<(), CompileError> {
        let object = self.closure_object(func, captures)?;
        self.set(inst.dest, Some(object));
        Ok(())
    }

    /// The closure object over the captured values.
    pub(crate) fn closure_object(&mut self, func: FuncId, captures: &[Operand]) -> Result<Value, CompileError> {
        if captures.is_empty() {
            let data = *self.env.statics.get(&func).ok_or_else(|| internal("a closure with no static object"))?;
            let global = self.env.clif.declare_data_in_func(data, self.b.func);
            return Ok(self.b.ins().symbol_value(types::I64, global));
        }
        let id = self.env.types.closure(func);
        let object = self.alloc(id);
        let code = self.func_ref(*self.env.funcs.get(&func).ok_or_else(|| internal("a closure over a function that was not compiled"))?);
        let code = self.b.ins().func_addr(types::I64, code);
        self.store(code, Addr { base: object, offset: CLOSURE_CODE_OFFSET as i32 });
        let target = &self.env.module.functions[func];
        let fields = self.env.types.table.get(id).object.as_ref().map(|shape| shape.fields.clone()).unwrap_or_default();
        for ((operand, &capture), field) in captures.iter().zip(&target.captures).zip(&fields) {
            let at = Addr { base: object, offset: field.offset as i32 };
            if target.regs[capture].cell {
                let cell = self.num_operand(operand, Num::Ref)?;
                self.store_reference(cell, at);
            } else {
                self.write(at, field.ty, operand)?;
            }
        }
        Ok(object)
    }

    /// The table's type of the value in a cell: the type of the cell register.
    fn cell_value_type(&self, cell: &Operand) -> Result<TypeId, CompileError> {
        match cell {
            Operand::Reg(reg) => Ok(self.reg_type(*reg)),
            Operand::Const(_) => Err(internal("a constant where a cell is read")),
        }
    }

    /// A new cell holding the value; `null` leaves it zeroed.
    pub(crate) fn new_cell(&mut self, inst: &Inst, value: &Operand) -> Result<(), CompileError> {
        let reg = inst.dest.ok_or_else(|| internal("a cell with no destination"))?;
        let ty = self.function.regs[reg].ty;
        let cell = self.alloc(self.env.types.boxed(ty));
        if !matches!(value, Operand::Const(Const::Null)) {
            self.write(Addr { base: cell, offset: BOX_VALUE_OFFSET as i32 }, self.env.types.id(ty), value)?;
        }
        self.set(inst.dest, Some(cell));
        Ok(())
    }

    pub(crate) fn load_cell(&mut self, inst: &Inst, cell: &Operand) -> Result<(), CompileError> {
        let id = self.cell_value_type(cell)?;
        let base = self.num_operand(cell, Num::Ref)?;
        self.read_into(inst.dest, Addr { base, offset: BOX_VALUE_OFFSET as i32 }, id)
    }

    pub(crate) fn store_cell(&mut self, cell: &Operand, value: &Operand) -> Result<(), CompileError> {
        let id = self.cell_value_type(cell)?;
        let base = self.num_operand(cell, Num::Ref)?;
        self.write(Addr { base, offset: BOX_VALUE_OFFSET as i32 }, id, value)
    }

    /// A call through a function value: the context, the result slot, the closure, the arguments, then the block for the `var` parameters.
    pub(crate) fn call_value(&mut self, inst: &Inst, closure: &Operand, args: &[Operand], outs: &[RegId]) -> Result<(), CompileError> {
        let TyKind::Function(function) = self.env.program.types.kind(self.ty_of(closure)).clone() else { return Err(CompileError::internal("a call of a value that is not a function")) };
        let types = self.env.types;
        let ret = types.id(function.ret);
        let ids: Vec<TypeId> = function.params.iter().map(|param| types.id(param.ty)).collect();
        let abi = closure_signature(&types.table, &ids, ret);
        let mut params = abi_params(&abi.params);
        let vars: Vec<TypeId> = function.params.iter().zip(&ids).filter(|(param, _)| param.is_var).map(|(_, &id)| id).collect();
        if !vars.is_empty() {
            params.push(cranelift_codegen::ir::AbiParam::new(types::I64));
        }
        let signature = Signature { params, returns: abi_params(&abi.results), call_conv: CallConv::Tail };

        let object = self.num_operand(closure, Num::Ref)?;
        let mut values = vec![self.context];
        let mut result_at = None;
        if abi.result_in_memory {
            let at = self.temp(types.table.layout(ret).size);
            values.push(self.address(at));
            result_at = Some(at);
        }
        values.push(object);
        for (arg, &id) in args.iter().zip(&ids) {
            match passing(&types.table, id) {
                Passing::Registers(_) => values.extend(self.operand_leaves(arg, id)?),
                Passing::Memory => {
                    let at = self.temp(types.table.layout(id).size);
                    self.write(at, id, arg)?;
                    values.push(self.address(at));
                }
            }
        }
        let area = if vars.is_empty() {
            None
        } else {
            let (places, total) = self.out_layout(&vars);
            let at = self.temp(total);
            values.push(self.address(at));
            Some((at, places))
        };
        let code = self.load(types::I64, Addr { base: object, offset: CLOSURE_CODE_OFFSET as i32 });
        let signature = self.b.import_signature(signature);
        let call = self.b.ins().call_indirect(signature, code, &values);
        let results = self.b.inst_results(call).to_vec();
        if let Some(dest) = inst.dest {
            match result_at {
                Some(at) => self.read_into(Some(dest), at, ret)?,
                None => self.store_leaves(dest, ret, &results)?,
            }
        }
        if let Some((at, places)) = area {
            for (&out, &(offset, id)) in outs.iter().zip(&places) {
                self.read_into(Some(out), at.at(offset), id)?;
            }
        }
        Ok(())
    }
}
