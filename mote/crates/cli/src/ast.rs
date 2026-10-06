use isa::opcode::Opcode;
use isa::value::Value;

#[derive(Debug, Clone, PartialEq)]
/// An instruction operand.
pub enum Operand {
    Register(u8),
    Integer(i64),
    Float(f64),
    Label(String),
    Symbol(String),
}

#[derive(Debug, Clone, PartialEq)]
/// One instruction line of a `.masm` function.
pub struct ParsedInstruction {
    pub opcode: Opcode,
    pub operands: Vec<Operand>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
/// A line of a `.masm` function body.
pub enum FunctionItem {
    Label(String),
    Instruction(ParsedInstruction),
    Constant(Value),
}

#[derive(Debug, Clone, PartialEq)]
/// A `.masm` function.
pub struct ParsedFunction {
    pub name: String,
    pub register_count: u16,
    pub param_count: u8,
    pub items: Vec<FunctionItem>,
}

#[derive(Debug, Clone, PartialEq)]
/// A `.masm` type declaration.
pub struct ParsedType {
    pub name: String,
    pub id: u64,
    pub fields: Vec<String>,
    pub is_value_type: bool,
    pub is_trivial: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
/// A parsed `.masm` file.
pub struct ParsedProgram {
    pub types: Vec<ParsedType>,
    pub functions: Vec<ParsedFunction>,
}
