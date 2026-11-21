//! Lowers values held in memory: structs, tuples, enums, optionals, unions and fixed arrays.

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{types, InstBuilder, MemFlags, StackSlot, StackSlotData, StackSlotKind, Type, Value};
use mote_abi::{align_up, FaultKind, Kind, PairPayload, Scalar, TypeId, BOX_VALUE_OFFSET};
use mote_ir::{Const, Inst, Op, Operand, RegId};
use mote_semantics::{BinaryOp, TyId, TyKind};

use crate::function::{class, clif_type, op_name, Lowering};
use crate::scalar::Num;
use crate::CompileError;

/// How a register holds its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Repr {
    /// A value with no room.
    Nothing,
    /// One Cranelift variable.
    Num(Num),
    /// A stack slot of this many bytes, laid out as the type table says.
    Mem(u32),
}

/// The scalars a value flattens to when it travels in registers, each with its offset in the value.
pub(crate) fn leaves_of(table: &mote_abi::TypeTable, id: TypeId, base: u32, out: &mut Vec<(u32, Scalar)>) {
    let info = table.get(id);
    match &info.kind {
        Kind::Unit => {}
        Kind::Scalar(scalar) => out.push((base, *scalar)),
        Kind::Struct(_) => {
            for field in &info.fields {
                leaves_of(table, field.ty, base + field.offset, out);
            }
        }
        Kind::FixedArray(element, count) => {
            let layout = table.layout(*element);
            if layout.size != 0 {
                for at in 0..*count {
                    leaves_of(table, *element, base + at * align_up(layout.size, layout.align), out);
                }
            }
        }
        _ if info.is_reference() => out.push((base, Scalar::Ref)),
        _ => out.extend((0..info.layout.size.div_ceil(8)).map(|word| (base + word * 8, Scalar::I64))),
    }
}

/// A place in memory: a pointer and a byte offset from it.
#[derive(Clone, Copy)]
pub(crate) struct Addr {
    pub base: Value,
    pub offset: i32,
}

impl Addr {
    pub fn at(self, by: u32) -> Addr {
        Addr { base: self.base, offset: self.offset + by as i32 }
    }
}

/// Where an aggregate is being built, and the register it is copied to when that was not safe to build in.
pub(crate) struct Building {
    at: Addr,
    copy_back: Option<(Addr, u32)>,
}

/// A scratch slot of a function and whether the instruction being lowered uses it.
pub(crate) struct Temp {
    slot: StackSlot,
    size: u32,
    used: bool,
}

/// Memory accesses may be unaligned and never trap.
fn flags() -> MemFlags {
    MemFlags::new().with_notrap()
}

/// The one place a store is emitted.
pub(crate) fn emit_store(b: &mut cranelift_frontend::FunctionBuilder, value: Value, at: Addr) {
    b.ins().store(flags(), value, at.base, at.offset);
}

/// The largest copy or fill that is done with loads and stores.
const INLINE_BYTES: u32 = 64;

/// An array filled in a loop past this many elements.
const UNROLLED_ELEMENTS: u32 = 8;

pub(crate) fn internal(what: &str) -> CompileError {
    CompileError::internal(what)
}

