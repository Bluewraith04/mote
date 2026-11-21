//! Lowers the reflection intrinsics: the `std.meta` tables, `Dyn` and `require`.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, InstBuilder, Value};
use mote_abi::{FaultKind, BOX_VALUE_OFFSET};
use mote_base::Span;
use mote_ir::{Inst, Operand};
use mote_semantics::TyId;

use mote_runtime::dynamic::{op, FULL, RANGE};
use crate::function::Lowering;
use crate::scalar::Num;
use crate::values::{internal, Addr};
use crate::CompileError;

/// An `Int`.
const INT: Num = Num::Int { bits: 64, signed: true };

impl Lowering<'_, '_, '_> {
    pub(crate) fn reflect_intrinsic(&mut self, name: &str, inst: &Inst, args: &[Operand]) -> Result<(), CompileError> {
        match (name, args) {
            ("require", [condition, message]) => {
                let held = self.operand(condition, TyId::BOOL)?.expect("a condition has a value");
                let message = self.num_operand(message, Num::Ref)?;
                let fail = self.b.create_block();
                let go = self.b.create_block();
                self.b.set_cold_block(fail);
                self.b.ins().brif(held, go, &[], fail, &[]);
                self.b.switch_to_block(fail);
                let fault = self.func_ref(self.env.imports.fault_message);
                self.b.ins().call(fault, &[self.context, message]);
                self.b.ins().trap(cranelift_codegen::ir::TrapCode::unwrap_user(1));
                self.b.switch_to_block(go);
            }
            ("meta_ints" | "meta_texts", []) => {
                let data = *self.env.meta_tables.get(name).ok_or_else(|| internal("a meta table that was not made"))?;
                let global = self.env.clif.declare_data_in_func(data, self.b.func);
                let table = self.b.ins().symbol_value(types::I64, global);
                self.set(inst.dest, Some(table));
            }
            ("dyn_of", [value, number]) => {
                let ty = self.ty_of(value);
                let id = self.env.types.id(ty);
                let boxed = self.env.types.try_boxed(ty).ok_or_else(|| internal("a Dyn of a type with no box"))?;
                let number = self.num_operand(number, INT)?;
                let held = self.alloc(boxed);
                self.write(Addr { base: held, offset: BOX_VALUE_OFFSET as i32 }, id, value)?;
                let dynamic = self.alloc(self.dyn_id()?);
                let (number_at, box_at) = (self.dyn_field(0)?, self.dyn_field(1)?);
                self.store(number, Addr { base: dynamic, offset: number_at });
                self.store(held, Addr { base: dynamic, offset: box_at });
                self.set(inst.dest, Some(dynamic));
            }
            ("dyn_type", [dynamic]) => {
                let dynamic = self.num_operand(dynamic, Num::Ref)?;
                let at = self.dyn_field(0)?;
                let number = self.load(types::I64, Addr { base: dynamic, offset: at });
                self.set(inst.dest, Some(number));
            }
            ("dyn_to", [dynamic, number]) => self.dyn_to(inst, dynamic, number)?,
            ("dyn_len" | "dyn_tag", [dynamic]) => {
                let dynamic = self.num_operand(dynamic, Num::Ref)?;
                let zero = self.b.ins().iconst(types::I64, 0);
                let code = if name == "dyn_len" { op::LEN } else { op::TAG };
                let result = self.dyn_call(code, [dynamic, zero, zero, zero]);
                self.set(inst.dest, Some(result));
            }
            ("dyn_get", [dynamic, at]) => {
                let (dynamic, at) = (self.num_operand(dynamic, Num::Ref)?, self.num_operand(at, INT)?);
                let zero = self.b.ins().iconst(types::I64, 0);
                let result = self.dyn_call(op::GET, [dynamic, at, zero, zero]);
                self.dyn_faults(result, inst.span);
                self.set(inst.dest, Some(result));
            }
            ("dyn_set", [dynamic, at, part]) => {
                let (dynamic, at, part) = (self.num_operand(dynamic, Num::Ref)?, self.num_operand(at, INT)?, self.num_operand(part, Num::Ref)?);
                let zero = self.b.ins().iconst(types::I64, 0);
                let result = self.dyn_call(op::SET, [dynamic, at, part, zero]);
                self.dyn_faults(result, inst.span);
            }
            ("dyn_show", [dynamic, top]) => {
                let dynamic = self.num_operand(dynamic, Num::Ref)?;
                let top = self.operand(top, TyId::BOOL)?.expect("a Bool has a value");
                let top = self.b.ins().uextend(types::I64, top);
                let zero = self.b.ins().iconst(types::I64, 0);
                let result = self.dyn_call(op::SHOW, [dynamic, top, zero, zero]);
                self.dyn_faults(result, inst.span);
                self.set(inst.dest, Some(result));
            }
            ("dyn_build", [number, tag, parts]) => {
                let dest = Self::dest_reg(inst)?;
                let id = self.reg_type(dest);
                let (number, tag, parts) = (self.num_operand(number, INT)?, self.num_operand(tag, INT)?, self.num_operand(parts, Num::Ref)?);
                let room = self.temp(self.env.types.table.layout(id).size.max(8));
                let out = self.address(room);
                let result = self.dyn_call(op::BUILD, [number, tag, parts, out]);
                self.dyn_faults(result, inst.span);
                self.read_into(inst.dest, room, id)?;
            }
            ("dyn_make", [number, tag, parts]) => {
                let (number, tag, parts) = (self.num_operand(number, INT)?, self.num_operand(tag, INT)?, self.num_operand(parts, Num::Ref)?);
                let zero = self.b.ins().iconst(types::I64, 0);
                let result = self.dyn_call(op::MAKE, [number, tag, parts, zero]);
                self.dyn_faults(result, inst.span);
                self.set(inst.dest, Some(result));
            }
            _ => return Err(CompileError::internal(format!("the intrinsic `{name}` with these arguments"))),
        }
        Ok(())
    }

    fn dyn_id(&self) -> Result<mote_abi::TypeId, CompileError> {
        self.env.types.dyn_type().ok_or_else(|| internal("a program with no Dyn"))
    }

    /// The offset of a field of a `Dyn`: 0 is the type number, 1 the box.
    fn dyn_field(&self, index: usize) -> Result<i32, CompileError> {
        let info = self.env.types.table.get(self.dyn_id()?);
        Ok(info.object.as_ref().ok_or_else(|| internal("a Dyn that is not a class"))?.fields[index].offset as i32)
    }

    /// A call of `mote_dyn`.
    fn dyn_call(&mut self, code: u32, [a, b, c, d]: [Value; 4]) -> Value {
        let code = self.b.ins().iconst(types::I32, i64::from(code));
        let call = self.func_ref(self.env.imports.dynamic);
        let result = self.b.ins().call(call, &[self.context, code, a, b, c, d]);
        self.b.inst_results(result)[0]
    }

    /// Faults for the two results of `mote_dyn` that say why it could not give a value.
    fn dyn_faults(&mut self, result: Value, span: Span) {
        let range = self.b.ins().icmp_imm(IntCC::Equal, result, RANGE as i64);
        self.fault_if(range, FaultKind::IndexOutOfRange, span);
        let full = self.b.ins().icmp_imm(IntCC::Equal, result, FULL as i64);
        self.fault_if(full, FaultKind::MemoryLimit, span);
    }

    /// `dyn_to`: the value a `Dyn` holds when it holds the type number, else none.
    fn dyn_to(&mut self, inst: &Inst, dynamic: &Operand, number: &Operand) -> Result<(), CompileError> {
        let dest = Self::dest_reg(inst)?;
        let (dynamic, number) = (self.num_operand(dynamic, Num::Ref)?, self.num_operand(number, INT)?);
        let (number_at, box_at) = (self.dyn_field(0)?, self.dyn_field(1)?);
        let held = self.load(types::I64, Addr { base: dynamic, offset: number_at });
        let boxed = self.load(types::I64, Addr { base: dynamic, offset: box_at });
        let same = self.b.ins().icmp(IntCC::Equal, held, number);
        let info = self.env.types.table.get(self.reg_type(dest));
        let tagged = if info.is_reference() {
            None
        } else {
            let field = *info.variants.get(1).and_then(|variant| variant.fields.first()).ok_or_else(|| internal("a dyn_to result that is not an optional"))?;
            Some((field, self.reg_addr(dest)?))
        };
        let some = self.b.create_block();
        let none = self.b.create_block();
        let join = self.b.create_block();
        self.b.ins().brif(same, some, &[], none, &[]);

        self.b.switch_to_block(some);
        let from = Addr { base: boxed, offset: BOX_VALUE_OFFSET as i32 };
        if let Some((field, at)) = tagged {
            let present = self.b.ins().iconst(types::I64, 1);
            self.store(present, at);
            self.copy_mem(at.at(field.offset), from, self.env.types.table.layout(field.ty).size);
            self.b.ins().jump(join, &[]);
            self.b.switch_to_block(none);
            let absent = self.b.ins().iconst(types::I64, 0);
            self.store(absent, at);
        } else {
            let value = self.load(types::I64, from);
            self.set(inst.dest, Some(value));
            self.b.ins().jump(join, &[]);
            self.b.switch_to_block(none);
            let null = self.b.ins().iconst(types::I64, 0);
            self.set(inst.dest, Some(null));
        }
        self.b.ins().jump(join, &[]);
        self.b.switch_to_block(join);
        Ok(())
    }
}
