//! Lowers the parameters, the results and the calls of functions under the `tail` convention.

use cranelift_codegen::ir::{self, InstBuilder};
use mote_abi::{align_up, passing, Passing, TypeId};
use mote_ir::{FuncId, Function, Inst, Operand, RegId};

use crate::function::Lowering;
use crate::values::{Addr, Repr};
use crate::CompileError;

impl Lowering<'_, '_, '_> {
    /// Where each `var` parameter's final value is stored in the block the caller provides, and the bytes the block takes.
    fn out_places(&self, function: &Function) -> (Vec<(u32, TypeId)>, u32) {
        let types = self.env.types;
        let ids: Vec<TypeId> = function.out_params.iter().map(|&position| types.id(function.regs[function.params[position]].ty)).collect();
        self.out_layout(&ids)
    }

    /// Where the final values of `var` parameters of these types are stored, and the bytes the block takes.
    pub(crate) fn out_layout(&self, ids: &[TypeId]) -> (Vec<(u32, TypeId)>, u32) {
        let types = self.env.types;
        let mut end = 0;
        let mut placed = Vec::new();
        for &id in ids {
            let layout = types.table.layout(id);
            let offset = align_up(end, layout.align);
            placed.push((offset, id));
            end = offset + layout.size;
        }
        (placed, end)
    }

    /// Binds the parameters of the entry block to the parameter registers.
    pub(crate) fn bind_params(&mut self, start: ir::Block) -> Result<(), CompileError> {
        let function = self.function;
        let types = self.env.types;
        let values = self.b.block_params(start).to_vec();
        let mut next = 1;
        if matches!(passing(&types.table, types.id(function.ret)), Passing::Memory) {
            self.result_addr = Some(values[next]);
            next += 1;
        }
        let closure = if self.env.closure_conv.contains(&self.func) {
            next += 1;
            Some(values[next - 1])
        } else {
            None
        };
        self.closure = closure;
        for &reg in &function.params {
            let id = types.id(function.regs[reg].ty);
            match passing(&types.table, id) {
                Passing::Registers(scalars) => {
                    self.store_leaves(reg, id, &values[next..next + scalars.len()])?;
                    next += scalars.len();
                }
                Passing::Memory => {
                    let Repr::Mem(size) = self.repr_of(id)? else { return Err(CompileError::internal("a parameter in memory with no room")) };
                    let to = self.reg_addr(reg)?;
                    self.copy_mem(to, Addr { base: values[next], offset: 0 }, size);
                    next += 1;
                }
            }
        }
        if !function.out_params.is_empty() {
            self.out_area = Some(values[next]);
        }
        if let Some(closure) = closure.filter(|_| !function.captures.is_empty()) {
            let id = types.closure(self.func);
            let fields = types.table.get(id).object.as_ref().map(|object| object.fields.clone()).unwrap_or_default();
            for (&capture, field) in function.captures.iter().zip(&fields) {
                let at = Addr { base: closure, offset: field.offset as i32 };
                if function.regs[capture].cell {
                    let cell = self.load(ir::types::I64, at);
                    if !self.vars.contains_key(&capture) {
                        return Err(CompileError::internal("a cell with no variable"));
                    }
                    self.define(capture, cell);
                } else {
                    self.read_into(Some(capture), at, field.ty)?;
                }
            }
        }
        Ok(())
    }

    /// Stores the final `var` parameters where the caller asked, then returns the value.
    pub(crate) fn return_value(&mut self, value: Option<&Operand>) -> Result<(), CompileError> {
        let function = self.function;
        let types = self.env.types;
        if let Some(area) = self.out_area {
            let (places, _) = self.out_places(function);
            for (&position, &(offset, id)) in function.out_params.iter().zip(&places) {
                self.write(Addr { base: area, offset: offset as i32 }, id, &Operand::Reg(function.params[position]))?;
            }
        }
        let ret = types.id(function.ret);
        match passing(&types.table, ret) {
            Passing::Registers(_) => {
                let values = match value {
                    Some(operand) => self.operand_leaves(operand, ret)?,
                    None => Vec::new(),
                };
                self.b.ins().return_(&values);
            }
            Passing::Memory => {
                let base = self.result_addr.ok_or_else(|| CompileError::internal("a result in memory with no address"))?;
                if let Some(operand) = value {
                    self.write(Addr { base, offset: 0 }, ret, operand)?;
                }
                self.b.ins().return_(&[]);
            }
        }
        Ok(())
    }

    /// A direct call: the context, the result slot, the arguments, then the block for the `var` parameters.
    pub(crate) fn call_function(&mut self, inst: &Inst, target: FuncId, args: &[Operand], outs: &[RegId]) -> Result<(), CompileError> {
        let function = &self.env.module.functions[target];
        let types = self.env.types;
        let clif = *self.env.funcs.get(&target).ok_or_else(|| CompileError::internal("a call to a function that was not compiled"))?;
        let ret = types.id(function.ret);
        let mut values = vec![self.context];
        let mut result_at = None;
        if matches!(passing(&types.table, ret), Passing::Memory) {
            let at = self.temp(types.table.layout(ret).size);
            values.push(self.address(at));
            result_at = Some(at);
        }
        if self.env.closure_conv.contains(&target) {
            if !function.captures.is_empty() {
                return Err(CompileError::internal("a direct call of a function that captures"));
            }
            values.push(self.b.ins().iconst(ir::types::I64, 0));
        }
        for (arg, &param) in args.iter().zip(&function.params) {
            let id = types.id(function.regs[param].ty);
            match passing(&types.table, id) {
                Passing::Registers(_) => values.extend(self.operand_leaves(arg, id)?),
                Passing::Memory => {
                    let at = self.temp(types.table.layout(id).size);
                    self.write(at, id, arg)?;
                    values.push(self.address(at));
                }
            }
        }
        let area = if function.out_params.is_empty() {
            None
        } else {
            let (places, total) = self.out_places(function);
            let at = self.temp(total);
            values.push(self.address(at));
            Some((at, places))
        };
        let fref = self.func_ref(clif);
        let call = self.b.ins().call(fref, &values);
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