impl Lowering<'_, '_, '_> {
    /// How a register of the type holds its value.
    pub(crate) fn repr(&self, ty: TyId) -> Result<Repr, CompileError> {
        match self.env.program.types.kind(ty) {
            TyKind::Unit | TyKind::Bool | TyKind::Int(_) | TyKind::Float(_) => Ok(class(self.env.program, ty)?.map_or(Repr::Nothing, Repr::Num)),
            _ => self.repr_of(self.env.types.id(ty)),
        }
    }

    /// How a value of the table's type is held.
    pub(crate) fn repr_of(&self, id: TypeId) -> Result<Repr, CompileError> {
        let info = self.env.types.table.get(id);
        Ok(match &info.kind {
            Kind::Unit => Repr::Nothing,
            Kind::Scalar(scalar) => Repr::Num(Num::from_scalar(*scalar)),
            Kind::Struct(_) | Kind::Enum(_) | Kind::Pair | Kind::FixedArray(..) => size_repr(info.layout.size),
            Kind::Optional(_) if !info.is_reference() => size_repr(info.layout.size),
            _ if info.is_reference() => Repr::Num(Num::Ref),
            _ => return Err(internal("a type with no representation")),
        })
    }

    pub(crate) fn reg_type(&self, reg: RegId) -> TypeId {
        self.env.types.id(self.function.regs[reg].ty)
    }

    /// A slot that holds a value of this many bytes, with room past it for a word read at its end.
    pub(crate) fn new_slot(&mut self, size: u32) -> StackSlot {
        self.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, size.div_ceil(8) * 8 + 8, 3))
    }

    pub(crate) fn slot_addr(&mut self, slot: StackSlot) -> Addr {
        Addr { base: self.b.ins().stack_addr(types::I64, slot, 0), offset: 0 }
    }

    /// A zeroed slot of at least this many bytes, which holds until the next instruction.
    pub(crate) fn temp(&mut self, size: u32) -> Addr {
        let slot = match self.temps.iter_mut().find(|temp| !temp.used && temp.size >= size) {
            Some(temp) => {
                temp.used = true;
                temp.slot
            }
            None => {
                let slot = self.new_slot(size);
                self.temps.push(Temp { slot, size, used: true });
                slot
            }
        };
        let at = self.slot_addr(slot);
        self.zero_mem(at, size);
        at
    }

    /// Frees the slots `temp` gave out for the next instruction.
    pub(crate) fn release_temps(&mut self) {
        for temp in &mut self.temps {
            temp.used = false;
        }
    }

    pub(crate) fn reg_addr(&mut self, reg: RegId) -> Result<Addr, CompileError> {
        let slot = *self.slots.get(&reg).ok_or_else(|| internal("a register with no slot"))?;
        Ok(self.slot_addr(slot))
    }

    /// The address as one pointer value.
    pub(crate) fn address(&mut self, at: Addr) -> Value {
        if at.offset == 0 { at.base } else { self.b.ins().iadd_imm(at.base, i64::from(at.offset)) }
    }

    pub(crate) fn load(&mut self, ty: Type, at: Addr) -> Value {
        self.b.ins().load(ty, flags(), at.base, at.offset)
    }

    pub(crate) fn store(&mut self, value: Value, at: Addr) {
        emit_store(&mut self.b, value, at);
    }

    pub(crate) fn copy_mem(&mut self, to: Addr, from: Addr, size: u32) {
        if size <= INLINE_BYTES {
            let mut parts = Vec::new();
            let mut done = 0;
            for width in [8, 4, 2, 1] {
                while size - done >= width {
                    parts.push((done, self.load(Type::int(width as u16 * 8).expect("a width of an integer"), from.at(done))));
                    done += width;
                }
            }
            for (offset, value) in parts {
                self.store(value, to.at(offset));
            }
        } else {
            let config = self.env.clif.isa().frontend_config();
            let (to, from) = (self.address(to), self.address(from));
            let size = self.b.ins().iconst(types::I64, i64::from(size));
            self.b.call_memmove(config, to, from, size);
        }
    }

    pub(crate) fn zero_mem(&mut self, at: Addr, size: u32) {
        if size <= INLINE_BYTES {
            let mut done = 0;
            for width in [8, 4, 2, 1] {
                while size - done >= width {
                    let zero = self.b.ins().iconst(Type::int(width as u16 * 8).expect("a width of an integer"), 0);
                    self.store(zero, at.at(done));
                    done += width;
                }
            }
        } else {
            let config = self.env.clif.isa().frontend_config();
            let buffer = self.address(at);
            let zero = self.b.ins().iconst(types::I8, 0);
            let size = self.b.ins().iconst(types::I64, i64::from(size));
            self.b.call_memset(config, buffer, zero, size);
        }
    }

    /// The value of an operand that is a number of the class.
    pub(crate) fn num_operand(&mut self, operand: &Operand, num: Num) -> Result<Value, CompileError> {
        match operand {
            Operand::Reg(reg) => {
                let var = *self.vars.get(reg).ok_or_else(|| internal("a number with no variable"))?;
                Ok(self.b.use_var(var))
            }
            Operand::Const(constant) => self.constant(*constant, num),
        }
    }

    /// Writes the operand, a value of the table's type, at the address.
    pub(crate) fn write(&mut self, at: Addr, id: TypeId, operand: &Operand) -> Result<(), CompileError> {
        if let Kind::Box(inner) = self.env.types.table.get(id).kind {
            let boxed = self.alloc(id);
            self.write(Addr { base: boxed, offset: BOX_VALUE_OFFSET as i32 }, inner, operand)?;
            self.store_reference(boxed, at);
            return Ok(());
        }
        match self.repr_of(id)? {
            Repr::Nothing => Ok(()),
            Repr::Num(num) => {
                let value = self.num_operand(operand, num)?;
                if num == Num::Ref {
                    self.store_reference(value, at);
                } else {
                    self.store(value, at);
                }
                Ok(())
            }
            Repr::Mem(size) => match operand {
                Operand::Reg(reg) => {
                    let from = self.reg_addr(*reg)?;
                    self.copy_mem(at, from, size);
                    Ok(())
                }
                Operand::Const(Const::Null) => {
                    self.zero_mem(at, size);
                    Ok(())
                }
                Operand::Const(_) => Err(internal("a constant for a value in memory")),
            },
        }
    }

    /// Reads a value of the table's type at the address into the register.
    pub(crate) fn read_into(&mut self, dest: Option<RegId>, at: Addr, id: TypeId) -> Result<(), CompileError> {
        let Some(reg) = dest else { return Ok(()) };
        if let Kind::Box(inner) = self.env.types.table.get(id).kind {
            let boxed = self.load(types::I64, at);
            return self.read_into(dest, Addr { base: boxed, offset: BOX_VALUE_OFFSET as i32 }, inner);
        }
        match self.repr_of(id)? {
            Repr::Nothing => {}
            Repr::Num(num) => {
                let value = self.load(num.clif(), at);
                if !self.vars.contains_key(&reg) {
                    return Err(internal("a number with no variable"));
                }
                self.define(reg, value);
            }
            Repr::Mem(size) => {
                let to = self.reg_addr(reg)?;
                self.copy_mem(to, at, size);
            }
        }
        Ok(())
    }

    /// The scalars a value flattens to when it travels in registers, each with its offset in the value.
    pub(crate) fn leaves(&self, id: TypeId, base: u32, out: &mut Vec<(u32, Scalar)>) {
        leaves_of(&self.env.types.table, id, base, out);
    }

    /// The scalars of an operand of the table's type, in the order a call passes them.
    pub(crate) fn operand_leaves(&mut self, operand: &Operand, id: TypeId) -> Result<Vec<Value>, CompileError> {
        match self.repr_of(id)? {
            Repr::Nothing => Ok(Vec::new()),
            Repr::Num(num) => Ok(vec![self.num_operand(operand, num)?]),
            Repr::Mem(_) => {
                let mut leaves = Vec::new();
                self.leaves(id, 0, &mut leaves);
                match operand {
                    Operand::Reg(reg) => {
                        let at = self.reg_addr(*reg)?;
                        Ok(leaves.into_iter().map(|(offset, scalar)| self.load(clif_type(scalar), at.at(offset))).collect())
                    }
                    Operand::Const(Const::Null) => Ok(leaves.into_iter().map(|(_, scalar)| self.zero_scalar(scalar)).collect()),
                    Operand::Const(_) => Err(internal("a constant for a value in memory")),
                }
            }
        }
    }

    fn zero_scalar(&mut self, scalar: Scalar) -> Value {
        match scalar {
            Scalar::F32 => self.b.ins().f32const(0.0),
            Scalar::F64 => self.b.ins().f64const(0.0),
            other => self.b.ins().iconst(clif_type(other), 0),
        }
    }

    /// Stores the scalars a call returned or received in the register's slot.
    pub(crate) fn store_leaves(&mut self, reg: RegId, id: TypeId, values: &[Value]) -> Result<(), CompileError> {
        match self.repr_of(id)? {
            Repr::Nothing => {}
            Repr::Num(_) => {
                if !self.vars.contains_key(&reg) {
                    return Err(internal("a number with no variable"));
                }
                self.define(reg, values[0]);
            }
            Repr::Mem(_) => {
                let at = self.reg_addr(reg)?;
                let mut leaves = Vec::new();
                self.leaves(id, 0, &mut leaves);
                for ((offset, _), &value) in leaves.iter().zip(values) {
                    self.store(value, at.at(*offset));
                }
            }
        }
        Ok(())
    }

    fn building(&mut self, dest: RegId, from: &[&Operand]) -> Result<Building, CompileError> {
        let real = self.reg_addr(dest)?;
        if from.iter().any(|operand| matches!(operand, Operand::Reg(reg) if *reg == dest)) {
            let size = self.env.types.table.layout(self.reg_type(dest)).size;
            let at = self.temp(size);
            Ok(Building { at, copy_back: Some((real, size)) })
        } else {
            Ok(Building { at: real, copy_back: None })
        }
    }

    fn built(&mut self, building: Building) {
        if let Some((real, size)) = building.copy_back {
            self.copy_mem(real, building.at, size);
        }
    }

    /// `x = operand` for a value of any representation.
    pub(crate) fn copy_into(&mut self, dest: RegId, operand: &Operand) -> Result<(), CompileError> {
        match self.repr(self.function.regs[dest].ty)? {
            Repr::Nothing => Ok(()),
            Repr::Num(num) => {
                let value = self.num_operand(operand, num)?;
                self.set(Some(dest), Some(value));
                Ok(())
            }
            Repr::Mem(_) => self.copy_value(dest, operand),
        }
    }

    /// `x = operand` for a value in memory.
    pub(crate) fn copy_value(&mut self, dest: RegId, operand: &Operand) -> Result<(), CompileError> {
        let id = self.reg_type(dest);
        let at = self.reg_addr(dest)?;
        self.write(at, id, operand)
    }

    pub(crate) fn dest_reg(inst: &Inst) -> Result<RegId, CompileError> {
        inst.dest.ok_or_else(|| internal("an op with no destination"))
    }

    /// The register an operand names, for an op that needs a value in memory.
    pub(crate) fn reg_of(operand: &Operand) -> Result<RegId, CompileError> {
        match operand {
            Operand::Reg(reg) => Ok(*reg),
            Operand::Const(_) => Err(internal("a constant where a value in memory is read")),
        }
    }

    /// An integer operand as 64 bits, sign or zero extended by its type.
    pub(crate) fn index_value(&mut self, operand: &Operand) -> Result<Value, CompileError> {
        let ty = self.ty_of(operand);
        let Some(Num::Int { bits, signed }) = class(self.env.program, ty)? else { return Err(internal("an index that is not an integer")) };
        let value = self.num_operand(operand, Num::Int { bits, signed })?;
        Ok(match (bits, signed) {
            (64, _) => value,
            (_, true) => self.b.ins().sextend(types::I64, value),
            (_, false) => self.b.ins().uextend(types::I64, value),
        })
    }

    /// The tag of an enum value as 64 bits.
    pub(crate) fn tag_of(&mut self, reg: RegId) -> Result<Value, CompileError> {
        let info = self.env.types.table.get(self.reg_type(reg));
        if !matches!(info.kind, Kind::Enum(_)) {
            return Err(CompileError::internal("the tag of this value"));
        }
        let ty = tag_type(info.tag_size);
        let at = self.reg_addr(reg)?;
        let tag = self.load(ty, at);
        Ok(if ty == types::I64 { tag } else { self.b.ins().uextend(types::I64, tag) })
    }

    /// A struct or tuple of the items, or a class object holding them.
    fn record(&mut self, inst: &Inst, items: &[Operand]) -> Result<(), CompileError> {
        let dest = Self::dest_reg(inst)?;
        let id = self.reg_type(dest);
        let info = self.env.types.table.get(id);
        if let (Kind::Class(_), Some(object)) = (&info.kind, &info.object) {
            let reference = self.alloc(id);
            for (field, item) in object.fields.iter().zip(items) {
                self.write(Addr { base: reference, offset: field.offset as i32 }, field.ty, item)?;
            }
            self.set(inst.dest, Some(reference));
            return Ok(());
        }
        if !matches!(info.kind, Kind::Struct(_)) {
            return Err(CompileError::internal(op_name(&inst.op)));
        }
        if self.repr_of(id)? == Repr::Nothing {
            return Ok(());
        }
        let building = self.building(dest, &items.iter().collect::<Vec<_>>())?;
        for (field, item) in info.fields.iter().zip(items) {
            self.write(building.at.at(field.offset), field.ty, item)?;
        }
        self.built(building);
        Ok(())
    }

    /// Lowers an op on a struct, tuple, enum, optional, union or fixed array.
    pub(crate) fn value_op(&mut self, inst: &Inst) -> Result<(), CompileError> {
        let mismatch = || CompileError::internal(op_name(&inst.op));
        match &inst.op {
            Op::Tuple(items) | Op::Record(items) => self.record(inst, items),
            Op::Range { start, end, inclusive } => self.record(inst, &[*start, *end, Operand::Const(Const::Bool(*inclusive))]),
            Op::Variant { variant, fields } => {
                let dest = Self::dest_reg(inst)?;
                let id = self.reg_type(dest);
                let info = self.env.types.table.get(id);
                if !matches!(info.kind, Kind::Enum(_)) {
                    return Err(mismatch());
                }
                let building = self.building(dest, &fields.iter().collect::<Vec<_>>())?;
                let tag = self.b.ins().iconst(tag_type(info.tag_size), i64::from(*variant));
                self.store(tag, building.at);
                for (field, item) in info.variants[*variant as usize].fields.iter().zip(fields) {
                    self.write(building.at.at(field.offset), field.ty, item)?;
                }
                self.built(building);
                Ok(())
            }
            Op::Array(items) => {
                let dest = Self::dest_reg(inst)?;
                let id = self.reg_type(dest);
                if matches!(self.env.types.table.get(id).kind, Kind::Array(_)) {
                    return self.array_literal(inst, items);
                }
                let Kind::FixedArray(element, _) = self.env.types.table.get(id).kind else { return Err(mismatch()) };
                if self.repr_of(id)? == Repr::Nothing {
                    return Ok(());
                }
                let stride = self.stride(element);
                let building = self.building(dest, &items.iter().collect::<Vec<_>>())?;
                for (at, item) in items.iter().enumerate() {
                    self.write(building.at.at(at as u32 * stride), element, item)?;
                }
                self.built(building);
                Ok(())
            }
            Op::List(items) => self.list_literal(inst, items),
            Op::ArrayRepeat { value, len } => {
                let dest = Self::dest_reg(inst)?;
                let id = self.reg_type(dest);
                if matches!(self.env.types.table.get(id).kind, Kind::Array(_)) {
                    return self.array_repeat(inst, value, len);
                }
                let Kind::FixedArray(element, count) = self.env.types.table.get(id).kind else { return Err(mismatch()) };
                if self.repr_of(id)? == Repr::Nothing {
                    return Ok(());
                }
                let stride = self.stride(element);
                let building = self.building(dest, &[value])?;
                self.fill(building.at, element, stride, count, value)?;
                self.built(building);
                Ok(())
            }
            Op::Field { object, index } | Op::TupleField { object, index } => {
                let reg = Self::reg_of(object)?;
                let id = self.reg_type(reg);
                let info = self.env.types.table.get(id);
                if let (Kind::Class(_), Some(shape)) = (&info.kind, &info.object) {
                    let field = *shape.fields.get(*index as usize).ok_or_else(|| internal("a field that is missing"))?;
                    let base = self.num_operand(object, Num::Ref)?;
                    return self.read_into(inst.dest, Addr { base, offset: field.offset as i32 }, field.ty);
                }
                if !matches!(info.kind, Kind::Struct(_)) {
                    return Err(mismatch());
                }
                let field = *info.fields.get(*index as usize).ok_or_else(|| internal("a field that is missing"))?;
                if self.repr_of(id)? == Repr::Nothing {
                    return Ok(());
                }
                let at = self.reg_addr(reg)?;
                self.read_into(inst.dest, at.at(field.offset), field.ty)
            }
            Op::SetField { object, index, value } => {
                let reg = Self::reg_of(object)?;
                let id = self.reg_type(reg);
                let info = self.env.types.table.get(id);
                if let (Kind::Class(_), Some(shape)) = (&info.kind, &info.object) {
                    let field = *shape.fields.get(*index as usize).ok_or_else(|| internal("a field that is missing"))?;
                    let base = self.num_operand(object, Num::Ref)?;
                    return self.write(Addr { base, offset: field.offset as i32 }, field.ty, value);
                }
                if !matches!(info.kind, Kind::Struct(_)) {
                    return Err(mismatch());
                }
                let field = *info.fields.get(*index as usize).ok_or_else(|| internal("a field that is missing"))?;
                if self.repr_of(id)? == Repr::Nothing {
                    return Ok(());
                }
                let at = self.reg_addr(reg)?;
                self.write(at.at(field.offset), field.ty, value)
            }
            Op::Len(object) => {
                let value = self.length(object, &mismatch)?;
                self.set(inst.dest, Some(value));
                Ok(())
            }
            Op::Slice { object, start, end, inclusive, step } => self.slice(inst, object, (start, end, *inclusive), step.as_ref()),
            Op::SetSlice { object, start, end, inclusive, value } => self.set_slice(inst, object, (start, end, *inclusive), value),
            Op::Index { object, index } => {
                let (at, element) = self.element(object, index, inst, &mismatch)?;
                match at {
                    Some(at) => self.read_into(inst.dest, at, element),
                    None => Ok(()),
                }
            }
            Op::SetIndex { object, index, value } => {
                let (at, element) = self.element(object, index, inst, &mismatch)?;
                match at {
                    Some(at) => self.write(at, element, value),
                    None => Ok(()),
                }
            }
            Op::IsSome(operand) => {
                let reg = Self::reg_of(operand)?;
                let some = if self.optional_is_reference(reg, &mismatch)? {
                    let reference = self.num_operand(operand, Num::Ref)?;
                    self.b.ins().icmp_imm(IntCC::NotEqual, reference, 0)
                } else {
                    let at = self.reg_addr(reg)?;
                    let tag = self.load(types::I64, at);
                    self.b.ins().icmp_imm(IntCC::NotEqual, tag, 0)
                };
                self.set(inst.dest, Some(some));
                Ok(())
            }
            Op::SomePayload(operand) => {
                let reg = Self::reg_of(operand)?;
                if self.optional_is_reference(reg, &mismatch)? {
                    let reference = self.num_operand(operand, Num::Ref)?;
                    self.set(inst.dest, Some(reference));
                    return Ok(());
                }
                self.read_payload(inst.dest, reg, 1, 0)
            }
            Op::Unwrap(operand) => {
                let reg = Self::reg_of(operand)?;
                let info = self.env.types.table.get(self.reg_type(reg));
                match info.kind {
                    Kind::Optional(_) if info.is_reference() => {
                        let reference = self.num_operand(operand, Num::Ref)?;
                        let none = self.b.ins().icmp_imm(IntCC::Equal, reference, 0);
                        self.fault_if(none, FaultKind::UnwrapFailed, inst.span);
                        self.set(inst.dest, Some(reference));
                        Ok(())
                    }
                    Kind::Optional(_) => {
                        let at = self.reg_addr(reg)?;
                        let tag = self.load(types::I64, at);
                        let none = self.b.ins().icmp_imm(IntCC::Equal, tag, 0);
                        self.fault_if(none, FaultKind::UnwrapFailed, inst.span);
                        self.read_payload(inst.dest, reg, 1, 0)
                    }
                    Kind::Enum(_) => {
                        let ok = self.env.ok_tag.ok_or_else(mismatch)?;
                        let tag = self.tag_of(reg)?;
                        let failed = self.b.ins().icmp_imm(IntCC::NotEqual, tag, i64::from(ok));
                        self.fault_if(failed, FaultKind::UnwrapFailed, inst.span);
                        if info.variants[ok as usize].fields.is_empty() {
                            return Err(mismatch());
                        }
                        self.read_payload(inst.dest, reg, ok, 0)
                    }
                    _ => Err(mismatch()),
                }
            }
            Op::VariantTag(operand) => {
                let tag = self.tag_of(Self::reg_of(operand)?)?;
                self.set(inst.dest, Some(tag));
                Ok(())
            }
            Op::IsVariant { object, variant } => {
                let tag = self.tag_of(Self::reg_of(object)?)?;
                let is = self.b.ins().icmp_imm(IntCC::Equal, tag, i64::from(*variant));
                self.set(inst.dest, Some(is));
                Ok(())
            }
            Op::VariantField { object, variant, index } => {
                let reg = Self::reg_of(object)?;
                if !matches!(self.env.types.table.get(self.reg_type(reg)).kind, Kind::Enum(_)) {
                    return Err(mismatch());
                }
                self.read_payload(inst.dest, reg, *variant, *index)
            }
            Op::Convert { conversion, operand } => self.convert(inst, *conversion, operand, &mismatch),
            Op::IsType { operand, ty } => {
                let reg = Self::reg_of(operand)?;
                if matches!(self.env.program.types.kind(self.function.regs[reg].ty), TyKind::Any) {
                    let result = match self.env.program.types.kind(*ty).clone() {
                        TyKind::Any => self.b.ins().iconst(types::I8, 1),
                        TyKind::Union(ms) => {
                            let at = self.reg_addr(reg)?;
                            let word = self.load(types::I64, at);
                            let mut result = self.b.ins().iconst(types::I8, 0);
                            for m in ms {
                                let id = self.env.types.id(m);
                                let is = self.b.ins().icmp_imm(IntCC::Equal, word, i64::from(id.raw()));
                                result = self.b.ins().bor(result, is);
                            }
                            result
                        }
                        _ => {
                            let at = self.reg_addr(reg)?;
                            let word = self.load(types::I64, at);
                            let id = self.env.types.id(*ty);
                            self.b.ins().icmp_imm(IntCC::Equal, word, i64::from(id.raw()))
                        }
                    };
                    self.set(inst.dest, Some(result));
                    return Ok(());
                }
                let members = self.union_members(reg, &mismatch)?;
                let fitting: Vec<TypeId> = members.iter().filter(|&&member| self.fits(member, *ty)).map(|&member| self.env.types.id(member)).collect();
                let result = if fitting.len() == members.len() {
                    self.b.ins().iconst(types::I8, 1)
                } else {
                    let at = self.reg_addr(reg)?;
                    let word = self.load(types::I64, at);
                    let mut result = self.b.ins().iconst(types::I8, 0);
                    for id in fitting {
                        let is = self.b.ins().icmp_imm(IntCC::Equal, word, i64::from(id.raw()));
                        result = self.b.ins().bor(result, is);
                    }
                    result
                };
                self.set(inst.dest, Some(result));
                Ok(())
            }
            Op::UnionPayload { operand, ty } => {
                let reg = Self::reg_of(operand)?;
                if !matches!(self.env.program.types.kind(self.function.regs[reg].ty), TyKind::Any) {
                    self.union_members(reg, &mismatch)?;
                }
                let dest = Self::dest_reg(inst)?;
                let to = self.env.types.id(*ty);
                if matches!(self.env.program.types.kind(*ty), TyKind::Union(_) | TyKind::Any) {
                    let (from, into) = (self.reg_addr(reg)?, self.reg_addr(dest)?);
                    self.copy_mem(into, from, 16);
                    return Ok(());
                }
                let at = self.reg_addr(reg)?;
                if self.env.types.table.pair_payload(to) == PairPayload::Boxed {
                    let boxed = self.load(types::I64, at.at(8));
                    return self.read_into(inst.dest, Addr { base: boxed, offset: BOX_VALUE_OFFSET as i32 }, to);
                }
                self.read_into(inst.dest, at.at(8), to)
            }
            Op::Concat(parts) => {
                let joined = self.concat(parts)?;
                self.set(inst.dest, Some(joined));
                Ok(())
            }
            Op::Show(operand) => {
                let text = self.show(operand, inst.span)?;
                self.set(inst.dest, Some(text));
                Ok(())
            }
            Op::Detach(operand) => self.detach(inst, operand),
            other => Err(CompileError::internal(op_name(other))),
        }
    }

    /// `x == null` and `x != null` on an optional held with a tag.
    pub(crate) fn null_compare(&mut self, op: BinaryOp, left: &Operand, right: &Operand) -> Result<Option<Value>, CompileError> {
        let ((Operand::Reg(reg), Operand::Const(Const::Null)) | (Operand::Const(Const::Null), Operand::Reg(reg))) = (left, right) else { return Ok(None) };
        let info = self.env.types.table.get(self.reg_type(*reg));
        if !matches!(op, BinaryOp::Eq | BinaryOp::NotEq) || !matches!(info.kind, Kind::Optional(_)) || info.is_reference() {
            return Ok(None);
        }
        let at = self.reg_addr(*reg)?;
        let tag = self.load(types::I64, at);
        let cc = if op == BinaryOp::Eq { IntCC::Equal } else { IntCC::NotEqual };
        Ok(Some(self.b.ins().icmp_imm(cc, tag, 0)))
    }

    /// The constant of the variant an enum value is, as the integer type.
    pub(crate) fn enum_constant(&mut self, operand: &Operand, constants: &[i128], target: Num) -> Result<Value, CompileError> {
        let tag = self.tag_of(Self::reg_of(operand)?)?;
        let mut found = self.b.ins().iconst(types::I64, constants.last().copied().unwrap_or(0) as i64);
        for (at, constant) in constants.iter().enumerate().rev().skip(1) {
            let is = self.b.ins().icmp_imm(IntCC::Equal, tag, at as i64);
            let value = self.b.ins().iconst(types::I64, *constant as i64);
            found = self.b.ins().select(is, value, found);
        }
        self.cast(found, Num::Int { bits: 64, signed: true }, target)
    }

    /// `==` and `!=` where an operand is an optional number held with a tag, against another or against a number of its type.
    pub(crate) fn optional_compare(&mut self, op: BinaryOp, left: &Operand, right: &Operand) -> Result<Option<Value>, CompileError> {
        if !matches!(op, BinaryOp::Eq | BinaryOp::NotEq) {
            return Ok(None);
        }
        let tagged = |this: &Self, operand: &Operand| match operand {
            Operand::Reg(reg) => matches!(this.env.program.types.kind(this.function.regs[*reg].ty), TyKind::Optional(_)) && !this.env.types.table.get(this.reg_type(*reg)).is_reference(),
            Operand::Const(_) => false,
        };
        let (left_tagged, right_tagged) = (tagged(self, left), tagged(self, right));
        let Some(Operand::Reg(first)) = [(left_tagged, left), (right_tagged, right)].into_iter().find(|(held, _)| *held).map(|(_, operand)| *operand) else { return Ok(None) };
        let TyKind::Optional(inner) = *self.env.program.types.kind(self.function.regs[first].ty) else { return Ok(None) };
        let Some(num) = class(self.env.program, inner)? else { return Ok(None) };
        let float = matches!(num, Num::F32 | Num::F64);
        let mut sides = Vec::new();
        for (held, operand) in [(left_tagged, left), (right_tagged, right)] {
            if let (true, Operand::Reg(reg)) = (held, operand) {
                let field = *self.env.types.table.get(self.reg_type(*reg)).variants.get(1).and_then(|variant| variant.fields.first()).ok_or_else(|| internal("an optional with no payload"))?;
                let at = self.reg_addr(*reg)?;
                let tag = self.load(types::I64, at);
                let payload = self.load(num.clif(), at.at(field.offset));
                sides.push((Some(tag), payload));
            } else {
                let value = self.operand(operand, inner)?.expect("a number has a value");
                sides.push((None, value));
            }
        }
        let same = |this: &mut Self, a: Value, b: Value| if float { this.b.ins().fcmp(FloatCC::Equal, a, b) } else { this.b.ins().icmp(IntCC::Equal, a, b) };
        let equal = match (sides[0], sides[1]) {
            ((Some(a_tag), a), (Some(b_tag), b)) => {
                let tags = self.b.ins().icmp(IntCC::Equal, a_tag, b_tag);
                let absent = self.b.ins().icmp_imm(IntCC::Equal, a_tag, 0);
                let payloads = same(self, a, b);
                let agree = self.b.ins().bor(absent, payloads);
                self.b.ins().band(tags, agree)
            }
            ((Some(tag), held), (None, value)) | ((None, value), (Some(tag), held)) => {
                let present = self.b.ins().icmp_imm(IntCC::NotEqual, tag, 0);
                let payloads = same(self, held, value);
                self.b.ins().band(present, payloads)
            }
            _ => return Ok(None),
        };
        Ok(Some(if op == BinaryOp::Eq { equal } else { self.b.ins().bxor_imm(equal, 1) }))
    }

    pub(crate) fn stride(&self, element: TypeId) -> u32 {
        let layout = self.env.types.table.layout(element);
        align_up(layout.size, layout.align)
    }

    /// The members of the union a register holds, when each is a plain type.
    fn union_members(&self, reg: RegId, mismatch: &dyn Fn() -> CompileError) -> Result<Vec<TyId>, CompileError> {
        let TyKind::Union(members) = self.env.program.types.kind(self.function.regs[reg].ty) else { return Err(mismatch()) };
        if members.iter().any(|&member| matches!(self.env.program.types.kind(member), TyKind::Optional(_) | TyKind::Union(_))) {
            return Err(CompileError::internal("a union with an optional member"));
        }
        Ok(members.clone())
    }

    /// The name of the type a union or `Any` value holds, as the string object of that type.
    pub(crate) fn union_type_name(&mut self, operand: &Operand) -> Result<Value, CompileError> {
        let reg = Self::reg_of(operand)?;
        let members = if matches!(self.env.program.types.kind(self.function.regs[reg].ty), TyKind::Any) {
            // What an `Any` was made from, and `Any` itself for a value no boxing site explains.
            self.env.any_members.iter().copied().chain([TyId::ANY]).collect()
        } else {
            self.union_members(reg, &|| CompileError::internal("type_tag this value"))?
        };
        let at = self.reg_addr(reg)?;
        let word = self.load(types::I64, at);
        let name = |this: &mut Self, member: TyId| -> Result<Value, CompileError> {
            let data = *this.env.type_names.get(&member).ok_or_else(|| internal("a union member with no name"))?;
            let global = this.env.clif.declare_data_in_func(data, this.b.func);
            Ok(this.b.ins().symbol_value(types::I64, global))
        };
        let last = *members.last().ok_or_else(|| internal("a union with no member"))?;
        let mut found = name(self, last)?;
        for &member in members.iter().rev().skip(1) {
            let is = self.b.ins().icmp_imm(IntCC::Equal, word, i64::from(self.env.types.id(member).raw()));
            let text = name(self, member)?;
            found = self.b.ins().select(is, text, found);
        }
        Ok(found)
    }

    /// Whether a value of the type fits the expected type, as `is` decides it.
    fn fits(&self, actual: TyId, expected: TyId) -> bool {
        if actual == expected {
            return true;
        }
        match (self.env.program.types.kind(expected), self.env.program.types.kind(actual)) {
            (TyKind::Any, _) => true,
            (TyKind::Union(members), _) => members.iter().any(|&member| self.fits(actual, member)),
            (TyKind::Optional(inner), _) => actual == TyId::UNIT || self.fits(actual, *inner),
            _ => false,
        }
    }

    /// Whether the optional in the register is a nullable reference; an error when it is not an optional.
    fn optional_is_reference(&self, reg: RegId, mismatch: &dyn Fn() -> CompileError) -> Result<bool, CompileError> {
        let info = self.env.types.table.get(self.reg_type(reg));
        if matches!(info.kind, Kind::Optional(_)) { Ok(info.is_reference()) } else { Err(mismatch()) }
    }

    /// Reads field `index` of a variant of the enum or optional in the register.
    fn read_payload(&mut self, dest: Option<RegId>, reg: RegId, variant: u32, index: u32) -> Result<(), CompileError> {
        let field = *self.env.types.table.get(self.reg_type(reg)).variants[variant as usize].fields.get(index as usize).ok_or_else(|| internal("a payload field that is missing"))?;
        if self.repr_of(self.reg_type(reg))? == Repr::Nothing {
            return Ok(());
        }
        let at = self.reg_addr(reg)?;
        self.read_into(dest, at.at(field.offset), field.ty)
    }

    /// The address of the element of a fixed array at the index, after the bounds check.
    fn element(&mut self, object: &Operand, index: &Operand, inst: &Inst, mismatch: &dyn Fn() -> CompileError) -> Result<(Option<Addr>, TypeId), CompileError> {
        let reg = Self::reg_of(object)?;
        let id = self.reg_type(reg);
        if matches!(self.env.types.table.get(id).kind, Kind::Array(_) | Kind::List(_)) {
            let (at, element) = self.sequence_element(object, index, inst.span)?;
            return Ok((Some(at), element));
        }
        let Kind::FixedArray(element, count) = self.env.types.table.get(id).kind else { return Err(mismatch()) };
        let at = self.index_value(index)?;
        let outside = self.b.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, at, i64::from(count));
        self.fault_if(outside, FaultKind::IndexOutOfRange, inst.span);
        if self.repr_of(id)? == Repr::Nothing {
            return Ok((None, element));
        }
        let base = self.reg_addr(reg)?;
        let stride = i64::from(self.stride(element));
        let by = self.b.ins().imul_imm(at, stride);
        let address = self.b.ins().iadd(base.base, by);
        Ok((Some(Addr { base: address, offset: base.offset }), element))
    }

    /// Writes the value at each of `count` elements `stride` bytes apart, in a loop when there are many.
    fn fill(&mut self, at: Addr, element: TypeId, stride: u32, count: u32, value: &Operand) -> Result<(), CompileError> {
        if count <= UNROLLED_ELEMENTS {
            for position in 0..count {
                self.write(at.at(position * stride), element, value)?;
            }
            return Ok(());
        }
        let count = self.b.ins().iconst(types::I64, i64::from(count));
        self.fill_loop(at, element, stride, count, value)
    }

    /// Writes the value at each of `count` elements `stride` bytes apart, in a loop.
    pub(crate) fn fill_loop(&mut self, at: Addr, element: TypeId, stride: u32, count: Value, value: &Operand) -> Result<(), CompileError> {
        let header = self.b.create_block();
        self.b.append_block_param(header, types::I64);
        let body = self.b.create_block();
        let done = self.b.create_block();
        let start = self.b.ins().iconst(types::I64, 0);
        self.b.ins().jump(header, &[start]);
        self.b.switch_to_block(header);
        let position = self.b.block_params(header)[0];
        let more = self.b.ins().icmp(IntCC::UnsignedLessThan, position, count);
        self.b.ins().brif(more, body, &[], done, &[]);
        self.b.switch_to_block(body);
        let by = self.b.ins().imul_imm(position, i64::from(stride));
        let cursor = Addr { base: self.b.ins().iadd(at.base, by), offset: at.offset };
        self.write(cursor, element, value)?;
        let next = self.b.ins().iadd_imm(position, 1);
        self.b.ins().jump(header, &[next]);
        self.b.switch_to_block(done);
        Ok(())
    }

    fn convert(&mut self, inst: &Inst, conversion: mote_semantics::Conversion, operand: &Operand, mismatch: &dyn Fn() -> CompileError) -> Result<(), CompileError> {
        use mote_semantics::Conversion;
        let dest = Self::dest_reg(inst)?;
        let id = self.reg_type(dest);
        let info = self.env.types.table.get(id);
        match conversion {
            Conversion::ToOptional => {
                if !matches!(info.kind, Kind::Optional(_)) {
                    return Err(mismatch());
                }
                if info.is_reference() {
                    return self.copy_into(dest, operand);
                }
                let field = info.variants[1].fields[0];
                let building = self.building(dest, &[operand])?;
                let some = self.b.ins().iconst(types::I64, 1);
                self.store(some, building.at);
                self.write(building.at.at(field.offset), field.ty, operand)?;
                self.built(building);
                Ok(())
            }
            Conversion::WidenUnion => self.copy_value(dest, operand),
            Conversion::ToUnion => {
                if !matches!(info.kind, Kind::Pair) {
                    return Err(mismatch());
                }
                let held = self.ty_of(operand);
                if matches!(self.env.program.types.kind(held), TyKind::Union(_) | TyKind::Any) {
                    return self.copy_value(dest, operand);
                }
                let held_id = self.env.types.id(held);
                let building = self.building(dest, &[operand])?;
                let word = self.b.ins().iconst(types::I64, i64::from(held_id.raw()));
                self.store(word, building.at);
                let zero = self.b.ins().iconst(types::I64, 0);
                self.store(zero, building.at.at(8));
                if self.env.types.table.pair_payload(held_id) == PairPayload::Boxed {
                    let boxed_id = self.env.types.boxed(held);
                    let boxed = self.alloc(boxed_id);
                    self.write(Addr { base: boxed, offset: BOX_VALUE_OFFSET as i32 }, held_id, operand)?;
                    self.store_reference(boxed, building.at.at(8));
                } else {
                    self.write(building.at.at(8), held_id, operand)?;
                }
                self.built(building);
                Ok(())
            }
            Conversion::Narrow | Conversion::Adapt | Conversion::ToView => Err(internal(&format!("the conversion {conversion:?}, which the lowering expands"))),
        }
    }
}

fn size_repr(size: u32) -> Repr {
    if size == 0 { Repr::Nothing } else { Repr::Mem(size) }
}

/// The integer type of an enum's tag.
pub(crate) fn tag_type(tag_size: u32) -> Type {
    match tag_size {
        1 => types::I8,
        2 => types::I16,
        _ => types::I32,
    }
}
