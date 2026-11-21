//! Lowers the `num_*`, `float_*` and `math_*` intrinsics.

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{types, InstBuilder, Value};
use mote_ir::{Inst, Operand};
use mote_semantics::{TyId, TyKind};

use crate::function::{class, sign_extended, Lowering};
use mote_runtime::helpers::math;
use crate::scalar::Num;
use crate::values::{internal, Addr};
use crate::CompileError;

/// The result of an integer operation done in a register: the value cut to the width, whether it was lost, and the exact value when the width is under 64 bits.
struct Done {
    wrapped: Value,
    overflow: Value,
    exact: Option<Value>,
}

/// An integer type: its width and whether it is signed.
#[derive(Clone, Copy)]
struct Width {
    bits: u32,
    signed: bool,
}

impl Width {
    fn num(self) -> Num {
        Num::Int { bits: self.bits, signed: self.signed }
    }

    fn min(self) -> i128 {
        if self.signed { -(1i128 << (self.bits - 1)) } else { 0 }
    }

    fn max(self) -> i128 {
        if self.signed { (1i128 << (self.bits - 1)) - 1 } else { (1i128 << self.bits) - 1 }
    }
}

impl Lowering<'_, '_, '_> {
    pub(crate) fn numeric_intrinsic(&mut self, name: &str, inst: &Inst, args: &[Operand]) -> Result<(), CompileError> {
        match (name, args) {
            ("num_to_string", [number]) => {
                let text = self.show(number, inst.span)?;
                self.set(inst.dest, Some(text));
                Ok(())
            }
            ("float_is_nan", [x]) => {
                let x = self.float_operand(x)?;
                let nan = self.b.ins().fcmp(FloatCC::Unordered, x, x);
                self.set(inst.dest, Some(nan));
                Ok(())
            }
            ("float_is_infinite", [x]) => {
                let x = self.float_operand(x)?;
                let magnitude = self.b.ins().fabs(x);
                let infinity = if self.b.func.dfg.value_type(x) == types::F32 { self.b.ins().f32const(f32::INFINITY) } else { self.b.ins().f64const(f64::INFINITY) };
                let is = self.b.ins().fcmp(FloatCC::Equal, magnitude, infinity);
                self.set(inst.dest, Some(is));
                Ok(())
            }
            ("float_to_int", [x]) => {
                let x = self.num_operand(x, Num::F64)?;
                let int = self.cast(x, Num::F64, Num::Int { bits: 64, signed: true })?;
                self.set(inst.dest, Some(int));
                Ok(())
            }
            (_, _) if name.starts_with("math_") => self.math(&name["math_".len()..], inst, args),
            (_, _) if name.starts_with("num_") => {
                let (rule, op) = name["num_".len()..].split_once('_').ok_or_else(|| internal("a number intrinsic with no rule"))?;
                self.int_rule(rule, op, inst, args)
            }
            _ => Err(CompileError::internal(format!("the intrinsic `{name}` with these arguments"))),
        }
    }

    /// A float operand in its own width.
    fn float_operand(&mut self, operand: &Operand) -> Result<Value, CompileError> {
        match class(self.env.program, self.ty_of(operand))? {
            Some(num @ (Num::F32 | Num::F64)) => self.num_operand(operand, num),
            _ => Err(CompileError::internal("a float operation on this value")),
        }
    }

    fn math(&mut self, name: &str, inst: &Inst, args: &[Operand]) -> Result<(), CompileError> {
        let x = self.num_operand(&args[0], Num::F64)?;
        let result = match name {
            "sqrt" => self.b.ins().sqrt(x),
            "floor" => self.b.ins().floor(x),
            "ceil" => self.b.ins().ceil(x),
            other => {
                let code = match other {
                    "round" => math::ROUND,
                    "exp" => math::EXP,
                    "ln" => math::LN,
                    "log10" => math::LOG10,
                    "sin" => math::SIN,
                    "cos" => math::COS,
                    "tan" => math::TAN,
                    "pow" => math::POW,
                    "atan2" => math::ATAN2,
                    _ => return Err(CompileError::internal(format!("the intrinsic `math_{other}`"))),
                };
                let y = match args.get(1) {
                    Some(y) => self.num_operand(y, Num::F64)?,
                    None => self.b.ins().f64const(0.0),
                };
                let code = self.b.ins().iconst(types::I32, i64::from(code));
                let call = self.func_ref(self.env.imports.math);
                let call = self.b.ins().call(call, &[code, x, y]);
                self.b.inst_results(call)[0]
            }
        };
        self.set(inst.dest, Some(result));
        Ok(())
    }

    /// The integer type an operation works on: the result's, else the first argument's.
    fn number_ty(&self, inst: &Inst, args: &[Operand]) -> Result<Width, CompileError> {
        let found = inst.dest.and_then(|dest| self.number_in(self.function.regs[dest].ty)).or_else(|| args.first().and_then(|arg| self.number_in(self.ty_of(arg))));
        match found.map(|ty| class(self.env.program, ty)).transpose()? {
            Some(Some(Num::Int { bits, signed })) => Ok(Width { bits, signed }),
            _ => Err(CompileError::internal("a number operation on this type")),
        }
    }

    fn number_in(&self, ty: TyId) -> Option<TyId> {
        match self.env.program.types.kind(ty) {
            TyKind::Int(_) | TyKind::Float(_) => Some(ty),
            TyKind::Optional(inner) => self.number_in(*inner),
            TyKind::Tuple(items) => items.first().and_then(|&item| self.number_in(item)),
            _ => None,
        }
    }

    fn int_rule(&mut self, rule: &str, op: &str, inst: &Inst, args: &[Operand]) -> Result<(), CompileError> {
        let width = self.number_ty(inst, args)?;
        let a = self.num_operand(&args[0], width.num())?;
        let shifting = matches!(op, "shl" | "shr");
        let b = match args.get(1) {
            Some(arg) if shifting => Some(self.num_operand(arg, Num::Int { bits: 64, signed: true })?),
            Some(arg) => Some(self.num_operand(arg, width.num())?),
            None => None,
        };
        let done = self.outcome(op, a, b, width)?;
        let ty = width.num().clif();
        match rule {
            "wrapping" => self.set(inst.dest, Some(done.wrapped)),
            "saturating" => {
                let value = self.saturated(op, a, b, &done, width)?;
                self.set(inst.dest, Some(value));
            }
            "checked" => {
                let dest = inst.dest.ok_or_else(|| internal("a checked operation with no destination"))?;
                let info = self.env.types.table.get(self.reg_type(dest));
                let field = *info.variants.get(1).and_then(|variant| variant.fields.first()).ok_or_else(|| internal("a checked result that is not an optional"))?;
                let at = self.reg_addr(dest)?;
                let some = self.b.ins().bxor_imm(done.overflow, 1);
                let tag = self.b.ins().uextend(types::I64, some);
                self.store(tag, at);
                let zero = self.b.ins().iconst(ty, 0);
                let payload = self.b.ins().select(done.overflow, zero, done.wrapped);
                self.store(payload, Addr { base: at.base, offset: at.offset + field.offset as i32 });
            }
            "overflowing" => {
                let dest = inst.dest.ok_or_else(|| internal("an overflowing operation with no destination"))?;
                let info = self.env.types.table.get(self.reg_type(dest));
                let [value, flag] = info.fields[..] else { return Err(internal("an overflowing result that is not a pair")) };
                let at = self.reg_addr(dest)?;
                self.store(done.wrapped, at.at(value.offset));
                self.store(done.overflow, at.at(flag.offset));
            }
            _ => return Err(CompileError::internal(format!("the intrinsic `num_{rule}_{op}`"))),
        }
        Ok(())
    }

    /// The value of an operation cut to the width of the type, and whether the cut lost something.
    fn outcome(&mut self, op: &str, a: Value, b: Option<Value>, width: Width) -> Result<Done, CompileError> {
        let Width { bits, signed } = width;
        let ty = width.num().clif();
        if matches!(op, "shl" | "shr") {
            let n = b.ok_or_else(|| internal("a shift with no amount"))?;
            let overflow = self.b.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, n, i64::from(bits));
            let amount = self.b.ins().band_imm(n, i64::from(bits - 1));
            let wrapped = match (op, signed) {
                ("shl", _) => self.b.ins().ishl(a, amount),
                (_, true) => self.b.ins().sshr(a, amount),
                (_, false) => self.b.ins().ushr(a, amount),
            };
            return Ok(Done { wrapped, overflow, exact: None });
        }
        if bits == 64 {
            let flagged = match (op, signed, b) {
                ("add", true, Some(b)) => Some(self.b.ins().sadd_overflow(a, b)),
                ("add", false, Some(b)) => Some(self.b.ins().uadd_overflow(a, b)),
                ("sub", true, Some(b)) => Some(self.b.ins().ssub_overflow(a, b)),
                ("sub", false, Some(b)) => Some(self.b.ins().usub_overflow(a, b)),
                ("mul", true, Some(b)) => Some(self.b.ins().smul_overflow(a, b)),
                ("mul", false, Some(b)) => Some(self.b.ins().umul_overflow(a, b)),
                _ => None,
            };
            if let Some((wrapped, overflow)) = flagged {
                return Ok(Done { wrapped, overflow, exact: None });
            }
            if op != "neg" {
                return Err(CompileError::internal(format!("the intrinsic `num_{op}`")));
            }
            let wrapped = self.b.ins().ineg(a);
            let overflow = if signed { self.b.ins().icmp_imm(IntCC::Equal, a, i64::MIN) } else { self.b.ins().icmp_imm(IntCC::NotEqual, a, 0) };
            return Ok(Done { wrapped, overflow, exact: None });
        }
        let wide = |lowering: &mut Self, value: Value| if signed { lowering.b.ins().sextend(types::I64, value) } else { lowering.b.ins().uextend(types::I64, value) };
        let x = wide(self, a);
        let exact = match (op, b) {
            ("add", Some(b)) => {
                let y = wide(self, b);
                self.b.ins().iadd(x, y)
            }
            ("sub", Some(b)) => {
                let y = wide(self, b);
                self.b.ins().isub(x, y)
            }
            ("mul", Some(b)) => {
                let y = wide(self, b);
                self.b.ins().imul(x, y)
            }
            ("neg", _) => self.b.ins().ineg(x),
            _ => return Err(CompileError::internal(format!("the intrinsic `num_{op}`"))),
        };
        let wrapped = self.b.ins().ireduce(ty, exact);
        let back = wide(self, wrapped);
        let overflow = self.b.ins().icmp(IntCC::NotEqual, back, exact);
        Ok(Done { wrapped, overflow, exact: Some(exact) })
    }

    /// The value of an operation held to the range of the type.
    fn saturated(&mut self, op: &str, a: Value, b: Option<Value>, done: &Done, width: Width) -> Result<Value, CompileError> {
        let Width { bits, signed } = width;
        let ty = width.num().clif();
        let constant = |lowering: &mut Self, value: i128| lowering.b.ins().iconst(ty, sign_extended(value, bits));
        if matches!(op, "shl" | "shr") {
            return self.saturated_shift(op, a, b, done, width);
        }
        if let Some(exact) = done.exact {
            let clamped = match (signed, op) {
                (true, _) => {
                    let (min, max) = (self.b.ins().iconst(types::I64, width.min() as i64), self.b.ins().iconst(types::I64, width.max() as i64));
                    let low = self.b.ins().smin(exact, max);
                    self.b.ins().smax(low, min)
                }
                (false, "sub") => {
                    let zero = self.b.ins().iconst(types::I64, 0);
                    self.b.ins().smax(exact, zero)
                }
                (false, "neg") => self.b.ins().iconst(types::I64, 0),
                (false, _) => {
                    let max = self.b.ins().iconst(types::I64, width.max() as i64);
                    self.b.ins().umin(exact, max)
                }
            };
            return Ok(self.b.ins().ireduce(ty, clamped));
        }
        let (min, max) = (constant(self, width.min()), constant(self, width.max()));
        let limit = if signed {
            match (op, b) {
                ("neg", _) => max,
                ("mul", Some(b)) => {
                    let negative_a = self.b.ins().icmp_imm(IntCC::SignedLessThan, a, 0);
                    let negative_b = self.b.ins().icmp_imm(IntCC::SignedLessThan, b, 0);
                    let differ = self.b.ins().bxor(negative_a, negative_b);
                    self.b.ins().select(differ, min, max)
                }
                _ => {
                    let negative = self.b.ins().icmp_imm(IntCC::SignedLessThan, a, 0);
                    self.b.ins().select(negative, min, max)
                }
            }
        } else if matches!(op, "sub" | "neg") {
            min
        } else {
            max
        };
        Ok(self.b.ins().select(done.overflow, limit, done.wrapped))
    }

    fn saturated_shift(&mut self, op: &str, a: Value, b: Option<Value>, done: &Done, width: Width) -> Result<Value, CompileError> {
        let Width { bits, signed } = width;
        let ty = width.num().clif();
        let n = b.ok_or_else(|| internal("a shift with no amount"))?;
        let amount = self.b.ins().band_imm(n, i64::from(bits - 1));
        let out = done.overflow;
        let negative = if signed { Some(self.b.ins().icmp_imm(IntCC::SignedLessThan, a, 0)) } else { None };
        if op == "shr" {
            let fill = match negative {
                Some(negative) => {
                    let minus_one = self.b.ins().iconst(ty, -1);
                    let zero = self.b.ins().iconst(ty, 0);
                    self.b.ins().select(negative, minus_one, zero)
                }
                None => self.b.ins().iconst(ty, 0),
            };
            return Ok(self.b.ins().select(out, fill, done.wrapped));
        }
        let back = if signed { self.b.ins().sshr(done.wrapped, amount) } else { self.b.ins().ushr(done.wrapped, amount) };
        let lost = self.b.ins().icmp(IntCC::NotEqual, back, a);
        let (min, max) = (self.b.ins().iconst(ty, sign_extended(width.min(), bits)), self.b.ins().iconst(ty, sign_extended(width.max(), bits)));
        let toward = match negative {
            Some(negative) => self.b.ins().select(negative, min, max),
            None => max,
        };
        let inside = self.b.ins().select(lost, toward, done.wrapped);
        let zero = self.b.ins().iconst(ty, 0);
        let is_zero = self.b.ins().icmp_imm(IntCC::Equal, a, 0);
        let outside = self.b.ins().select(is_zero, zero, toward);
        Ok(self.b.ins().select(out, outside, inside))
    }
}
