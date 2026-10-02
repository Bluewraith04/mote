use std::collections::HashMap;
use crate::ast::*;
use runtime::CodeObject;
use isa::encoding::*;
use isa::opcode::Opcode;
use isa::value::TypeDescriptor;

#[derive(Debug, Clone)]
/// A program assembled from `.masm` text.
pub struct AssembledProgram {
    pub code_objects: Vec<CodeObject>,
    pub type_descriptors: Vec<TypeDescriptor>,
}

/// The `.masm` assembler.
pub struct Assembler;

impl Assembler {
    pub fn assemble(program: ParsedProgram) -> Result<AssembledProgram, String> {
        let mut type_descriptors = Vec::new();
        let mut type_symbol_map = HashMap::new();

        for (idx, parsed_type) in program.types.iter().enumerate() {
            type_symbol_map.insert(parsed_type.name.clone(), idx as u16);
            let field_names: Vec<Option<String>> = parsed_type
                .fields
                .iter()
                .map(|f| Some(f.clone()))
                .collect();
            let desc = if parsed_type.is_value_type {
                TypeDescriptor::new_value_type(parsed_type.id, field_names, parsed_type.is_trivial)
            } else {
                TypeDescriptor::new(parsed_type.id, field_names)
            };
            type_descriptors.push(desc);
        }

        let mut func_symbol_map = HashMap::new();
        for (idx, func) in program.functions.iter().enumerate() {
            func_symbol_map.insert(func.name.clone(), idx as u16);
        }

        let mut code_objects = Vec::new();

        for func in &program.functions {
            let mut constants = Vec::new();
            let mut label_map: HashMap<String, usize> = HashMap::new();
            let mut instructions_raw = Vec::new();

            let mut inst_idx = 0;
            for item in &func.items {
                match item {
                    FunctionItem::Label(label_name) => {
                        label_map.insert(label_name.clone(), inst_idx);
                    }
                    FunctionItem::Constant(val) => {
                        constants.push(*val);
                    }
                    FunctionItem::Instruction(inst) => {
                        instructions_raw.push(inst);
                        inst_idx += 1;
                    }
                }
            }

            let mut encoded_instructions = Vec::new();

            for (pc, inst) in instructions_raw.iter().enumerate() {
                let encoded = Self::encode_instruction(
                    pc,
                    inst,
                    &label_map,
                    &func_symbol_map,
                    &type_symbol_map,
                    &mut constants,
                )?;
                encoded_instructions.push(encoded);
            }

            code_objects.push(CodeObject::new(
                encoded_instructions,
                constants,
                func.register_count,
                func.param_count,
            ));
        }

        Ok(AssembledProgram {
            code_objects,
            type_descriptors,
        })
    }

