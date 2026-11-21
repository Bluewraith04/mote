//! Lowers off-heap pointer and manual allocation intrinsics.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, InstBuilder, Value};
use mote_abi::{align_up, FaultKind, TypeId};
use mote_ir::{Inst, Operand, RegId};
use mote_runtime::helpers::mem;
use mote_semantics::{BuiltinTy, TyId, TyKind};

use crate::function::Lowering;
use crate::values::{internal, Addr};
use crate::CompileError;

impl Lowering<'_, '_, '_> {
    fn pointer_elem(&self, ty: TyId) -> Result<(TypeId, u32, u32), CompileError> {
        let TyKind::Builtin { kind: BuiltinTy::Pointer, args } = self.env.program.types.kind(ty) else {
            return Err(internal("expected Pointer<T>"));
        };
        let &[elem] = args.as_slice() else {
            return Err(internal("Pointer<T> with missing element type"));
        };
        let id = self.env.types.id(elem);
        let layout = self.env.types.table.layout(id);
        let align = layout.align.max(1);
        let stride = align_up(layout.size, align);
        Ok((id, stride, align))
    }

    fn ptr_addr(&mut self, op: &Operand) -> Result<(TyId, Value), CompileError> {
        let ty = self.ty_of(op);
        let id = self.env.types.id(ty);
        let leaves = self.operand_leaves(op, id)?;
        let &[addr] = leaves.as_slice() else {
            return Err(internal("Pointer<T> expected one address word"));
        };
        Ok((ty, addr))
    }

    fn store_ptr(&mut self, dest: Option<RegId>, id: TypeId, addr: Value) -> Result<(), CompileError> {
        let Some(reg) = dest else { return Ok(()) };
        self.store_leaves(reg, id, &[addr])
    }

    fn call_mem(&mut self, op: u32, a: Value, b: Value, c: Value, d: Value) -> Value {
        let func = self.func_ref(self.env.imports.mem);
        let op = self.b.ins().iconst(types::I32, i64::from(op));
        let call = self.b.ins().call(func, &[op, a, b, c, d]);
        self.b.inst_results(call)[0]
    }

    pub(crate) fn mem_intrinsic(&mut self, name: &str, inst: &Inst, args: &[Operand]) -> Result<(), CompileError> {
        match (name, args) {
            ("ptr_null", []) => {
                if let Some(dest) = inst.dest {
                    let id = self.reg_type(dest);
                    let zero = self.b.ins().iconst(types::I64, 0);
                    self.store_ptr(Some(dest), id, zero)?;
                }
            }
            ("ptr_from_addr", [addr_op]) => {
                let addr = self.operand(addr_op, TyId::UINT64)?.expect("address word");
                if let Some(dest) = inst.dest {
                    let id = self.reg_type(dest);
                    self.store_ptr(Some(dest), id, addr)?;
                }
            }
            ("ptr_alloc", [count_op]) => {
                let Some(dest) = inst.dest else { return Ok(()) };
                let ptr_ty = self.function.regs[dest].ty;
                let ptr_id = self.env.types.id(ptr_ty);
                let (_, stride, align) = self.pointer_elem(ptr_ty)?;
                let count = self.operand(count_op, TyId::INT)?.expect("allocation count");
                let neg = self.b.ins().icmp_imm(IntCC::SignedLessThan, count, 0);
                self.fault_if(neg, FaultKind::IndexOutOfRange, inst.span);
                let stride_val = self.b.ins().iconst(types::I64, i64::from(stride));
                let align_val = self.b.ins().iconst(types::I64, i64::from(align));
                let zero = self.b.ins().iconst(types::I64, 0);
                let ptr_val = self.call_mem(mem::ALLOC, count, stride_val, align_val, zero);
                let pos = self.b.ins().icmp_imm(IntCC::SignedGreaterThan, count, 0);
                let null_res = self.b.ins().icmp_imm(IntCC::Equal, ptr_val, 0);
                let oom = self.b.ins().band(pos, null_res);
                self.fault_if(oom, FaultKind::MemoryLimit, inst.span);
                self.store_ptr(Some(dest), ptr_id, ptr_val)?;
            }
            ("ptr_realloc", [ptr_op, count_op]) => {
                let (ptr_ty, old_addr) = self.ptr_addr(ptr_op)?;
                let ptr_id = self.env.types.id(ptr_ty);
                let (_, stride, align) = self.pointer_elem(ptr_ty)?;
                let count = self.operand(count_op, TyId::INT)?.expect("reallocation count");
                let neg = self.b.ins().icmp_imm(IntCC::SignedLessThan, count, 0);
                self.fault_if(neg, FaultKind::IndexOutOfRange, inst.span);
                let stride_val = self.b.ins().iconst(types::I64, i64::from(stride));
                let align_val = self.b.ins().iconst(types::I64, i64::from(align));
                let ptr_val = self.call_mem(mem::REALLOC, old_addr, count, stride_val, align_val);
                let pos = self.b.ins().icmp_imm(IntCC::SignedGreaterThan, count, 0);
                let null_res = self.b.ins().icmp_imm(IntCC::Equal, ptr_val, 0);
                let oom = self.b.ins().band(pos, null_res);
                self.fault_if(oom, FaultKind::MemoryLimit, inst.span);
                self.store_ptr(inst.dest, ptr_id, ptr_val)?;
            }
            ("ptr_free", [ptr_op]) => {
                let (_, addr) = self.ptr_addr(ptr_op)?;
                let zero = self.b.ins().iconst(types::I64, 0);
                self.call_mem(mem::FREE, addr, zero, zero, zero);
            }
            ("ptr_offset", [ptr_op, delta_op]) => {
                let (ptr_ty, addr) = self.ptr_addr(ptr_op)?;
                let ptr_id = self.env.types.id(ptr_ty);
                let (_, stride, _) = self.pointer_elem(ptr_ty)?;
                let is_null = self.b.ins().icmp_imm(IntCC::Equal, addr, 0);
                self.fault_if(is_null, FaultKind::NullPointer, inst.span);
                let delta = self.operand(delta_op, TyId::INT)?.expect("pointer offset");
                let byte_delta = self.b.ins().imul_imm(delta, i64::from(stride));
                let next = self.b.ins().iadd(addr, byte_delta);
                self.store_ptr(inst.dest, ptr_id, next)?;
            }
            ("ptr_load", [ptr_op, index_op]) => {
                let (ptr_ty, addr) = self.ptr_addr(ptr_op)?;
                let (elem_id, stride, _) = self.pointer_elem(ptr_ty)?;
                let is_null = self.b.ins().icmp_imm(IntCC::Equal, addr, 0);
                self.fault_if(is_null, FaultKind::NullPointer, inst.span);
                let index = self.operand(index_op, TyId::INT)?.expect("pointer index");
                let byte_off = self.b.ins().imul_imm(index, i64::from(stride));
                let elem_addr = self.b.ins().iadd(addr, byte_off);
                self.read_into(inst.dest, Addr { base: elem_addr, offset: 0 }, elem_id)?;
            }
            ("ptr_store", [ptr_op, index_op, val_op]) => {
                let (ptr_ty, addr) = self.ptr_addr(ptr_op)?;
                let (elem_id, stride, _) = self.pointer_elem(ptr_ty)?;
                let is_null = self.b.ins().icmp_imm(IntCC::Equal, addr, 0);
                self.fault_if(is_null, FaultKind::NullPointer, inst.span);
                let index = self.operand(index_op, TyId::INT)?.expect("pointer index");
                let byte_off = self.b.ins().imul_imm(index, i64::from(stride));
                let elem_addr = self.b.ins().iadd(addr, byte_off);
                self.write(Addr { base: elem_addr, offset: 0 }, elem_id, val_op)?;
            }
            ("ptr_copy", [dst_op, src_op, count_op]) => {
                let (ptr_ty, dst) = self.ptr_addr(dst_op)?;
                let (_, src) = self.ptr_addr(src_op)?;
                let (_, stride, _) = self.pointer_elem(ptr_ty)?;
                let count = self.operand(count_op, TyId::INT)?.expect("copy count");
                let neg = self.b.ins().icmp_imm(IntCC::SignedLessThan, count, 0);
                self.fault_if(neg, FaultKind::IndexOutOfRange, inst.span);
                let pos = self.b.ins().icmp_imm(IntCC::SignedGreaterThan, count, 0);
                let dst_null = self.b.ins().icmp_imm(IntCC::Equal, dst, 0);
                let src_null = self.b.ins().icmp_imm(IntCC::Equal, src, 0);
                let either_null = self.b.ins().bor(dst_null, src_null);
                let bad_null = self.b.ins().band(pos, either_null);
                self.fault_if(bad_null, FaultKind::NullPointer, inst.span);
                let stride_val = self.b.ins().iconst(types::I64, i64::from(stride));
                let err = self.call_mem(mem::COPY, dst, src, count, stride_val);
                let overflow = self.b.ins().icmp_imm(IntCC::NotEqual, err, 0);
                self.fault_if(overflow, FaultKind::IntegerOverflow, inst.span);
            }
            ("ptr_zero", [dst_op, count_op]) => {
                let (ptr_ty, dst) = self.ptr_addr(dst_op)?;
                let (_, stride, _) = self.pointer_elem(ptr_ty)?;
                let count = self.operand(count_op, TyId::INT)?.expect("zero count");
                let neg = self.b.ins().icmp_imm(IntCC::SignedLessThan, count, 0);
                self.fault_if(neg, FaultKind::IndexOutOfRange, inst.span);
                let pos = self.b.ins().icmp_imm(IntCC::SignedGreaterThan, count, 0);
                let dst_null = self.b.ins().icmp_imm(IntCC::Equal, dst, 0);
                let bad_null = self.b.ins().band(pos, dst_null);
                self.fault_if(bad_null, FaultKind::NullPointer, inst.span);
                let stride_val = self.b.ins().iconst(types::I64, i64::from(stride));
                let zero = self.b.ins().iconst(types::I64, 0);
                let err = self.call_mem(mem::ZERO, dst, count, stride_val, zero);
                let overflow = self.b.ins().icmp_imm(IntCC::NotEqual, err, 0);
                self.fault_if(overflow, FaultKind::IntegerOverflow, inst.span);
            }
            ("ptr_cast", [ptr_op]) => {
                let (_, addr) = self.ptr_addr(ptr_op)?;
                if let Some(dest) = inst.dest {
                    let dst_id = self.reg_type(dest);
                    self.store_ptr(Some(dest), dst_id, addr)?;
                }
            }
            _ => return Err(internal(&format!("unknown memory intrinsic `{name}`"))),
        }
        Ok(())
    }
}
