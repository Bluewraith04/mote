//! Lowers the element, copy and equality intrinsics of lists and arrays, `fault` and `typeof`.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, InstBuilder, Value};
use mote_abi::{FaultKind, Kind, ELEMENTS_OFFSET, LENGTH_OFFSET, LIST_BUFFER_OFFSET};
use mote_base::Span;
use mote_ir::{Inst, Operand, RegId};
use mote_semantics::{BinaryOp, TyKind};

use crate::function::Lowering;
use mote_runtime::helpers::value;
use crate::scalar::Num;
use crate::values::{internal, Addr};
use crate::CompileError;

impl Lowering<'_, '_, '_> {
    /// The destination gets a copy of everything the operand reaches through classes, boxes, lists and arrays.
    pub(crate) fn deep_clone(&mut self, inst: &Inst, original: &Operand) -> Result<(), CompileError> {
        let id = self.env.types.id(self.ty_of(original));
        let from = self.spill_as(original, id)?;
        let size = self.env.types.table.layout(id).size.max(8);
        let to = self.temp(size);
        let (source, target) = (self.address(from), self.address(to));
        let done = self.value_call(value::DEEP_CLONE, id.raw(), source, target);
        let full = self.b.ins().icmp_imm(IntCC::Equal, done, 0);
        self.fault_if(full, FaultKind::MemoryLimit, inst.span);
        self.read_into(inst.dest, to, id)
    }

    pub(crate) fn collection_intrinsic(&mut self, name: &str, inst: &Inst, args: &[Operand], outs: &[RegId]) -> Result<(), CompileError> {
        match (name, args) {
            ("array_len", [array]) => {
                let len = self.length(array, &|| CompileError::internal("array_len of this value"))?;
                self.set(inst.dest, Some(len));
            }
            ("list_get" | "array_get", [sequence, index]) => {
                let (at, element) = self.sequence_element(sequence, index, inst.span)?;
                self.read_into(inst.dest, at, element)?;
            }
            ("list_set" | "array_set", [sequence, index, item]) => {
                let (at, element) = self.sequence_element(sequence, index, inst.span)?;
                self.write(at, element, item)?;
                self.keep(sequence, outs)?;
            }
            ("list_clear", [list]) => {
                let object = self.num_operand(list, Num::Ref)?;
                let zero = self.b.ins().iconst(types::I64, 0);
                self.store(zero, Addr { base: object, offset: LENGTH_OFFSET as i32 });
                self.keep(list, outs)?;
            }
            ("list_pop", [list]) => {
                self.pop(inst, list)?;
                self.keep(list, outs)?;
            }
            ("list_slice" | "array_slice", [sequence, start, end]) => self.slice(inst, sequence, (&Some(*start), &Some(*end), false), None)?,
            ("list_clone" | "array_clone", [sequence]) => self.slice(inst, sequence, (&None, &None, false), None)?,
            ("list_set_slice", [list, start, end, values]) => {
                self.set_slice(inst, list, (&Some(*start), &Some(*end), false), values)?;
                self.keep(list, outs)?;
            }
            ("list_contains" | "array_contains", [sequence, item]) => {
                let container = self.operand_ty(&[sequence]);
                let id = self.env.types.id(container);
                let (Kind::List(element) | Kind::Array(element)) = self.env.types.table.get(id).kind else { return Err(CompileError::internal("contains of this value")) };
                let object = self.num_operand(sequence, Num::Ref)?;
                let at = self.spill_as(item, element)?;
                let at = self.address(at);
                let found = self.value_call(value::CONTAINS, id.raw(), object, at);
                let found = self.b.ins().ireduce(types::I8, found);
                self.set(inst.dest, Some(found));
            }
            ("equal_of" | "key_eq", [a, b]) => {
                let ty = self.operand_ty(&[a, b]);
                let id = self.env.types.id(ty);
                let (left, right) = (self.spill_as(a, id)?, self.spill_as(b, id)?);
                let (left, right) = (self.address(left), self.address(right));
                let same = self.value_call(if name == "key_eq" { value::KEY_EQ } else { value::EQUAL }, id.raw(), left, right);
                let same = self.b.ins().ireduce(types::I8, same);
                self.set(inst.dest, Some(same));
            }
            ("hash_of", [key]) => {
                let id = self.env.types.id(self.ty_of(key));
                let at = self.spill_as(key, id)?;
                let at = self.address(at);
                let zero = self.b.ins().iconst(types::I64, 0);
                let hash = self.value_call(value::HASH, id.raw(), at, zero);
                self.set(inst.dest, Some(hash));
            }
            ("deep_clone", [original]) => self.deep_clone(inst, original)?,
            ("typeof", [value]) => {
                let ty = self.ty_of(value);
                if matches!(self.env.program.types.kind(ty), TyKind::Union(_)) {
                    let name = self.union_type_name(value)?;
                    self.set(inst.dest, Some(name));
                    return Ok(());
                }
                let data = *self.env.type_names.get(&ty).ok_or_else(|| internal("a typeof with no name"))?;
                let global = self.env.clif.declare_data_in_func(data, self.b.func);
                let name = self.b.ins().symbol_value(types::I64, global);
                self.set(inst.dest, Some(name));
            }
            ("fault", [message]) => {
                let message = self.num_operand(message, Num::Ref)?;
                let fault = self.func_ref(self.env.imports.fault_message);
                self.b.ins().call(fault, &[self.context, message]);
                self.b.ins().trap(cranelift_codegen::ir::TrapCode::unwrap_user(1));
                let after = self.b.create_block();
                self.b.switch_to_block(after);
            }
            _ => return Err(CompileError::internal(format!("the intrinsic `{name}` with these arguments"))),
        }
        Ok(())
    }

    /// The object of a `var` argument, which the call leaves as it was, for the register that receives its final value.
    fn keep(&mut self, object: &Operand, outs: &[RegId]) -> Result<(), CompileError> {
        for &out in outs {
            let object = self.num_operand(object, Num::Ref)?;
            self.set(Some(out), Some(object));
        }
        Ok(())
    }

    /// The operand written to a stack slot as a value of the type.
    pub(crate) fn spill_as(&mut self, operand: &Operand, id: mote_abi::TypeId) -> Result<Addr, CompileError> {
        let size = self.env.types.table.layout(id).size.max(8);
        let at = self.temp(size);
        self.write(at, id, operand)?;
        Ok(at)
    }

    /// `==` and `!=` of values that are not numbers: structs, enums, tuples, optionals, unions, lists and arrays by contents.
    pub(crate) fn value_equality(&mut self, op: BinaryOp, left: &Operand, right: &Operand) -> Result<Option<Value>, CompileError> {
        if !matches!(op, BinaryOp::Eq | BinaryOp::NotEq) {
            return Ok(None);
        }
        let ty = self.ty_of(left);
        if ty != self.ty_of(right) || matches!(self.env.program.types.kind(ty), TyKind::Unit | TyKind::Bool | TyKind::Int(_) | TyKind::Float(_) | TyKind::String) {
            return Ok(None);
        }
        let id = self.env.types.id(ty);
        let (a, b) = (self.spill_as(left, id)?, self.spill_as(right, id)?);
        let (a, b) = (self.address(a), self.address(b));
        let same = self.value_call(value::EQUAL, id.raw(), a, b);
        let same = self.b.ins().ireduce(types::I8, same);
        Ok(Some(if op == BinaryOp::Eq { same } else { self.b.ins().bxor_imm(same, 1) }))
    }

    /// Whether `print` and `show` take the operand as a number, `Bool` or string.
    pub(crate) fn is_plain(&self, operand: &Operand) -> bool {
        matches!(self.env.program.types.kind(self.ty_of(operand)), TyKind::Bool | TyKind::Int(_) | TyKind::Float(_) | TyKind::String)
    }

    /// `print` or `println` of any value, written by the type table; the flags are those of `mote_print`.
    pub(crate) fn print_value(&mut self, operand: &Operand, flags: u32) -> Result<(), CompileError> {
        let id = self.env.types.id(self.ty_of(operand));
        let at = self.spill_as(operand, id)?;
        let (at, flag) = (self.address(at), self.b.ins().iconst(types::I64, i64::from(flags)));
        self.value_call(value::PRINT, id.raw(), at, flag);
        Ok(())
    }

    /// A new string of the text of any value; a full heap faults.
    pub(crate) fn show_value(&mut self, operand: &Operand, span: Span) -> Result<Value, CompileError> {
        let id = self.env.types.id(self.ty_of(operand));
        let at = self.spill_as(operand, id)?;
        let (at, unused) = (self.address(at), self.b.ins().iconst(types::I64, 0));
        let text = self.value_call(value::SHOW, id.raw(), at, unused);
        let full = self.b.ins().icmp_imm(IntCC::Equal, text, 0);
        self.fault_if(full, FaultKind::MemoryLimit, span);
        Ok(text)
    }

    /// A call of `mote_value`.
    fn value_call(&mut self, op: u32, ty: u32, a: Value, b: Value) -> Value {
        let op = self.b.ins().iconst(types::I32, i64::from(op));
        let ty = self.b.ins().iconst(types::I32, i64::from(ty));
        let call = self.func_ref(self.env.imports.value);
        let result = self.b.ins().call(call, &[self.context, op, ty, a, b]);
        self.b.inst_results(result)[0]
    }

    /// `list.pop()`: the last element and a shorter list, or none for an empty one.
    fn pop(&mut self, inst: &Inst, list: &Operand) -> Result<(), CompileError> {
        let dest = Self::dest_reg(inst)?;
        let id = self.env.types.id(self.ty_of(list));
        let Kind::List(element) = self.env.types.table.get(id).kind else { return Err(CompileError::internal("list_pop of this value")) };
        let stride = self.stride(element);
        let object = self.num_operand(list, Num::Ref)?;
        let len = self.load(types::I64, Addr { base: object, offset: LENGTH_OFFSET as i32 });
        let info = self.env.types.table.get(self.reg_type(dest));
        let held = if info.is_reference() {
            None
        } else {
            let field = *info.variants.get(1).and_then(|variant| variant.fields.first()).ok_or_else(|| internal("a pop result that is not an optional"))?;
            Some((field, self.reg_addr(dest)?))
        };
        let empty = self.b.ins().icmp_imm(IntCC::Equal, len, 0);
        let take = self.b.create_block();
        let none = self.b.create_block();
        let join = self.b.create_block();
        self.b.ins().brif(empty, none, &[], take, &[]);

        self.b.switch_to_block(take);
        let last = self.b.ins().iadd_imm(len, -1);
        self.store(last, Addr { base: object, offset: LENGTH_OFFSET as i32 });
        let buffer = self.load(types::I64, Addr { base: object, offset: LIST_BUFFER_OFFSET as i32 });
        let by = self.b.ins().imul_imm(last, i64::from(stride));
        let first = self.b.ins().iadd(buffer, by);
        let from = Addr { base: first, offset: ELEMENTS_OFFSET as i32 };
        if let Some((field, at)) = held {
            let some = self.b.ins().iconst(types::I64, 1);
            self.store(some, at);
            self.copy_mem(at.at(field.offset), from, self.env.types.table.layout(element).size);
            self.b.ins().jump(join, &[]);
            self.b.switch_to_block(none);
            let absent = self.b.ins().iconst(types::I64, 0);
            self.store(absent, at);
        } else {
            let popped = self.load(types::I64, from);
            self.set(inst.dest, Some(popped));
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