    fn encode_instruction(
        pc: usize,
        inst: &ParsedInstruction,
        labels: &HashMap<String, usize>,
        funcs: &HashMap<String, u16>,
        types: &HashMap<String, u16>,
        constants: &mut Vec<isa::value::Value>,
    ) -> Result<Instruction, String> {
        let op = inst.opcode;
        match op {
            Opcode::ADD
            | Opcode::SUB
            | Opcode::MUL
            | Opcode::DIV
            | Opcode::MOD
            | Opcode::AND
            | Opcode::OR
            | Opcode::XOR
            | Opcode::BAND
            | Opcode::BOR
            | Opcode::BXOR
            | Opcode::SHL
            | Opcode::SHR
            | Opcode::EQ
            | Opcode::NE
            | Opcode::LT
            | Opcode::LE
            | Opcode::GT
            | Opcode::GE => {
                if inst.operands.len() != 3 {
                    return Err(format!(
                        "Opcode {:?} at line {} expects 3 operands (dest, src1, src2)",
                        op, inst.line
                    ));
                }
                let a = Self::expect_reg(&inst.operands[0], inst.line)?;
                let b = Self::expect_reg(&inst.operands[1], inst.line)?;
                let c = Self::expect_reg(&inst.operands[2], inst.line)?;
                Ok(encode_r3(op, a, b, c))
            }

            Opcode::NEG | Opcode::NOT | Opcode::BNOT | Opcode::MOVE | Opcode::TYPEOF | Opcode::COPYVAL | Opcode::SOME | Opcode::UNSOME => {
                if inst.operands.len() != 2 {
                    return Err(format!(
                        "Opcode {:?} at line {} expects 2 operands (dest, src)",
                        op, inst.line
                    ));
                }
                let a = Self::expect_reg(&inst.operands[0], inst.line)?;
                let b = Self::expect_reg(&inst.operands[1], inst.line)?;
                Ok(encode_r2(op, a, b))
            }

            Opcode::DEBUGPRINT => {
                if inst.operands.len() != 1 {
                    return Err(format!(
                        "DEBUGPRINT at line {} expects 1 operand (src)",
                        inst.line
                    ));
                }
                let a = Self::expect_reg(&inst.operands[0], inst.line)?;
                Ok(encode_r2(op, 0, a))
            }

            Opcode::LOADI => {
                if inst.operands.len() != 2 {
                    return Err(format!(
                        "LOADI at line {} expects 2 operands (dest, immediate)",
                        inst.line
                    ));
                }
                let a = Self::expect_reg(&inst.operands[0], inst.line)?;
                match &inst.operands[1] {
                    Operand::Integer(n) => {
                        if (0..=u16::MAX as i64).contains(n) {
                            let bx = *n as u16;
                            Ok(encode_ri(Opcode::LOADI, a, bx))
                        } else {
                            let const_val = isa::value::Value::small_int(*n);
                            let const_idx = if let Some(existing_idx) = constants.iter().position(|&c| c == const_val) {
                                existing_idx
                            } else {
                                let idx = constants.len();
                                constants.push(const_val);
                                idx
                            };
                            Ok(encode_ri(Opcode::LOADK, a, const_idx as u16))
                        }
                    }
                    other => Err(format!("LOADI expects integer immediate, found {:?} at line {}", other, inst.line)),
                }
            }

            Opcode::LOADK => {
                if inst.operands.len() != 2 {
                    return Err(format!(
                        "LOADK at line {} expects 2 operands (dest, const_index)",
                        inst.line
                    ));
                }
                let a = Self::expect_reg(&inst.operands[0], inst.line)?;
                let bx = match &inst.operands[1] {
                    Operand::Integer(n) => *n as u16,
                    other => return Err(format!("LOADK expects integer constant index, found {:?} at line {}", other, inst.line)),
                };
                Ok(encode_ri(op, a, bx))
            }

            Opcode::NEWOBJ | Opcode::STAMP => {
                if inst.operands.len() != 2 {
                    return Err(format!(
                        "{:?} at line {} expects 2 operands (dest, type)",
                        op, inst.line
                    ));
                }
                let a = Self::expect_reg(&inst.operands[0], inst.line)?;
                let type_idx = match &inst.operands[1] {
                    Operand::Symbol(s) | Operand::Label(s) => {
                        *types.get(s).ok_or_else(|| format!("Unknown type @{} at line {}", s, inst.line))?
                    }
                    Operand::Integer(n) => *n as u16,
                    other => return Err(format!("{:?} expects type symbol or index, found {:?} at line {}", op, other, inst.line)),
                };
                Ok(encode_ri(op, a, type_idx))
            }

            Opcode::GETFIELD => {
                if inst.operands.len() != 3 {
                    return Err(format!(
                        "GETFIELD at line {} expects 3 operands (dest, obj, field)",
                        inst.line
                    ));
                }
                let dest = Self::expect_reg(&inst.operands[0], inst.line)?;
                let obj = Self::expect_reg(&inst.operands[1], inst.line)?;
                let field = match &inst.operands[2] {
                    Operand::Integer(n) => *n as u8,
                    Operand::Register(r) => *r,
                    other => return Err(format!("GETFIELD expects field index, found {:?} at line {}", other, inst.line)),
                };
                Ok(encode_getfield(dest, obj, field))
            }

            Opcode::SETFIELD => {
                if inst.operands.len() != 3 {
                    return Err(format!(
                        "SETFIELD at line {} expects 3 operands (obj, field, val)",
                        inst.line
                    ));
                }
                let obj = Self::expect_reg(&inst.operands[0], inst.line)?;
                let field = match &inst.operands[1] {
                    Operand::Integer(n) => *n as u8,
                    Operand::Register(r) => *r,
                    other => return Err(format!("SETFIELD expects field index, found {:?} at line {}", other, inst.line)),
                };
                let val = Self::expect_reg(&inst.operands[2], inst.line)?;
                Ok(encode_setfield(obj, field, val))
            }

            Opcode::JMP => {
                if inst.operands.len() != 1 {
                    return Err(format!("JMP at line {} expects 1 operand (target)", inst.line));
                }
                let offset = Self::resolve_jump_target(pc, &inst.operands[0], labels, inst.line)?;
                Ok(encode_ju(op, offset))
            }

            Opcode::JMPIF | Opcode::JMPIFNOT => {
                if inst.operands.len() != 2 {
                    return Err(format!(
                        "{:?} at line {} expects 2 operands (cond, target)",
                        op, inst.line
                    ));
                }
                let cond = Self::expect_reg(&inst.operands[0], inst.line)?;
                let offset = Self::resolve_jump_target(pc, &inst.operands[1], labels, inst.line)? as i16;
                Ok(encode_jc(op, cond, offset))
            }

            Opcode::CALL => {
                if inst.operands.len() != 2 {
                    return Err(format!(
                        "CALL at line {} expects 2 operands (dest, func); args start at dest+1",
                        inst.line
                    ));
                }
                let dest = Self::expect_reg(&inst.operands[0], inst.line)?;
                let func_idx = match &inst.operands[1] {
                    Operand::Symbol(s) | Operand::Label(s) => {
                        *funcs.get(s).ok_or_else(|| format!("Unknown function @{} at line {}", s, inst.line))?
                    }
                    Operand::Integer(n) => *n as u16,
                    other => return Err(format!("CALL expects function symbol or index, found {:?} at line {}", other, inst.line)),
                };
                Ok(encode_call(dest, func_idx))
            }

            Opcode::GETCAPTURE => {
                if inst.operands.len() != 2 {
                    return Err(format!(
                        "GETCAPTURE at line {} expects 2 operands (dest, capture_index)",
                        inst.line
                    ));
                }
                let dest = Self::expect_reg(&inst.operands[0], inst.line)?;
                let idx = match &inst.operands[1] {
                    Operand::Integer(n) => *n as u16,
                    other => return Err(format!("GETCAPTURE expects an integer capture index, found {:?} at line {}", other, inst.line)),
                };
                Ok(encode_getcapture(dest, idx))
            }

            Opcode::GETGLOBAL | Opcode::SETGLOBAL => {
                if inst.operands.len() != 2 {
                    return Err(format!(
                        "{:?} at line {} expects 2 operands (reg, global_index)",
                        inst.opcode, inst.line
                    ));
                }
                let reg = Self::expect_reg(&inst.operands[0], inst.line)?;
                let idx = match &inst.operands[1] {
                    Operand::Integer(n) => *n as u16,
                    other => return Err(format!("{:?} expects an integer global index, found {:?} at line {}", inst.opcode, other, inst.line)),
                };
                Ok(if inst.opcode == Opcode::GETGLOBAL {
                    encode_getglobal(reg, idx)
                } else {
                    encode_setglobal(reg, idx)
                })
            }

            Opcode::CALLV => {
                if inst.operands.len() != 3 {
                    return Err(format!(
                        "CALLV at line {} expects 3 operands (dest, callee_reg, arg_base)",
                        inst.line
                    ));
                }
                let dest = Self::expect_reg(&inst.operands[0], inst.line)?;
                let callee_reg = Self::expect_reg(&inst.operands[1], inst.line)?;
                let arg_base = Self::expect_reg(&inst.operands[2], inst.line)?;
                Ok(encode_callv(dest, callee_reg, arg_base))
            }

            Opcode::RET => {
                let ret_reg = if inst.operands.is_empty() {
                    0
                } else {
                    Self::expect_reg(&inst.operands[0], inst.line)?
                };
                Ok(encode_ret(ret_reg))
            }

            Opcode::RETN => match inst.operands.as_slice() {
                [first, Operand::Integer(n)] if (1..=255).contains(n) => Ok(encode_r2(Opcode::RETN, Self::expect_reg(first, inst.line)?, *n as u8)),
                _ => Err(format!("RETN at line {} expects a register and a count", inst.line)),
            },

            Opcode::MKGEN => Ok(encode_none(Opcode::MKGEN)),
            Opcode::RESUME => {
                if inst.operands.len() != 3 {
                    return Err(format!(
                        "RESUME at line {} expects 3 operands (dest, generator, status)",
                        inst.line
                    ));
                }
                let a = Self::expect_reg(&inst.operands[0], inst.line)?;
                let b = Self::expect_reg(&inst.operands[1], inst.line)?;
                let c = Self::expect_reg(&inst.operands[2], inst.line)?;
                Ok(encode_r3(op, a, b, c))
            }
            Opcode::YIELD => {
                if inst.operands.len() != 1 {
                    return Err(format!("YIELD at line {} expects 1 operand (src)", inst.line));
                }
                let a = Self::expect_reg(&inst.operands[0], inst.line)?;
                Ok(encode_r2(op, a, 0))
            }

            Opcode::NOP => Ok(encode_none(Opcode::NOP)),
            Opcode::HALT => Ok(encode_none(Opcode::HALT)),

            Opcode::ENTERARENA => {
                let cap = if inst.operands.is_empty() {
                    0
                } else {
                    match &inst.operands[0] {
                        Operand::Integer(n) => *n as u16,
                        other => return Err(format!("ENTERARENA expects integer capacity, found {:?} at line {}", other, inst.line)),
                    }
                };
                Ok(encode_ri(Opcode::ENTERARENA, 0, cap))
            }
            Opcode::EXITARENA => Ok(encode_none(Opcode::EXITARENA)),
            Opcode::ARENAALLOC => {
                if inst.operands.len() != 2 {
                    return Err(format!(
                        "ARENAALLOC at line {} expects 2 operands (dest, type)",
                        inst.line
                    ));
                }
                let a = Self::expect_reg(&inst.operands[0], inst.line)?;
                let type_idx = match &inst.operands[1] {
                    Operand::Symbol(s) | Operand::Label(s) => {
                        *types.get(s).ok_or_else(|| format!("Unknown type @{} at line {}", s, inst.line))?
                    }
                    Operand::Integer(n) => *n as u16,
                    other => return Err(format!("ARENAALLOC expects type symbol or index, found {:?} at line {}", other, inst.line)),
                };
                Ok(encode_ri(Opcode::ARENAALLOC, a, type_idx))
            }

            Opcode::CALLNATIVEW | Opcode::CALLNATIVEF => {
                if inst.operands.len() != 2 {
                    return Err(format!("{:?} at line {} expects 2 operands (dest, table_idx)", inst.opcode, inst.line));
                }
                let dest = Self::expect_reg(&inst.operands[0], inst.line)?;
                let table_idx = match &inst.operands[1] {
                    Operand::Integer(n) => *n as u16,
                    other => return Err(format!("{:?} expects an integer table index, found {:?} at line {}", inst.opcode, other, inst.line)),
                };
                Ok(encode_ri(inst.opcode, dest, table_idx))
            }

            Opcode::CALLNATIVE | Opcode::CALLINTRINSIC => {
                if inst.operands.len() != 3 {
                    return Err(format!(
                        "CALLNATIVE at line {} expects 3 operands (dest, func_idx, arg_base)",
                        inst.line
                    ));
                }
                let dest = Self::expect_reg(&inst.operands[0], inst.line)?;
                let func_idx = match &inst.operands[1] {
                    Operand::Integer(n) => *n as u8,
                    Operand::Register(r) => *r,
                    other => return Err(format!("CALLNATIVE expects integer function index, found {:?} at line {}", other, inst.line)),
                };
                let arg_base = match &inst.operands[2] {
                    Operand::Register(r) => *r,
                    Operand::Integer(n) => *n as u8,
                    other => return Err(format!("CALLNATIVE expects register or integer for arg_base, found {:?} at line {}", other, inst.line)),
                };
                Ok(encode_r3(inst.opcode, dest, func_idx, arg_base))
            }

            Opcode::NEWSTR => Err(format!(
                "NEWSTR at line {} is not supported in textual assembly (no string-table syntax)",
                inst.line
            )),

            Opcode::SCOPEENTER | Opcode::SCOPEEXIT | Opcode::SPAWN | Opcode::LOADTYPE | Opcode::STAMPT | Opcode::ASTYPE | Opcode::ISTYPE => Err(format!(
                "{:?} at line {} is not supported in textual assembly (compiler-only opcode)",
                op, inst.line
            )),
        }
    }

    fn expect_reg(op: &Operand, line: usize) -> Result<u8, String> {
        match op {
            Operand::Register(r) => Ok(*r),
            other => Err(format!("Expected register (e.g. r0), found {:?} at line {}", other, line)),
        }
    }

    fn resolve_jump_target(
        pc: usize,
        op: &Operand,
        labels: &HashMap<String, usize>,
        line: usize,
    ) -> Result<i32, String> {
        match op {
            Operand::Label(s) | Operand::Symbol(s) => {
                let target_pc = labels
                    .get(s)
                    .ok_or_else(|| format!("Undefined label '{}' at line {}", s, line))?;
                let offset = *target_pc as i32 - pc as i32;
                Ok(offset)
            }
            Operand::Integer(n) => Ok(*n as i32),
            other => Err(format!("Expected label or integer jump offset, found {:?} at line {}", other, line)),
        }
    }
}
