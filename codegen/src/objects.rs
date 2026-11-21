//! Lowers allocation, classes, strings and the references they hold.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, InstBuilder, Value};
use mote_abi::{inline_size, Header, TypeId, CONTEXT_REGION_CURSOR, CONTEXT_REGION_LIMIT, ELEMENTS_OFFSET, LENGTH_OFFSET};
use mote_base::Span;
use mote_ir::{Const, Inst, Operand};
use mote_semantics::{BinaryOp, TyId, TyKind};

use crate::function::Lowering;
use mote_runtime::helpers::{shown, SHOW_BUFFER};
use crate::scalar::Num;
use crate::values::Addr;
use crate::CompileError;

impl Lowering<'_, '_, '_> {
    /// A new zeroed object of the type: bumped from the task's region when the type is small and fixed in size and the region has room, else from the runtime.
    pub(crate) fn alloc(&mut self, id: TypeId) -> Value {
        let size = inline_size(&self.env.types.table, id);
        let inline = size.map(|size| {
            let cursor = self.load(types::I64, Addr { base: self.context, offset: CONTEXT_REGION_CURSOR as i32 });
            let limit = self.load(types::I64, Addr { base: self.context, offset: CONTEXT_REGION_LIMIT as i32 });
            let room = self.b.ins().isub(limit, cursor);
            let fits = self.b.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, room, i64::from(size));
            let (bump, slow, done) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
            self.b.append_block_param(done, types::I64);
            self.b.set_cold_block(slow);
            self.b.ins().brif(fits, bump, &[], slow, &[]);
            self.b.switch_to_block(bump);
            let end = self.b.ins().iadd_imm(cursor, i64::from(size));
            self.store(end, Addr { base: self.context, offset: CONTEXT_REGION_CURSOR as i32 });
            let header = self.b.ins().iconst(types::I64, Header::new(id, 0).bits() as i64);
            self.store(header, Addr { base: cursor, offset: 0 });
            self.b.ins().jump(done, &[cursor]);
            self.b.switch_to_block(slow);
            done
        });
        let alloc = self.func_ref(self.env.imports.alloc);
        let ty = self.b.ins().iconst(types::I32, i64::from(id.raw()));
        let call = self.b.ins().call(alloc, &[self.context, ty]);
        let object = self.b.inst_results(call)[0];
        let Some(done) = inline else { return object };
        self.b.ins().jump(done, &[object]);
        self.b.switch_to_block(done);
        self.b.block_params(done)[0]
    }

    /// A new zeroed string or array of the type with room for `count` elements, whose length word is set.
    pub(crate) fn alloc_array(&mut self, id: TypeId, count: Value) -> Value {
        let alloc = self.func_ref(self.env.imports.alloc_array);
        let ty = self.b.ins().iconst(types::I32, i64::from(id.raw()));
        let call = self.b.ins().call(alloc, &[self.context, ty, count]);
        self.b.inst_results(call)[0]
    }

    /// Every store of a reference is emitted here.
    pub(crate) fn store_reference(&mut self, value: Value, at: Addr) {
        self.store(value, at);
    }

    fn string_type(&self) -> TypeId {
        self.env.types.id(TyId::STRING)
    }

    /// Copies `length` bytes, which do not overlap.
    pub(crate) fn copy_bytes(&mut self, to: Value, from: Value, length: Value) {
        let config = self.env.clif.isa().frontend_config();
        self.b.call_memcpy(config, to, from, length);
    }

    /// Copies `length` bytes, which may overlap.
    pub(crate) fn move_bytes(&mut self, to: Value, from: Value, length: Value) {
        let config = self.env.clif.isa().frontend_config();
        self.b.call_memmove(config, to, from, length);
    }

    /// A new string of the parts, which are strings, in order.
    pub(crate) fn concat(&mut self, parts: &[Operand]) -> Result<Value, CompileError> {
        let mut held = Vec::new();
        let mut total = self.b.ins().iconst(types::I64, 0);
        for part in parts {
            let string = self.num_operand(part, Num::Ref)?;
            let length = self.load(types::I64, Addr { base: string, offset: LENGTH_OFFSET as i32 });
            total = self.b.ins().iadd(total, length);
            held.push((string, length));
        }
        let joined = self.alloc_array(self.string_type(), total);
        let mut cursor = self.b.ins().iconst(types::I64, 0);
        for (string, length) in held {
            let to = self.b.ins().iadd(joined, cursor);
            let to = self.b.ins().iadd_imm(to, i64::from(ELEMENTS_OFFSET));
            let from = self.b.ins().iadd_imm(string, i64::from(ELEMENTS_OFFSET));
            self.copy_bytes(to, from, length);
            cursor = self.b.ins().iadd(cursor, length);
        }
        Ok(joined)
    }

    /// Any value as a new string; a string is itself.
    pub(crate) fn show(&mut self, operand: &Operand, span: Span) -> Result<Value, CompileError> {
        if !self.is_plain(operand) {
            return self.show_value(operand, span);
        }
        let (bits, kind) = self.shown_bits(operand)?;
        if kind == shown::STRING {
            return Ok(bits);
        }
        let scratch = self.temp(SHOW_BUFFER);
        let buffer = self.address(scratch);
        let kind = self.b.ins().iconst(types::I32, i64::from(kind));
        let show = self.func_ref(self.env.imports.show);
        let call = self.b.ins().call(show, &[bits, kind, buffer]);
        let length = self.b.inst_results(call)[0];
        let string = self.alloc_array(self.string_type(), length);
        let to = self.b.ins().iadd_imm(string, i64::from(ELEMENTS_OFFSET));
        self.copy_bytes(to, buffer, length);
        Ok(string)
    }

    /// `+` and the comparisons on strings, and `==` and `!=` on references; `None` for operands of another kind.
    pub(crate) fn reference_binary(&mut self, op: BinaryOp, left: &Operand, right: &Operand) -> Result<Option<Value>, CompileError> {
        let ty = self.operand_ty(&[left, right]);
        if matches!(self.env.program.types.kind(ty), TyKind::String) {
            if op == BinaryOp::Add {
                return Ok(Some(self.concat(&[*left, *right])?));
            }
            let cc = match op {
                BinaryOp::Eq => IntCC::Equal,
                BinaryOp::NotEq => IntCC::NotEqual,
                BinaryOp::Lt => IntCC::SignedLessThan,
                BinaryOp::LtEq => IntCC::SignedLessThanOrEqual,
                BinaryOp::Gt => IntCC::SignedGreaterThan,
                BinaryOp::GtEq => IntCC::SignedGreaterThanOrEqual,
                _ => return Err(CompileError::internal("this String operation")),
            };
            let (a, b) = (self.num_operand(left, Num::Ref)?, self.num_operand(right, Num::Ref)?);
            let compare = self.func_ref(self.env.imports.compare);
            let call = self.b.ins().call(compare, &[a, b]);
            let order = self.b.inst_results(call)[0];
            return Ok(Some(self.b.ins().icmp_imm(cc, order, 0)));
        }
        if self.repr(ty)? == crate::values::Repr::Num(Num::Ref) && matches!(op, BinaryOp::Eq | BinaryOp::NotEq) {
            let id = self.env.types.id(ty);
            let null = matches!(left, Operand::Const(Const::Null)) || matches!(right, Operand::Const(Const::Null));
            if !null && (self.env.types.is_map(id) || !matches!(self.env.types.table.get(id).kind, mote_abi::Kind::Class(_) | mote_abi::Kind::Closure(_) | mote_abi::Kind::Handle(..))) {
                return Ok(None);
            }
            let (a, b) = (self.num_operand(left, Num::Ref)?, self.num_operand(right, Num::Ref)?);
            let cc = if op == BinaryOp::Eq { IntCC::Equal } else { IntCC::NotEqual };
            return Ok(Some(self.b.ins().icmp(cc, a, b)));
        }
        Ok(None)
    }

    /// `Detach`: a copy of the value, deep when it holds references.
    pub(crate) fn detach(&mut self, inst: &Inst, operand: &Operand) -> Result<(), CompileError> {
        let dest = inst.dest.ok_or_else(|| CompileError::internal("a detach with no destination"))?;
        let info = self.env.types.table.get(self.reg_type(dest));
        if !info.slots.is_empty() && !matches!(info.kind, mote_abi::Kind::String) {
            return self.deep_clone(inst, operand);
        }
        self.copy_into(dest, operand)
    }
}
