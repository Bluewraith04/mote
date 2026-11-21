//! Lowers calls to external library functions: the C signature, the conversion of each argument and result, and `var` parameters.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, AbiParam, InstBuilder, MemFlags, Signature, Value};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::{DataId, Module as ClifModule};
use mote_abi::{CType, FaultKind, NativeSig, Scalar, ELEMENTS_OFFSET, LENGTH_OFFSET};
use mote_base::Span;
use mote_ir::{Inst, Operand, RegId};
use mote_semantics::{DefId, TyId, TyKind};

use crate::function::{clif_type, Lowering};
use mote_runtime::loader::native;
use crate::scalar::Num;
use crate::values::{emit_store, internal, Addr};
use crate::CompileError;

/// The C signature of an extern function; a narrow integer is widened by sign or by zero as the type says.
pub(crate) fn c_signature(sig: &NativeSig, call_conv: CallConv) -> Signature {
    let mut params = Vec::new();
    for param in &sig.params {
        if param.by_ref && param.ty != CType::FixedArray {
            params.push(AbiParam::new(types::I64));
            continue;
        }
        for &scalar in param.ty.scalars() {
            let abi = AbiParam::new(clif_type(scalar));
            params.push(match scalar {
                Scalar::I8 | Scalar::I16 | Scalar::I32 if param.ty.signed() => abi.sext(),
                Scalar::I8 | Scalar::I16 | Scalar::I32 => abi.uext(),
                _ => abi,
            });
        }
    }
    let returns = sig.result_scalars().iter().map(|&scalar| AbiParam::new(clif_type(scalar))).collect();
    Signature { params, returns, call_conv }
}

/// A value of a machine type as 64 bits.
fn to_bits(b: &mut FunctionBuilder, value: Value) -> Value {
    match b.func.dfg.value_type(value) {
        types::I64 => value,
        types::F64 => b.ins().bitcast(types::I64, MemFlags::new(), value),
        types::F32 => {
            let bits = b.ins().bitcast(types::I32, MemFlags::new(), value);
            b.ins().uextend(types::I64, bits)
        }
        _ => b.ins().uextend(types::I64, value),
    }
}

/// The 64 bits made by `to_bits` as a value of the scalar.
fn from_bits(b: &mut FunctionBuilder, bits: Value, scalar: Scalar) -> Value {
    match scalar {
        Scalar::I64 | Scalar::Ref => bits,
        Scalar::F64 => b.ins().bitcast(types::F64, MemFlags::new(), bits),
        Scalar::F32 => {
            let narrow = b.ins().ireduce(types::I32, bits);
            b.ins().bitcast(types::F32, MemFlags::new(), narrow)
        }
        other => b.ins().ireduce(clif_type(other), bits),
    }
}

/// The function that calls the external function at `index` of the table with its arguments and result in a block of 64-bit words, for a call that runs on a helper thread.
pub(crate) fn thunk(b: &mut FunctionBuilder, jit: &dyn ClifModule, sig: &NativeSig, slots: DataId, index: usize, call_conv: CallConv) {
    let entry = b.create_block();
    b.append_block_params_for_function_params(entry);
    b.switch_to_block(entry);
    let buffer = b.block_params(entry)[0];
    let table = jit.declare_data_in_func(slots, b.func);
    let base = b.ins().symbol_value(types::I64, table);
    let target = b.ins().load(types::I64, MemFlags::trusted(), base, (index * 8) as i32);
    let mut arguments = Vec::new();
    let mut word = 0;
    for param in &sig.params {
        for &scalar in param.scalars() {
            let bits = b.ins().load(types::I64, MemFlags::trusted(), buffer, word * 8);
            arguments.push(from_bits(b, bits, scalar));
            word += 1;
        }
    }
    let reference = b.import_signature(c_signature(sig, call_conv));
    let call = b.ins().call_indirect(reference, target, &arguments);
    if let Some(&result) = b.inst_results(call).first() {
        let bits = to_bits(b, result);
        emit_store(b, bits, Addr { base: buffer, offset: 0 });
    }
    b.ins().return_(&[]);
    b.seal_all_blocks();
}

/// A `var` argument passed by pointer: where the callee left the value, and what it goes back to.
struct ByRef {
    out: RegId,
    ctype: CType,
    ty: TyId,
    at: Addr,
}

