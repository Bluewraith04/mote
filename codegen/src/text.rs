//! Lowers the `string_*` and `parse_*` intrinsics.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, InstBuilder, Type, Value};
use mote_abi::{FaultKind, LENGTH_OFFSET};
use mote_ir::{Inst, Operand};
use mote_semantics::TyId;

use crate::function::Lowering;
use mote_runtime::helpers::text;
use crate::scalar::Num;
use crate::values::{internal, Addr};
use crate::CompileError;

/// The operands and type ids a call of `mote_text` is given.
#[derive(Default)]
struct Call {
    a: Option<Value>,
    b: Option<Value>,
    c: Option<Value>,
    types: [u32; 3],
}

impl Lowering<'_, '_, '_> {
    pub(crate) fn text_intrinsic(&mut self, name: &str, inst: &Inst, args: &[Operand]) -> Result<(), CompileError> {
        let string = self.env.types.id(TyId::STRING).raw();
        match (name, args) {
            ("string_len", [s]) => {
                let s = self.num_operand(s, Num::Ref)?;
                let len = self.load(types::I64, Addr { base: s, offset: LENGTH_OFFSET as i32 });
                self.set(inst.dest, Some(len));
            }
            ("string_concat", [a, b]) => {
                let joined = self.concat(&[*a, *b])?;
                self.set(inst.dest, Some(joined));
            }
            ("string_slice", [s, start, end]) => self.string_slice(inst, s, start, end)?,
            ("string_byte_at", [s, index]) => {
                let string = self.num_operand(s, Num::Ref)?;
                let len = self.load(types::I64, Addr { base: string, offset: LENGTH_OFFSET as i32 });
                let at = self.index_value(index)?;
                let outside = self.b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, at, len);
                self.fault_if(outside, FaultKind::IndexOutOfRange, inst.span);
                let base = self.b.ins().iadd_imm(string, i64::from(mote_abi::ELEMENTS_OFFSET));
                let ptr = self.b.ins().iadd(base, at);
                let byte = self.load(types::I8, Addr { base: ptr, offset: 0 });
                self.set(inst.dest, Some(byte));
            }
            ("string_is_boundary", [s, index]) => {
                let string = self.num_operand(s, Num::Ref)?;
                let at = self.index_value(index)?;
                let cut = self.func_ref(self.env.imports.cut);
                let call = self.b.ins().call(cut, &[string, at, at]);
                let whole = self.b.inst_results(call)[0];
                let flag = self.b.ins().ireduce(types::I8, whole);
                self.set(inst.dest, Some(flag));
            }
            ("string_replace", [s, target, replacement]) => {
                let call = Call { a: Some(self.num_operand(s, Num::Ref)?), b: Some(self.num_operand(target, Num::Ref)?), c: Some(self.num_operand(replacement, Num::Ref)?), types: [string, 0, 0] };
                self.text_string(inst, text::REPLACE, call);
            }
            ("string_to_upper", [s]) => self.text_unary(inst, text::UPPER, s, string)?,
            ("string_to_lower", [s]) => self.text_unary(inst, text::LOWER, s, string)?,
            ("string_trim", [s]) => self.text_unary(inst, text::TRIM, s, string)?,
            ("string_repeat", [s, count]) => {
                let count = self.index_value(count)?;
                let negative = self.b.ins().icmp_imm(IntCC::SignedLessThan, count, 0);
                self.fault_if(negative, FaultKind::IndexOutOfRange, inst.span);
                let call = Call { a: Some(self.num_operand(s, Num::Ref)?), b: Some(count), c: None, types: [string, 0, 0] };
                self.text_string(inst, text::REPEAT, call);
            }
            ("string_find" | "string_rfind", [s, needle]) => {
                let call = Call { a: Some(self.num_operand(s, Num::Ref)?), b: Some(self.num_operand(needle, Num::Ref)?), ..Call::default() };
                let found = self.text_call(text::FIND + u32::from(name == "string_rfind"), call);
                let none = self.b.ins().icmp_imm(IntCC::Equal, found, -1);
                let some = self.b.ins().bxor_imm(none, 1);
                let zero = self.b.ins().iconst(types::I64, 0);
                let payload = self.b.ins().select(none, zero, found);
                self.optional_number(inst, some, payload)?;
            }
            ("string_split", [s, separator]) => {
                let list = self.reg_type(Self::dest_reg(inst)?);
                let buffer = self.env.types.buffer(list);
                let call = Call { a: Some(self.num_operand(s, Num::Ref)?), b: Some(self.num_operand(separator, Num::Ref)?), c: None, types: [list.raw(), buffer.raw(), string] };
                self.text_string(inst, text::SPLIT, call);
            }
            ("string_bytes", [s]) => {
                let array = self.reg_type(Self::dest_reg(inst)?);
                let call = Call { a: Some(self.num_operand(s, Num::Ref)?), c: None, types: [array.raw(), 0, 0], b: None };
                self.text_string(inst, text::BYTES, call);
            }
            ("string_from_bytes", [bytes]) => self.bytes_to_string(inst, bytes, string)?,
            ("parse_int", [s]) => self.parse(inst, text::PARSE_INT, s, types::I64)?,
            ("parse_float", [s]) => self.parse(inst, text::PARSE_FLOAT, s, types::F64)?,
            _ => return Err(CompileError::internal(format!("the intrinsic `{name}` with these arguments"))),
        }
        Ok(())
    }

    /// A call of `mote_text`; the operands it is not given are 0.
    fn text_call(&mut self, op: u32, call: Call) -> Value {
        let zero = self.b.ins().iconst(types::I64, 0);
        let op = self.b.ins().iconst(types::I32, i64::from(op));
        let [t1, t2, t3] = call.types.map(|id| self.b.ins().iconst(types::I32, i64::from(id)));
        let text = self.func_ref(self.env.imports.text);
        let result = self.b.ins().call(text, &[self.context, op, call.a.unwrap_or(zero), call.b.unwrap_or(zero), call.c.unwrap_or(zero), t1, t2, t3]);
        self.b.inst_results(result)[0]
    }

    /// A call of `mote_text` whose result is an object, which is a `MemoryLimit` fault when it is 0.
    fn text_string(&mut self, inst: &Inst, op: u32, call: Call) {
        let object = self.text_call(op, call);
        let full = self.b.ins().icmp_imm(IntCC::Equal, object, 0);
        self.fault_if(full, FaultKind::MemoryLimit, inst.span);
        self.set(inst.dest, Some(object));
    }

    fn text_unary(&mut self, inst: &Inst, op: u32, s: &Operand, string: u32) -> Result<(), CompileError> {
        let call = Call { a: Some(self.num_operand(s, Num::Ref)?), types: [string, 0, 0], ..Call::default() };
        self.text_string(inst, op, call);
        Ok(())
    }

    /// `s[start..end]`: `IndexOutOfRange` for a range outside the string or inside a character.
    fn string_slice(&mut self, inst: &Inst, s: &Operand, start: &Operand, end: &Operand) -> Result<(), CompileError> {
        let string = self.num_operand(s, Num::Ref)?;
        let len = self.load(types::I64, Addr { base: string, offset: LENGTH_OFFSET as i32 });
        let (start, end) = (self.index_value(start)?, self.index_value(end)?);
        let negative = self.b.ins().icmp_imm(IntCC::SignedLessThan, start, 0);
        let inverted = self.b.ins().icmp(IntCC::SignedLessThan, end, start);
        let past = self.b.ins().icmp(IntCC::SignedGreaterThan, end, len);
        let outside = self.b.ins().bor(negative, inverted);
        let outside = self.b.ins().bor(outside, past);
        self.fault_if(outside, FaultKind::IndexOutOfRange, inst.span);
        let cut = self.func_ref(self.env.imports.cut);
        let call = self.b.ins().call(cut, &[string, start, end]);
        let whole = self.b.inst_results(call)[0];
        let broken = self.b.ins().icmp_imm(IntCC::Equal, whole, 0);
        self.fault_if(broken, FaultKind::IndexOutOfRange, inst.span);
        let count = self.b.ins().isub(end, start);
        let piece = self.alloc_array(self.env.types.id(TyId::STRING), count);
        let from = self.b.ins().iadd(string, start);
        let from = self.b.ins().iadd_imm(from, i64::from(mote_abi::ELEMENTS_OFFSET));
        let to = self.b.ins().iadd_imm(piece, i64::from(mote_abi::ELEMENTS_OFFSET));
        self.copy_bytes(to, from, count);
        self.set(inst.dest, Some(piece));
        Ok(())
    }

    /// `String?` of the bytes, which is `null` when they are not UTF-8.
    fn bytes_to_string(&mut self, inst: &Inst, bytes: &Operand, string: u32) -> Result<(), CompileError> {
        let bytes = self.num_operand(bytes, Num::Ref)?;
        let valid = self.text_call(text::VALID, Call { a: Some(bytes), ..Call::default() });
        let make = self.b.create_block();
        let join = self.b.create_block();
        self.b.append_block_param(join, types::I64);
        let null = self.b.ins().iconst(types::I64, 0);
        self.b.ins().brif(valid, make, &[], join, &[null]);
        self.b.switch_to_block(make);
        let made = self.text_call(text::FROM_BYTES, Call { a: Some(bytes), types: [string, 0, 0], ..Call::default() });
        let full = self.b.ins().icmp_imm(IntCC::Equal, made, 0);
        self.fault_if(full, FaultKind::MemoryLimit, inst.span);
        self.b.ins().jump(join, &[made]);
        self.b.switch_to_block(join);
        let result = self.b.block_params(join)[0];
        self.set(inst.dest, Some(result));
        Ok(())
    }

    /// `parse_int` and `parse_float`: the number, or none when the text is not one.
    fn parse(&mut self, inst: &Inst, op: u32, s: &Operand, ty: Type) -> Result<(), CompileError> {
        let slot = self.temp(8);
        let zero = self.b.ins().iconst(types::I64, 0);
        self.store(zero, slot);
        let out = self.address(slot);
        let call = Call { a: Some(self.num_operand(s, Num::Ref)?), c: Some(out), ..Call::default() };
        let parsed = self.text_call(op, call);
        let some = self.b.ins().ireduce(types::I8, parsed);
        let payload = self.load(ty, slot);
        self.optional_number(inst, some, payload)
    }

    /// Writes an optional number into the destination: the tag word and the payload.
    fn optional_number(&mut self, inst: &Inst, some: Value, payload: Value) -> Result<(), CompileError> {
        let dest = inst.dest.ok_or_else(|| internal("an optional number with no destination"))?;
        let info = self.env.types.table.get(self.reg_type(dest));
        let field = *info.variants.get(1).and_then(|variant| variant.fields.first()).ok_or_else(|| internal("a result that is not an optional number"))?;
        let at = self.reg_addr(dest)?;
        let tag = self.b.ins().uextend(types::I64, some);
        self.store(tag, at);
        self.store(payload, Addr { base: at.base, offset: at.offset + field.offset as i32 });
        Ok(())
    }
}
