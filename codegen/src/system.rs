//! Lowers the `sys_*` intrinsics: the program's input, arguments, end and clock.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, InstBuilder, Value};
use mote_abi::FaultKind;
use mote_ir::{Inst, Operand};
use mote_runtime::helpers::system;
use mote_semantics::TyId;

use crate::function::Lowering;
use crate::CompileError;

impl Lowering<'_, '_, '_> {
    pub(crate) fn system_intrinsic(&mut self, name: &str, inst: &Inst, args: &[Operand]) -> Result<(), CompileError> {
        let string = self.env.types.id(TyId::STRING).raw();
        match (name, args) {
            ("sys_read_line", []) => {
                let line = self.system_call(system::READ_LINE, None, [string, 0, 0]);
                let full = self.b.ins().icmp_imm(IntCC::Equal, line, 0);
                self.fault_if(full, FaultKind::MemoryLimit, inst.span);
                let end = self.b.ins().icmp_imm(IntCC::Equal, line, -1);
                let none = self.b.ins().iconst(types::I64, 0);
                let result = self.b.ins().select(end, none, line);
                self.set(inst.dest, Some(result));
            }
            ("sys_args", []) => {
                let list = self.reg_type(Self::dest_reg(inst)?);
                let buffer = self.env.types.buffer(list);
                let made = self.system_call(system::ARGS, None, [list.raw(), buffer.raw(), string]);
                let full = self.b.ins().icmp_imm(IntCC::Equal, made, 0);
                self.fault_if(full, FaultKind::MemoryLimit, inst.span);
                self.set(inst.dest, Some(made));
            }
            ("sys_exit", [code]) => {
                let code = self.index_value(code)?;
                self.system_call(system::EXIT, Some(code), [0, 0, 0]);
            }
            ("sys_now", []) => {
                let now = self.system_call(system::NOW, None, [0, 0, 0]);
                self.set(inst.dest, Some(now));
            }
            _ => return Err(CompileError::internal(format!("the intrinsic `{name}` with these arguments"))),
        }
        Ok(())
    }

    /// A call of `mote_system`; an operand it is not given is 0.
    fn system_call(&mut self, op: u32, a: Option<Value>, ids: [u32; 3]) -> Value {
        let a = a.unwrap_or_else(|| self.b.ins().iconst(types::I64, 0));
        let op = self.b.ins().iconst(types::I32, i64::from(op));
        let [t1, t2, t3] = ids.map(|id| self.b.ins().iconst(types::I32, i64::from(id)));
        let call = self.func_ref(self.env.imports.system);
        let result = self.b.ins().call(call, &[self.context, op, a, t1, t2, t3]);
        self.b.inst_results(result)[0]
    }
}