impl Lowering<'_, '_, '_> {
    pub(crate) fn native_call(&mut self, inst: &Inst, def: DefId, args: &[Operand], outs: &[RegId]) -> Result<(), CompileError> {
        let index = *self.env.natives.get(&def).ok_or_else(|| CompileError::internal("an extern fn with no C signature"))?;
        let sig = self.env.link.functions[index].sig.clone();
        let function = self.env.program.defs[def].function().ok_or_else(|| internal("an extern entry that is not a function"))?;
        let (declared, ret): (Vec<TyId>, TyId) = (function.params.iter().map(|param| param.ty).collect(), function.ret);

        let mut values = Vec::new();
        let mut owned = Vec::new();
        let mut copies = Vec::new();
        let mut outs = outs.iter();
        for ((param, &ty), arg) in sig.params.iter().zip(&declared).zip(args) {
            if param.by_ref {
                let out = *outs.next().ok_or_else(|| internal("a var parameter with no result"))?;
                if param.ty == CType::FixedArray {
                    let (at, ptr, length) = self.fixed_array_pair(ty, arg)?;
                    values.push(ptr);
                    values.push(length);
                    copies.push(ByRef { out, ctype: param.ty, ty, at });
                } else {
                    let converted = self.c_arguments(param.ty, ty, arg, &mut owned)?;
                    let at = self.temp(8);
                    self.store(converted[0], at);
                    values.push(self.address(at));
                    copies.push(ByRef { out, ctype: param.ty, ty, at });
                }
            } else {
                let converted = self.c_arguments(param.ty, ty, arg, &mut owned)?;
                values.extend(converted);
            }
        }

        let results = if sig.blocking { self.blocking_call(index, &sig, &values)? } else { self.direct_call(index, &sig, &values)? };
        for copy in copies {
            if copy.ctype == CType::FixedArray {
                let id = self.env.types.id(copy.ty);
                self.read_into(Some(copy.out), copy.at, id)?;
            } else {
                let scalar = copy.ctype.scalars()[0];
                let value = self.load(clif_type(scalar), copy.at);
                self.take_c(copy.ctype, copy.ty, value, Some(copy.out), None)?;
            }
        }
        if let Some(&result) = results.first() {
            self.take_c(sig.ret, ret, result, inst.dest, Some(inst.span))?;
        }
        for pointer in owned {
            self.native_helper(native::FREE, pointer, None);
        }
        Ok(())
    }

    /// A call of `mote_native`.
    fn native_helper(&mut self, code: u32, a: Value, b: Option<Value>) -> Value {
        let code = self.b.ins().iconst(types::I32, i64::from(code));
        let b = b.unwrap_or_else(|| self.b.ins().iconst(types::I64, 0));
        let call = self.func_ref(self.env.imports.native);
        let result = self.b.ins().call(call, &[self.context, code, a, b]);
        self.b.inst_results(result)[0]
    }

    /// The address of the external function at `index`, read from the table the loader fills.
    fn native_target(&mut self, index: usize) -> Result<Value, CompileError> {
        let slots = self.env.native_slots.ok_or_else(|| internal("external calls with no table"))?;
        let table = self.env.clif.declare_data_in_func(slots, self.b.func);
        let base = self.b.ins().symbol_value(types::I64, table);
        Ok(self.load(types::I64, Addr { base, offset: (index * 8) as i32 }))
    }

    fn direct_call(&mut self, index: usize, sig: &NativeSig, values: &[Value]) -> Result<Vec<Value>, CompileError> {
        let target = self.native_target(index)?;
        let reference = self.b.import_signature(c_signature(sig, self.env.clif.isa().default_call_conv()));
        let call = self.b.ins().call_indirect(reference, target, values);
        Ok(self.b.inst_results(call).to_vec())
    }

        fn blocking_call(&mut self, index: usize, sig: &NativeSig, values: &[Value]) -> Result<Vec<Value>, CompileError> {
        let thunk = *self.env.thunks.get(&index).ok_or_else(|| internal("a blocking extern fn with no thunk"))?;
        let buffer = self.temp(8 * values.len().max(1) as u32);
        for (position, &value) in values.iter().enumerate() {
            let bits = to_bits(&mut self.b, value);
            self.store(bits, buffer.at(8 * position as u32));
        }
        let reference = self.func_ref(thunk);
        let code = self.b.ins().func_addr(types::I64, reference);
        let at = self.address(buffer);
        let words = self.b.ins().iconst(types::I64, values.len().max(1) as i64);
        self.entry(self.env.imports.native_blocking, &[code, at, words]);
        let Some(&scalar) = sig.result_scalars().first() else { return Ok(Vec::new()) };
        let bits = self.load(types::I64, buffer);
        Ok(vec![from_bits(&mut self.b, bits, scalar)])
    }

    /// Spills a fixed-size array argument to a stack slot and returns its slot address, element pointer, and element count.
    fn fixed_array_pair(&mut self, ty: TyId, arg: &Operand) -> Result<(Addr, Value, Value), CompileError> {
        let id = self.env.types.id(ty);
        let at = self.spill_as(arg, id)?;
        let ptr = self.address(at);
        let count = match self.env.program.types.kind(ty) {
            TyKind::FixedArray(_, count) => count.as_u32().ok_or_else(|| internal("an unresolved fixed array length"))?,
            _ => return Err(internal("a fixed array C parameter that is not a fixed array")),
        };
        let length = self.b.ins().iconst(types::I64, i64::from(count));
        Ok((at, ptr, length))
    }

    /// The machine values an argument of the C type is passed as; a copy of a string is added to `owned` to be freed after the call.
    fn c_arguments(&mut self, ctype: CType, ty: TyId, arg: &Operand, owned: &mut Vec<Value>) -> Result<Vec<Value>, CompileError> {
        Ok(match ctype {
            CType::Void | CType::StringOut => return Err(internal("a parameter that is a result type")),
            CType::PortEnd => {
                let port = self.num_operand(arg, Num::Ref)?;
                self.entry(self.env.imports.port_ref, &[port]).to_vec()
            }
            CType::StringIn => {
                let string = self.num_operand(arg, Num::Ref)?;
                let copy = self.native_helper(native::IN, string, None);
                owned.push(copy);
                vec![copy]
            }
            CType::Array => {
                let array = self.num_operand(arg, Num::Ref)?;
                let length = self.load(types::I64, Addr { base: array, offset: LENGTH_OFFSET as i32 });
                let elements = self.b.ins().iadd_imm(array, i64::from(ELEMENTS_OFFSET));
                vec![elements, length]
            }
            CType::FixedArray => {
                let (_, ptr, length) = self.fixed_array_pair(ty, arg)?;
                vec![ptr, length]
            }
            CType::Pointer => {
                let id = self.env.types.id(ty);
                let at = self.spill_as(arg, id)?;
                vec![self.load(types::I64, at)]
            }
            CType::NullablePointer => {
                let id = self.env.types.id(ty);
                let at = self.spill_as(arg, id)?;
                let field = *self.env.types.table.get(id).variants.get(1).and_then(|variant| variant.fields.first()).ok_or_else(|| internal("a nullable pointer that is not an optional"))?;
                let tag = self.load(types::I64, at);
                let address = self.load(types::I64, at.at(field.offset));
                let none = self.b.ins().icmp_imm(IntCC::Equal, tag, 0);
                let zero = self.b.ins().iconst(types::I64, 0);
                vec![self.b.ins().select(none, zero, address)]
            }
            _ => {
                let value = self.operand(arg, ty)?.ok_or_else(|| internal("a number with no value"))?;
                vec![value]
            }
        })
    }

    /// Stores a value the callee gave in the register for its Mote type; a result faults for a null `Pointer` and a full heap.
    fn take_c(&mut self, ctype: CType, ty: TyId, value: Value, dest: Option<RegId>, result_of: Option<Span>) -> Result<(), CompileError> {
        match ctype {
            CType::Void => {}
            CType::StringOut => {
                let text = self.native_helper(native::OUT, value, None);
                let full = self.b.ins().icmp_imm(IntCC::Equal, text, 0);
                self.fault_if(full, FaultKind::MemoryLimit, result_of.unwrap_or_default());
                self.set(dest, Some(text));
            }
            CType::Pointer | CType::NullablePointer => {
                if let (CType::Pointer, Some(span)) = (ctype, result_of) {
                    let null = self.b.ins().icmp_imm(IntCC::Equal, value, 0);
                    self.fault_if(null, FaultKind::NullPointer, span);
                }
                let dest = dest.ok_or_else(|| internal("a pointer with no register"))?;
                let at = self.reg_addr(dest)?;
                if ctype == CType::NullablePointer {
                    let id = self.reg_type(dest);
                    let field = *self.env.types.table.get(id).variants.get(1).and_then(|variant| variant.fields.first()).ok_or_else(|| internal("a nullable pointer that is not an optional"))?;
                    let present = self.b.ins().icmp_imm(IntCC::NotEqual, value, 0);
                    let tag = self.b.ins().uextend(types::I64, present);
                    self.store(tag, at);
                    self.store(value, at.at(field.offset));
                } else {
                    self.store(value, at);
                }
            }
            _ => {
                let value = if ty == TyId::BOOL { self.b.ins().icmp_imm(IntCC::NotEqual, value, 0) } else { value };
                self.set(dest, Some(value));
            }
        }
        Ok(())
    }
}
