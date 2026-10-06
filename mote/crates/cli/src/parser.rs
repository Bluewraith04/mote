use crate::ast::*;
use crate::token::{Token, TokenKind};
use isa::opcode::Opcode;
use isa::value::Value;

/// The `.masm` parser.
pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token {
            kind: TokenKind::Eof,
            line: 0,
            col: 0,
        })
    }

    fn advance(&mut self) -> Token {
        if self.pos < self.tokens.len() {
            let tok = self.tokens[self.pos].clone();
            self.pos += 1;
            tok
        } else {
            Token {
                kind: TokenKind::Eof,
                line: 0,
                col: 0,
            }
        }
    }

    fn skip_newlines(&mut self) {
        while self.peek().kind == TokenKind::Newline {
            self.advance();
        }
    }

    pub fn parse_program(&mut self) -> Result<ParsedProgram, String> {
        let mut program = ParsedProgram::default();
        let mut top_level_items: Vec<FunctionItem> = Vec::new();

        self.skip_newlines();
        while self.peek().kind != TokenKind::Eof {
            match &self.peek().kind {
                TokenKind::Directive(d) if d == "type" => {
                    let type_decl = self.parse_type_decl()?;
                    program.types.push(type_decl);
                }
                TokenKind::Directive(d) if d == "func" => {
                    let func = self.parse_func_decl()?;
                    program.functions.push(func);
                }
                _ => {
                    let item = self.parse_function_item()?;
                    top_level_items.push(item);
                }
            }
            self.skip_newlines();
        }

        if !top_level_items.is_empty() {
            let main_func = ParsedFunction {
                name: "main".to_string(),
                register_count: 256,
                param_count: 0,
                items: top_level_items,
            };
            program.functions.insert(0, main_func);
        }

        Ok(program)
    }

    fn parse_type_decl(&mut self) -> Result<ParsedType, String> {
        self.advance();

        let name = match self.advance().kind {
            TokenKind::Symbol(s) | TokenKind::Identifier(s) => s,
            other => return Err(format!("Expected type name after .type, found {:?}", other)),
        };

        let mut id = 0u64;
        let mut fields = Vec::new();
        let mut is_value_type = false;
        let mut is_trivial = false;

        while self.peek().kind != TokenKind::Newline && self.peek().kind != TokenKind::Eof {
            match self.peek().kind.clone() {
                TokenKind::Identifier(ref key) if key == "valtype" => {
                    self.advance();
                    is_value_type = true;
                }
                TokenKind::Identifier(ref key) if key == "trivial" => {
                    self.advance();
                    is_trivial = true;
                }
                TokenKind::Identifier(ref key) if key == "id" => {
                    self.advance();
                    if self.peek().kind == TokenKind::Equals {
                        self.advance();
                    }
                    match self.advance().kind {
                        TokenKind::Integer(n) => id = n as u64,
                        other => return Err(format!("Expected integer type id, found {:?}", other)),
                    }
                }
                TokenKind::Identifier(ref key) if key == "fields" => {
                    self.advance();
                    if self.peek().kind == TokenKind::Equals {
                        self.advance();
                    }
                    loop {
                        match self.advance().kind {
                            TokenKind::Identifier(f) | TokenKind::StringLit(f) => fields.push(f),
                            other => return Err(format!("Expected field name, found {:?}", other)),
                        }
                        if self.peek().kind == TokenKind::Comma {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                }
                TokenKind::Integer(n) => {
                    self.advance();
                    id = n as u64;
                }
                TokenKind::Identifier(f) | TokenKind::StringLit(f) => {
                    self.advance();
                    fields.push(f);
                }
                TokenKind::Comma => {
                    self.advance();
                }
                other => return Err(format!("Unexpected token in .type declaration: {:?}", other)),
            }
        }

        Ok(ParsedType {
            name,
            id,
            fields,
            is_value_type,
            is_trivial,
        })
    }

    fn parse_func_decl(&mut self) -> Result<ParsedFunction, String> {
        self.advance();

        let name = match self.advance().kind {
            TokenKind::Symbol(s) | TokenKind::Identifier(s) => s,
            other => return Err(format!("Expected function name after .func, found {:?}", other)),
        };

        let mut regs: u16 = 256;
        let mut params: u8 = 0;

        while self.peek().kind != TokenKind::Newline && self.peek().kind != TokenKind::Eof {
            match self.peek().kind.clone() {
                TokenKind::Identifier(ref key) if key == "regs" => {
                    self.advance();
                    if self.peek().kind == TokenKind::Equals {
                        self.advance();
                    }
                    match self.advance().kind {
                        TokenKind::Integer(n) => regs = n as u16,
                        other => return Err(format!("Expected integer register count, found {:?}", other)),
                    }
                }
                TokenKind::Identifier(ref key) if key == "params" => {
                    self.advance();
                    if self.peek().kind == TokenKind::Equals {
                        self.advance();
                    }
                    match self.advance().kind {
                        TokenKind::Integer(n) => params = n as u8,
                        other => return Err(format!("Expected integer param count, found {:?}", other)),
                    }
                }
                TokenKind::Integer(n) => {
                    self.advance();
                    regs = n as u16;
                }
                TokenKind::Comma => {
                    self.advance();
                }
                other => return Err(format!("Unexpected token in .func header: {:?}", other)),
            }
        }

        self.skip_newlines();

        let mut items = Vec::new();
        while self.peek().kind != TokenKind::Eof {
            if let TokenKind::Directive(ref d) = self.peek().kind {
                if d == "end" {
                    self.advance();
                    break;
                }
            }
            let item = self.parse_function_item()?;
            items.push(item);
            self.skip_newlines();
        }

        Ok(ParsedFunction {
            name,
            register_count: regs,
            param_count: params,
            items,
        })
    }

    fn parse_function_item(&mut self) -> Result<FunctionItem, String> {
        self.skip_newlines();

        if let TokenKind::Directive(ref d) = self.peek().kind {
            if d == "const" {
                self.advance();
                match self.advance().kind {
                    TokenKind::Integer(n) => return Ok(FunctionItem::Constant(Value::small_int(n))),
                    TokenKind::Float(f) => return Ok(FunctionItem::Constant(Value::float(f))),
                    other => return Err(format!("Unsupported constant value: {:?}", other)),
                }
            }
        }

        if let TokenKind::Identifier(ref name) = self.peek().kind {
            let next_tok = self.tokens.get(self.pos + 1);
            if let Some(tok) = next_tok {
                if tok.kind == TokenKind::Colon {
                    let label_name = name.clone();
                    self.advance();
                    self.advance();
                    return Ok(FunctionItem::Label(label_name));
                }
            }
        }

        let tok = self.advance();
        let (opcode_name, line) = match tok.kind {
            TokenKind::Identifier(id) => (id, tok.line),
            other => return Err(format!("Expected instruction mnemonic or directive, found {:?} at line {}", other, tok.line)),
        };

        let opcode = self.parse_opcode(&opcode_name)?;
        let mut operands = Vec::new();

        while self.peek().kind != TokenKind::Newline && self.peek().kind != TokenKind::Eof {
            let op_tok = self.advance();
            let operand = match op_tok.kind {
                TokenKind::Identifier(ref s) if s.starts_with('r') || s.starts_with('R') => {
                    if let Ok(reg_idx) = s[1..].parse::<u8>() {
                        Operand::Register(reg_idx)
                    } else {
                        Operand::Label(s.clone())
                    }
                }
                TokenKind::Identifier(s) => Operand::Label(s),
                TokenKind::Symbol(s) => Operand::Symbol(s),
                TokenKind::Integer(n) => Operand::Integer(n),
                TokenKind::Float(f) => Operand::Float(f),
                TokenKind::Comma => continue,
                other => return Err(format!("Unexpected operand token {:?} at line {}", other, op_tok.line)),
            };
            operands.push(operand);
        }

        Ok(FunctionItem::Instruction(ParsedInstruction {
            opcode,
            operands,
            line,
        }))
    }

    fn parse_opcode(&self, s: &str) -> Result<Opcode, String> {
        let upper = s.to_uppercase();
        match upper.as_str() {
            "ADD" => Ok(Opcode::ADD),
            "SUB" => Ok(Opcode::SUB),
            "MUL" => Ok(Opcode::MUL),
            "DIV" => Ok(Opcode::DIV),
            "MOD" => Ok(Opcode::MOD),
            "NEG" => Ok(Opcode::NEG),
            "AND" => Ok(Opcode::AND),
            "OR" => Ok(Opcode::OR),
            "NOT" => Ok(Opcode::NOT),
            "XOR" => Ok(Opcode::XOR),
            "BAND" => Ok(Opcode::BAND),
            "BOR" => Ok(Opcode::BOR),
            "BXOR" => Ok(Opcode::BXOR),
            "BNOT" => Ok(Opcode::BNOT),
            "SHL" => Ok(Opcode::SHL),
            "SHR" => Ok(Opcode::SHR),
            "EQ" => Ok(Opcode::EQ),
            "NE" => Ok(Opcode::NE),
            "LT" => Ok(Opcode::LT),
            "LE" => Ok(Opcode::LE),
            "GT" => Ok(Opcode::GT),
            "GE" => Ok(Opcode::GE),
            "NEWOBJ" => Ok(Opcode::NEWOBJ),
            "SETFIELD" => Ok(Opcode::SETFIELD),
            "GETFIELD" => Ok(Opcode::GETFIELD),
            "TYPEOF" => Ok(Opcode::TYPEOF),
            "MOVE" => Ok(Opcode::MOVE),
            "COPYVAL" => Ok(Opcode::COPYVAL),
            "MKGEN" => Ok(Opcode::MKGEN),
            "RESUME" => Ok(Opcode::RESUME),
            "YIELD" => Ok(Opcode::YIELD),
            "CALLNATIVEW" => Ok(Opcode::CALLNATIVEW),
            "CALLNATIVEF" => Ok(Opcode::CALLNATIVEF),
            "LOADI" => Ok(Opcode::LOADI),
            "LOADK" => Ok(Opcode::LOADK),
            "JMP" => Ok(Opcode::JMP),
            "JMPIF" => Ok(Opcode::JMPIF),
            "JMPIFNOT" => Ok(Opcode::JMPIFNOT),
            "CALL" => Ok(Opcode::CALL),
            "RET" => Ok(Opcode::RET),
            "RETN" => Ok(Opcode::RETN),
            "NOP" => Ok(Opcode::NOP),
            "HALT" => Ok(Opcode::HALT),
            "DEBUGPRINT" => Ok(Opcode::DEBUGPRINT),
            "ENTERARENA" => Ok(Opcode::ENTERARENA),
            "EXITARENA" => Ok(Opcode::EXITARENA),
            "ARENAALLOC" => Ok(Opcode::ARENAALLOC),
            "CALLNATIVE" => Ok(Opcode::CALLNATIVE),
            "CALLINTRINSIC" => Ok(Opcode::CALLINTRINSIC),
            "STAMP" => Ok(Opcode::STAMP),
            "LOADTYPE" => Ok(Opcode::LOADTYPE),
            "STAMPT" => Ok(Opcode::STAMPT),
            "ASTYPE" => Ok(Opcode::ASTYPE),
            "ISTYPE" => Ok(Opcode::ISTYPE),
            "SOME" => Ok(Opcode::SOME),
            "UNSOME" => Ok(Opcode::UNSOME),
            _ => Err(format!("Unknown opcode mnemonic: '{}'", s)),
        }
    }
}
