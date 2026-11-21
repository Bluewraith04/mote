//! Lowers arrays, lists, slices and the growth of a list.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, InstBuilder, Value};
use mote_abi::{FaultKind, Kind, TypeId, ELEMENTS_OFFSET, LENGTH_OFFSET, LIST_BUFFER_OFFSET, LIST_CAPACITY_OFFSET};
use mote_base::Span;
use mote_ir::{Const, Inst, Operand, RegId};
use mote_semantics::TyId;

use crate::function::Lowering;
use crate::scalar::Num;
use crate::values::{internal, Addr};
use crate::CompileError;

/// The most elements `[x; n]` makes, which is 64 MiB in words.
const REPEAT_LIMIT: i64 = 64 * 1024 * 1024 / 8;

/// The elements a list gets when its first element is added.
const FIRST_CAPACITY: i64 = 4;

/// The bounds of a slice: start, end and whether the end is included.
pub(crate) type Bounds<'a> = (&'a Option<Operand>, &'a Option<Operand>, bool);

/// Where the elements of a sequence are and how many there are.
struct Sequence {
    first: Value,
    len: Value,
    stride: u32,
    element: Option<TypeId>,
}

impl Lowering<'_, '_, '_> {
    fn sequence(&mut self, operand: &Operand) -> Result<Sequence, CompileError> {
        let id = match operand {
            Operand::Reg(reg) => self.reg_type(*reg),
            Operand::Const(Const::String(_)) => self.env.types.id(TyId::STRING),
            Operand::Const(_) => return Err(internal("a constant where a sequence is read")),
        };
        match self.env.types.table.get(id).kind {
            Kind::FixedArray(element, count) => {
                let Operand::Reg(reg) = operand else { return Err(internal("a constant fixed array")) };
                let first = if self.repr_of(id)? == crate::values::Repr::Nothing {
                    self.b.ins().iconst(types::I64, 0)
                } else {
                    let at = self.reg_addr(*reg)?;
                    self.address(at)
                };
                let len = self.b.ins().iconst(types::I64, i64::from(count));
                Ok(Sequence { first, len, stride: self.stride(element), element: Some(element) })
            }
            Kind::Array(element) => {
                let object = self.num_operand(operand, Num::Ref)?;
                let len = self.load(types::I64, Addr { base: object, offset: LENGTH_OFFSET as i32 });
                let first = self.b.ins().iadd_imm(object, i64::from(ELEMENTS_OFFSET));
                Ok(Sequence { first, len, stride: self.stride(element), element: Some(element) })
            }
            Kind::String => {
                let object = self.num_operand(operand, Num::Ref)?;
                let len = self.load(types::I64, Addr { base: object, offset: LENGTH_OFFSET as i32 });
                let first = self.b.ins().iadd_imm(object, i64::from(ELEMENTS_OFFSET));
                Ok(Sequence { first, len, stride: 1, element: None })
            }
            Kind::List(element) => {
                let list = self.num_operand(operand, Num::Ref)?;
                let len = self.load(types::I64, Addr { base: list, offset: LENGTH_OFFSET as i32 });
                let buffer = self.load(types::I64, Addr { base: list, offset: LIST_BUFFER_OFFSET as i32 });
                let first = self.b.ins().iadd_imm(buffer, i64::from(ELEMENTS_OFFSET));
                Ok(Sequence { first, len, stride: self.stride(element), element: Some(element) })
            }
            _ => Err(CompileError::internal("this sequence")),
        }
    }

    /// A new array, string or list of `count` zeroed elements: the object, and the address of its first element.
    fn new_sequence(&mut self, id: TypeId, count: Value) -> Result<(Value, Value), CompileError> {
        match self.env.types.table.get(id).kind {
            Kind::Array(_) | Kind::String => {
                let object = self.alloc_array(id, count);
                Ok((object, self.b.ins().iadd_imm(object, i64::from(ELEMENTS_OFFSET))))
            }
            Kind::List(_) => {
                let buffer = self.alloc_array(self.env.types.buffer(id), count);
                let list = self.alloc(id);
                self.store_reference(buffer, Addr { base: list, offset: LIST_BUFFER_OFFSET as i32 });
                self.store(count, Addr { base: list, offset: LENGTH_OFFSET as i32 });
                self.store(count, Addr { base: list, offset: LIST_CAPACITY_OFFSET as i32 });
                Ok((list, self.b.ins().iadd_imm(buffer, i64::from(ELEMENTS_OFFSET))))
            }
            _ => Err(internal("a new sequence that is not an array, string or list")),
        }
    }

    /// `[a, b, c]` as an array.
    pub(crate) fn array_literal(&mut self, inst: &Inst, items: &[Operand]) -> Result<(), CompileError> {
        let id = self.reg_type(Self::dest_reg(inst)?);
        let Kind::Array(element) = self.env.types.table.get(id).kind else { return Err(internal("an array literal that is not an array")) };
        self.fill_literal(inst, id, element, items)
    }

    /// `[a, b, c]` as a `List`.
    pub(crate) fn list_literal(&mut self, inst: &Inst, items: &[Operand]) -> Result<(), CompileError> {
        let id = self.reg_type(Self::dest_reg(inst)?);
        let Kind::List(element) = self.env.types.table.get(id).kind else { return Err(CompileError::internal("List of this type")) };
        self.fill_literal(inst, id, element, items)
    }

    fn fill_literal(&mut self, inst: &Inst, id: TypeId, element: TypeId, items: &[Operand]) -> Result<(), CompileError> {
        let stride = self.stride(element);
        let count = self.b.ins().iconst(types::I64, items.len() as i64);
        let (object, first) = self.new_sequence(id, count)?;
        for (at, item) in items.iter().enumerate() {
            self.write(Addr { base: first, offset: (at as u32 * stride) as i32 }, element, item)?;
        }
        self.set(inst.dest, Some(object));
        Ok(())
    }

    /// `[value; len]` as an array.
    pub(crate) fn array_repeat(&mut self, inst: &Inst, value: &Operand, len: &Operand) -> Result<(), CompileError> {
        let id = self.reg_type(Self::dest_reg(inst)?);
        let Kind::Array(element) = self.env.types.table.get(id).kind else { return Err(internal("a repeat that is not an array")) };
        let count = self.index_value(len)?;
        let negative = self.b.ins().icmp_imm(IntCC::SignedLessThan, count, 0);
        let past = self.b.ins().icmp_imm(IntCC::SignedGreaterThan, count, REPEAT_LIMIT);
        let bad = self.b.ins().bor(negative, past);
        self.fault_if(bad, FaultKind::IndexOutOfRange, inst.span);
        let (object, first) = self.new_sequence(id, count)?;
        let stride = self.stride(element);
        self.fill_loop(Addr { base: first, offset: 0 }, element, stride, count, value)?;
        self.set(inst.dest, Some(object));
        Ok(())
    }

    /// The number of elements of a fixed array, array, list or string.
    pub(crate) fn length(&mut self, operand: &Operand, mismatch: &dyn Fn() -> CompileError) -> Result<Value, CompileError> {
        let id = match operand {
            Operand::Reg(reg) => self.reg_type(*reg),
            Operand::Const(Const::String(_)) => self.env.types.id(TyId::STRING),
            Operand::Const(_) => return Err(mismatch()),
        };
        match self.env.types.table.get(id).kind {
            Kind::FixedArray(_, count) => Ok(self.b.ins().iconst(types::I64, i64::from(count))),
            Kind::Array(_) | Kind::List(_) | Kind::String => {
                let object = self.num_operand(operand, Num::Ref)?;
                Ok(self.load(types::I64, Addr { base: object, offset: LENGTH_OFFSET as i32 }))
            }
            _ => Err(mismatch()),
        }
    }

    /// The address and type of the element of an array or list at the index, after the bounds check.
    pub(crate) fn sequence_element(&mut self, object: &Operand, index: &Operand, span: Span) -> Result<(Addr, TypeId), CompileError> {
        let sequence = self.sequence(object)?;
        let element = sequence.element.ok_or_else(|| internal("an index into a string"))?;
        let at = self.index_value(index)?;
        let outside = self.b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, at, sequence.len);
        self.fault_if(outside, FaultKind::IndexOutOfRange, span);
        let by = self.b.ins().imul_imm(at, i64::from(sequence.stride));
        let address = self.b.ins().iadd(sequence.first, by);
        Ok((Addr { base: address, offset: 0 }, element))
    }

    /// The start and end of a range over `len` elements, faulting when it is not inside them.
    fn bounds(&mut self, (start, end, inclusive): Bounds, len: Value, span: Span) -> Result<(Value, Value), CompileError> {
        let start = match start {
            Some(start) => self.index_value(start)?,
            None => self.b.ins().iconst(types::I64, 0),
        };
        let end = match end {
            Some(end) => {
                let end = self.index_value(end)?;
                if inclusive { self.b.ins().iadd_imm(end, 1) } else { end }
            }
            None => len,
        };
        let negative = self.b.ins().icmp_imm(IntCC::SignedLessThan, start, 0);
        let inverted = self.b.ins().icmp(IntCC::SignedLessThan, end, start);
        let past = self.b.ins().icmp(IntCC::SignedGreaterThan, end, len);
        let outside = self.b.ins().bor(negative, inverted);
        let outside = self.b.ins().bor(outside, past);
        self.fault_if(outside, FaultKind::IndexOutOfRange, span);
        Ok((start, end))
    }

    /// `object[start..end]` and its step: a copy of the range, in the kind of the result.
    pub(crate) fn slice(&mut self, inst: &Inst, object: &Operand, bounds: Bounds, step: Option<&Operand>) -> Result<(), CompileError> {
        let id = self.reg_type(Self::dest_reg(inst)?);
        let source = self.sequence(object)?;
        let (start, end) = self.bounds(bounds, source.len, inst.span)?;
        let count = self.b.ins().isub(end, start);
        if self.env.types.table.get(id).kind == Kind::String {
            let string = self.num_operand(object, Num::Ref)?;
            let cut = self.func_ref(self.env.imports.cut);
            let call = self.b.ins().call(cut, &[string, start, end]);
            let whole = self.b.inst_results(call)[0];
            let broken = self.b.ins().icmp_imm(IntCC::Equal, whole, 0);
            self.fault_if(broken, FaultKind::IndexOutOfRange, inst.span);
            let (object, first) = self.new_sequence(id, count)?;
            let from = self.b.ins().iadd(source.first, start);
            self.copy_bytes(first, from, count);
            self.set(inst.dest, Some(object));
            return Ok(());
        }
        let stride = source.stride;
        let from = {
            let by = self.b.ins().imul_imm(start, i64::from(stride));
            self.b.ins().iadd(source.first, by)
        };
        let Some(step) = step else {
            let (object, first) = self.new_sequence(id, count)?;
            let bytes = self.b.ins().imul_imm(count, i64::from(stride));
            self.copy_bytes(first, from, bytes);
            self.set(inst.dest, Some(object));
            return Ok(());
        };
        let step = self.index_value(step)?;
        let short = self.b.ins().icmp_imm(IntCC::SignedLessThan, step, 1);
        self.fault_if(short, FaultKind::IndexOutOfRange, inst.span);
        let rounded = self.b.ins().iadd(count, step);
        let rounded = self.b.ins().iadd_imm(rounded, -1);
        let taken = self.b.ins().udiv(rounded, step);
        let (object, first) = self.new_sequence(id, taken)?;
        let header = self.b.create_block();
        self.b.append_block_param(header, types::I64);
        let body = self.b.create_block();
        let done = self.b.create_block();
        let zero = self.b.ins().iconst(types::I64, 0);
        self.b.ins().jump(header, &[zero]);
        self.b.switch_to_block(header);
        let position = self.b.block_params(header)[0];
        let more = self.b.ins().icmp(IntCC::UnsignedLessThan, position, taken);
        self.b.ins().brif(more, body, &[], done, &[]);
        self.b.switch_to_block(body);
        let jump = self.b.ins().imul(position, step);
        let jump = self.b.ins().imul_imm(jump, i64::from(stride));
        let read = Addr { base: self.b.ins().iadd(from, jump), offset: 0 };
        let by = self.b.ins().imul_imm(position, i64::from(stride));
        let write = Addr { base: self.b.ins().iadd(first, by), offset: 0 };
        self.copy_mem(write, read, stride);
        let next = self.b.ins().iadd_imm(position, 1);
        self.b.ins().jump(header, &[next]);
        self.b.switch_to_block(done);
        self.set(inst.dest, Some(object));
        Ok(())
    }

    /// `list[start..end] = value`: the range of a list overwritten by the elements of another list of its length, or by one value.
    pub(crate) fn set_slice(&mut self, inst: &Inst, object: &Operand, bounds: Bounds, value: &Operand) -> Result<(), CompileError> {
        let target = self.sequence(object)?;
        let Some(element) = target.element.filter(|_| matches!(self.env.types.table.get(self.reg_type_of(object)).kind, Kind::List(_))) else {
            return Err(CompileError::internal("SetSlice of this value"));
        };
        let (start, end) = self.bounds(bounds, target.len, inst.span)?;
        let count = self.b.ins().isub(end, start);
        let by = self.b.ins().imul_imm(start, i64::from(target.stride));
        let to = self.b.ins().iadd(target.first, by);
        if self.ty_of(value) == self.ty_of(object) {
            let source = self.sequence(value)?;
            let differs = self.b.ins().icmp(IntCC::NotEqual, source.len, count);
            self.fault_if(differs, FaultKind::IndexOutOfRange, inst.span);
            let bytes = self.b.ins().imul_imm(count, i64::from(target.stride));
            self.move_bytes(to, source.first, bytes);
            return Ok(());
        }
        self.fill_loop(Addr { base: to, offset: 0 }, element, target.stride, count, value)
    }

    fn reg_type_of(&self, operand: &Operand) -> TypeId {
        match operand {
            Operand::Reg(reg) => self.reg_type(*reg),
            Operand::Const(_) => self.env.types.id(TyId::STRING),
        }
    }

    /// `list_new`, `list_len` and `list_push`, which grows the list when it is full.
    pub(crate) fn list_intrinsic(&mut self, name: &str, inst: &Inst, args: &[Operand], outs: &[RegId]) -> Result<(), CompileError> {
        match (name, args) {
            ("list_new", []) => {
                let id = self.reg_type(Self::dest_reg(inst)?);
                let none = self.b.ins().iconst(types::I64, 0);
                let (list, _) = self.new_sequence(id, none)?;
                self.set(inst.dest, Some(list));
                Ok(())
            }
            ("list_len", [list]) => {
                let len = self.length(list, &|| CompileError::internal("list_len of this value"))?;
                self.set(inst.dest, Some(len));
                Ok(())
            }
            ("list_push", [list, value]) => self.push(list, value, outs),
            _ => Err(CompileError::internal(format!("the intrinsic `{name}` with these arguments"))),
        }
    }

    fn push(&mut self, list: &Operand, value: &Operand, outs: &[RegId]) -> Result<(), CompileError> {
        let id = self.reg_type_of(list);
        let Kind::List(element) = self.env.types.table.get(id).kind else { return Err(CompileError::internal("list_push of this value")) };
        let stride = self.stride(element);
        let buffer_id = self.env.types.buffer(id);
        let object = self.num_operand(list, Num::Ref)?;
        let len = self.load(types::I64, Addr { base: object, offset: LENGTH_OFFSET as i32 });
        let capacity = self.load(types::I64, Addr { base: object, offset: LIST_CAPACITY_OFFSET as i32 });
        let full = self.b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, len, capacity);
        let grow = self.b.create_block();
        let join = self.b.create_block();
        self.b.ins().brif(full, grow, &[], join, &[]);
        self.b.switch_to_block(grow);
        let empty = self.b.ins().icmp_imm(IntCC::Equal, capacity, 0);
        let doubled = self.b.ins().ishl_imm(capacity, 1);
        let first = self.b.ins().iconst(types::I64, FIRST_CAPACITY);
        let grown = self.b.ins().select(empty, first, doubled);
        let old = self.load(types::I64, Addr { base: object, offset: LIST_BUFFER_OFFSET as i32 });
        let new = self.alloc_array(buffer_id, grown);
        let to = self.b.ins().iadd_imm(new, i64::from(ELEMENTS_OFFSET));
        let from = self.b.ins().iadd_imm(old, i64::from(ELEMENTS_OFFSET));
        let bytes = self.b.ins().imul_imm(len, i64::from(stride));
        self.copy_bytes(to, from, bytes);
        self.store_reference(new, Addr { base: object, offset: LIST_BUFFER_OFFSET as i32 });
        self.store(grown, Addr { base: object, offset: LIST_CAPACITY_OFFSET as i32 });
        self.b.ins().jump(join, &[]);
        self.b.switch_to_block(join);
        let buffer = self.load(types::I64, Addr { base: object, offset: LIST_BUFFER_OFFSET as i32 });
        let by = self.b.ins().imul_imm(len, i64::from(stride));
        let at = self.b.ins().iadd(buffer, by);
        self.write(Addr { base: at, offset: ELEMENTS_OFFSET as i32 }, element, value)?;
        let longer = self.b.ins().iadd_imm(len, 1);
        self.store(longer, Addr { base: object, offset: LENGTH_OFFSET as i32 });
        for &out in outs {
            self.set(Some(out), Some(object));
        }
        Ok(())
    }
}
