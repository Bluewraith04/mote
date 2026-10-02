#[derive(Debug, Clone, PartialEq)]
/// The kinds of `.masm` token.
pub enum TokenKind {
    Directive(String),
    Identifier(String),
    Symbol(String),
    Integer(i64),
    Float(f64),
    StringLit(String),
    Colon,
    Comma,
    Equals,
    Newline,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
/// A `.masm` token with its position.
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub col: usize,
}

/// The `.masm` tokenizer.
pub struct Tokenizer {
    chars: Vec<(usize, char)>,
    pos: usize,
    line: usize,
    col: usize,
}

impl Tokenizer {
    pub fn new(input: &str) -> Self {
        let chars: Vec<(usize, char)> = input.char_indices().collect();
        Self {
            chars,
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).map(|&(_, c)| c)
    }

    fn peek_next(&self) -> Option<char> {
        self.chars.get(self.pos + 1).map(|&(_, c)| c)
    }

    fn advance(&mut self) -> Option<char> {
        if let Some(&(_, c)) = self.chars.get(self.pos) {
            self.pos += 1;
            if c == '\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += 1;
            }
            Some(c)
        } else {
            None
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();

        while let Some(c) = self.peek() {
            let start_line = self.line;
            let start_col = self.col;

            match c {
                ' ' | '\t' | '\r' => {
                    self.advance();
                }
                '\n' => {
                    self.advance();
                    if let Some(last) = tokens.last() {
                        if (last as &Token).kind != TokenKind::Newline {
                            tokens.push(Token {
                                kind: TokenKind::Newline,
                                line: start_line,
                                col: start_col,
                            });
                        }
                    }
                }
                ';' | '#' => {
                    while let Some(nc) = self.peek() {
                        if nc == '\n' {
                            break;
                        }
                        self.advance();
                    }
                }
                '/' if self.peek_next() == Some('/') => {
                    while let Some(nc) = self.peek() {
                        if nc == '\n' {
                            break;
                        }
                        self.advance();
                    }
                }
                ':' => {
                    self.advance();
                    tokens.push(Token { kind: TokenKind::Colon, line: start_line, col: start_col });
                }
                ',' => {
                    self.advance();
                    tokens.push(Token { kind: TokenKind::Comma, line: start_line, col: start_col });
                }
                '=' => {
                    self.advance();
                    tokens.push(Token { kind: TokenKind::Equals, line: start_line, col: start_col });
                }
                '.' => {
                    self.advance();
                    let mut name = String::new();
                    while let Some(nc) = self.peek() {
                        if nc.is_alphanumeric() || nc == '_' {
                            name.push(nc);
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    tokens.push(Token {
                        kind: TokenKind::Directive(name),
                        line: start_line,
                        col: start_col,
                    });
                }
                '@' => {
                    self.advance();
                    let mut name = String::new();
                    while let Some(nc) = self.peek() {
                        if nc.is_alphanumeric() || nc == '_' {
                            name.push(nc);
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    tokens.push(Token {
                        kind: TokenKind::Symbol(name),
                        line: start_line,
                        col: start_col,
                    });
                }
                '"' => {
                    self.advance();
                    let mut s = String::new();
                    let mut closed = false;
                    while let Some(nc) = self.advance() {
                        if nc == '"' {
                            closed = true;
                            break;
                        }
                        s.push(nc);
                    }
                    if !closed {
                        return Err(format!("Unterminated string literal at line {}:{}", start_line, start_col));
                    }
                    tokens.push(Token {
                        kind: TokenKind::StringLit(s),
                        line: start_line,
                        col: start_col,
                    });
                }
                '-' | '+' | '0'..='9' => {
                    let mut num_str = String::new();
                    let is_sign = c == '-' || c == '+';
                    if is_sign {
                        num_str.push(c);
                        self.advance();
                    }

                    if is_sign && !self.peek().map(|ch| ch.is_ascii_digit()).unwrap_or(false) {
                        return Err(format!("Unexpected sign with no digits at line {}:{}", start_line, start_col));
                    }

                    let mut is_float = false;
                    let mut is_hex = false;

                    if (num_str == "0" || (num_str.is_empty() && self.peek() == Some('0')))
                        && (self.peek_next() == Some('x') || self.peek_next() == Some('X')) {
                            is_hex = true;
                            num_str.push(self.advance().unwrap());
                            num_str.push(self.advance().unwrap());
                        }

                    while let Some(nc) = self.peek() {
                        if (is_hex && nc.is_ascii_hexdigit()) || (!is_hex && nc.is_ascii_digit()) {
                            num_str.push(nc);
                            self.advance();
                        } else if !is_hex && nc == '.' && self.peek_next().map(|d| d.is_ascii_digit()).unwrap_or(false) {
                            is_float = true;
                            num_str.push(nc);
                            self.advance();
                        } else {
                            break;
                        }
                    }

                    if is_float {
                        let val: f64 = num_str.parse().map_err(|e| format!("Invalid float '{}' at {}:{}: {}", num_str, start_line, start_col, e))?;
                        tokens.push(Token { kind: TokenKind::Float(val), line: start_line, col: start_col });
                    } else if is_hex {
                        let clean = num_str.trim_start_matches("0x").trim_start_matches("0X");
                        let val = i64::from_str_radix(clean, 16).map_err(|e| format!("Invalid hex integer '{}' at {}:{}: {}", num_str, start_line, start_col, e))?;
                        tokens.push(Token { kind: TokenKind::Integer(val), line: start_line, col: start_col });
                    } else {
                        let val: i64 = num_str.parse().map_err(|e| format!("Invalid integer '{}' at {}:{}: {}", num_str, start_line, start_col, e))?;
                        tokens.push(Token { kind: TokenKind::Integer(val), line: start_line, col: start_col });
                    }
                }
                c if c.is_alphabetic() || c == '_' => {
                    let mut ident = String::new();
                    while let Some(nc) = self.peek() {
                        if nc.is_alphanumeric() || nc == '_' {
                            ident.push(nc);
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    tokens.push(Token {
                        kind: TokenKind::Identifier(ident),
                        line: start_line,
                        col: start_col,
                    });
                }
                other => {
                    return Err(format!("Unexpected character '{}' at line {}:{}", other, start_line, start_col));
                }
            }
        }

        tokens.push(Token {
            kind: TokenKind::Eof,
            line: self.line,
            col: self.col,
        });

        Ok(tokens)
    }
}
