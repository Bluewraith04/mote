//! Lowers mutexes, onces and atomics: a lock call goes to the runtime with the value in a stack slot, and an atomic is an instruction on the word of its handle.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, AtomicRmwOp, InstBuilder, MemFlags, Value};
use cranelift_module::FuncId as ClifFunc;
use mote_abi::{FaultKind, TypeId};
use mote_ir::{Inst, Operand};

use crate::function::Lowering;
use crate::scalar::Num;
use crate::values::{internal, Addr};
use crate::CompileError;

/// The `Int` an atomic holds.
const INT: Num = Num::Int { bits: 64, signed: true };

/// Where the value of an atomic lies in its handle.
const VALUE: i32 = 8;

impl Lowering<'_, '_, '_> {
    /// The type and the offset of the value a `Mutex` or `Once` of the type holds.
    fn held(&self, handle: TypeId) -> Result<(TypeId, u32), CompileError> {
        let field = self.env.types.table.get(handle).object.as_ref().and_then(|shape| shape.fields.first().copied());
        field.map(|field| (field.ty, field.offset)).ok_or_else(|| internal("a lock with no value"))
    }

    /// The rows of mutexes and onces.
    pub(crate) fn lock_intrinsic(&mut self, name: &str, inst: &Inst, args: &[Operand]) -> Result<(), CompileError> {
        match (name, args) {
            ("mutex_new", [value]) => self.lock_new(inst, self.env.imports.mutex_new, Some(value)),
            ("once_new", []) => self.lock_new(inst, self.env.imports.once_new, None),
            ("mutex_take", [lock]) => self.lock_read(inst, lock, self.env.imports.mutex_take),
            ("once_value", [cell]) => self.lock_read(inst, cell, self.env.imports.once_value),
            ("mutex_put", [lock, value]) => self.lock_write(lock, value, self.env.imports.mutex_put),
            ("once_finish", [cell, value]) => self.lock_write(cell, value, self.env.imports.once_finish),
            ("once_begin", [cell]) => {
                let handle = self.num_operand(cell, Num::Ref)?;
                let run = self.entry(self.env.imports.once_begin, &[handle])[0];
                let run = self.b.ins().ireduce(types::I8, run);
                self.set(inst.dest, Some(run));
                Ok(())
            }
            _ => Err(CompileError::internal(format!("the intrinsic `{name}` with these arguments"))),
        }
    }

    /// A new handle of the register's type, holding the value if there is one.
    fn lock_new(&mut self, inst: &Inst, import: ClifFunc, value: Option<&Operand>) -> Result<(), CompileError> {
        let handle_type = self.reg_type(Self::dest_reg(inst)?);
        let ty = self.b.ins().iconst(types::I32, i64::from(handle_type.raw()));
        let handle = self.entry(import, &[ty])[0];
        if let Some(value) = value {
            let (payload, offset) = self.held(handle_type)?;
            self.write(Addr { base: handle, offset: offset as i32 }, payload, value)?;
        }
        self.set(inst.dest, Some(handle));
        Ok(())
    }

    /// Takes the value of the handle through a stack slot into the destination.
    fn lock_read(&mut self, inst: &Inst, lock: &Operand, import: ClifFunc) -> Result<(), CompileError> {
        let (payload, _) = self.held(self.env.types.id(self.ty_of(lock)))?;
        let size = self.env.types.table.layout(payload).size.max(8);
        let slot = self.temp(size);
        let handle = self.num_operand(lock, Num::Ref)?;
        let dest = self.address(slot);
        self.entry(import, &[handle, dest]);
        self.read_into(inst.dest, slot, payload)
    }

    /// Gives the value to the handle through a stack slot.
    fn lock_write(&mut self, lock: &Operand, value: &Operand, import: ClifFunc) -> Result<(), CompileError> {
        let (payload, _) = self.held(self.env.types.id(self.ty_of(lock)))?;
        let at = self.spill_as(value, payload)?;
        let handle = self.num_operand(lock, Num::Ref)?;
        let source = self.address(at);
        self.entry(import, &[handle, source]);
        Ok(())
    }

    /// The operand as the word an atomic holds: an `Int` as it is, a `Bool` as 0 or 1.
    fn atomic_word(&mut self, flag: bool, operand: &Operand) -> Result<Value, CompileError> {
        if flag {
            let value = self.num_operand(operand, Num::Bool)?;
            Ok(self.b.ins().uextend(types::I64, value))
        } else {
            self.num_operand(operand, INT)
        }
    }

    /// The word of an atomic as the value the row gives.
    fn atomic_value(&mut self, flag: bool, word: Value) -> Value {
        if flag { self.b.ins().icmp_imm(IntCC::NotEqual, word, 0) } else { word }
    }

    /// The rows of atomic integers and flags, all sequentially consistent.
    pub(crate) fn atomic_intrinsic(&mut self, name: &str, inst: &Inst, args: &[Operand]) -> Result<(), CompileError> {
        let flag = name.starts_with("atomic_bool");
        let flags = MemFlags::trusted();
        if let ("atomic_int_new" | "atomic_bool_new", [value]) = (name, args) {
            let handle = self.alloc(self.reg_type(Self::dest_reg(inst)?));
            let word = self.atomic_word(flag, value)?;
            self.store(word, Addr { base: handle, offset: VALUE });
            self.set(inst.dest, Some(handle));
            return Ok(());
        }
        let Some(cell) = args.first() else { return Err(CompileError::internal(format!("the intrinsic `{name}` with these arguments"))) };
        let cell = self.num_operand(cell, Num::Ref)?;
        let at = self.b.ins().iadd_imm(cell, i64::from(VALUE));
        let operation = name.trim_start_matches("atomic_int_").trim_start_matches("atomic_bool_");
        match (operation, &args[1..]) {
            ("load", []) => {
                let word = self.b.ins().atomic_load(types::I64, flags, at);
                let value = self.atomic_value(flag, word);
                self.set(inst.dest, Some(value));
            }
            ("store", [value]) => {
                let word = self.atomic_word(flag, value)?;
                self.b.ins().atomic_store(flags, word, at);
            }
            ("swap", [value]) => {
                let word = self.atomic_word(flag, value)?;
                let old = self.b.ins().atomic_rmw(types::I64, flags, AtomicRmwOp::Xchg, at, word);
                let old = self.atomic_value(flag, old);
                self.set(inst.dest, Some(old));
            }
            ("compare_exchange", [expected, value]) => {
                let expected = self.atomic_word(flag, expected)?;
                let word = self.atomic_word(flag, value)?;
                let seen = self.b.ins().atomic_cas(flags, at, expected, word);
                let swapped = self.b.ins().icmp(IntCC::Equal, seen, expected);
                self.set(inst.dest, Some(swapped));
            }
            ("fetch_add" | "fetch_sub", [by]) => {
                let by = self.num_operand(by, INT)?;
                let (again, done) = (self.b.create_block(), self.b.create_block());
                self.b.ins().jump(again, &[]);
                self.b.switch_to_block(again);
                let old = self.b.ins().atomic_load(types::I64, flags, at);
                let (next, overflow) = if operation == "fetch_add" { self.b.ins().sadd_overflow(old, by) } else { self.b.ins().ssub_overflow(old, by) };
                self.fault_if(overflow, FaultKind::IntegerOverflow, inst.span);
                let seen = self.b.ins().atomic_cas(flags, at, old, next);
                let swapped = self.b.ins().icmp(IntCC::Equal, seen, old);
                self.b.ins().brif(swapped, done, &[], again, &[]);
                self.b.switch_to_block(done);
                self.set(inst.dest, Some(old));
            }
            _ => return Err(CompileError::internal(format!("the intrinsic `{name}` with these arguments"))),
        }
        Ok(())
    }
}
