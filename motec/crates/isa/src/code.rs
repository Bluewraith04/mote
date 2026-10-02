//! A compiled function: instructions plus the tables they index.

use crate::encoding::Instruction;
use crate::value::Value;

/// The source text behind instructions `[start_pc, end_pc)`: `len` bytes at `offset` of source `source`, starting at `line`:`col`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceSpan {
    pub start_pc: u32,
    pub end_pc: u32,
    pub source: u32,
    pub offset: u32,
    pub len: u32,
    pub line: u32,
    pub col: u32,
}

#[derive(Clone, Debug)]
/// A compiled function: instructions, register count and source spans.
pub struct CodeObject {
    pub instructions: Vec<Instruction>,
    pub constants: Vec<Value>,
    pub register_count: u16,
    pub param_count: u8,
    /// String literals for the `NEWSTR` opcode: `NEWSTR rA, bx` allocates a heap
    /// string from `string_table[bx]`.
    pub string_table: Vec<String>,
    /// Source spans of the AST nodes that produced the instructions; empty in a release build.
    pub spans: Vec<SourceSpan>,
}

impl CodeObject {
    pub fn new(instructions: Vec<Instruction>, constants: Vec<Value>, register_count: u16, param_count: u8) -> Self {
        Self {
            instructions,
            constants,
            register_count,
            param_count,
            string_table: Vec::new(),
            spans: Vec::new(),
        }
    }

    /// Builder: attach a `NEWSTR` string table.
    pub fn with_string_table(mut self, string_table: Vec<String>) -> Self {
        self.string_table = string_table;
        self
    }

    /// Builder: attach the source span table.
    pub fn with_spans(mut self, spans: Vec<SourceSpan>) -> Self {
        self.spans = spans;
        self
    }

    /// The innermost span covering instruction `pc`.
    pub fn span_at(&self, pc: usize) -> Option<&SourceSpan> {
        let pc = pc as u32;
        self.spans
            .iter()
            .filter(|s| s.start_pc <= pc && pc < s.end_pc)
            .min_by_key(|s| s.end_pc - s.start_pc)
    }
}

/// A source text a span table points into; `id` is the span's `source`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceFile {
    pub id: u32,
    pub path: String,
    pub text: String,
}
