use logos::Logos;
use crate::span::{next_source_id, Span};
use crate::token::{Token, TokenKind};

/// Turns source text into tokens.
pub struct Lexer<'a> {
    source: &'a str,
    line_starts: Vec<usize>,
    source_id: u32,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        let source_id = next_source_id();
        if !crate::span::is_release() {
            crate::span::register_source_text(source_id, source);
        }
        let mut line_starts = vec![0];
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self {
            source,
            line_starts,
            source_id,
        }
    }

    pub fn source_id(&self) -> u32 {
        self.source_id
    }

    fn offset_to_line_col(&self, offset: usize) -> (usize, usize) {
        let line_idx = match self.line_starts.binary_search(&offset) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        };
        let line = line_idx + 1;
        let line_start = self.line_starts[line_idx];
        let col = offset - line_start + 1;
        (line, col)
    }

    fn span_from_range(&self, range: std::ops::Range<usize>) -> Span {
        let (start_line, start_col) = self.offset_to_line_col(range.start);
        Span::new(range.start, range.end, start_line, start_col).with_source(self.source_id)
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, (String, Span)> {
        let mut tokens = Vec::new();
        let mut raw_lexer = TokenKind::lexer(self.source);
        let mut delimiter_depth = 0usize;
        let mut last_token: Option<TokenKind> = None;

        while let Some(res) = raw_lexer.next() {
            let range = raw_lexer.span();
            let span = self.span_from_range(range);

            let raw_kind = match res {
                Ok(k) => k,
                Err(_) => {
                    return Err((format!("Invalid token at '{}'", raw_lexer.slice()), span));
                }
            };

            match raw_kind {
                TokenKind::BlockComment => {
                    continue;
                }
                TokenKind::Newline => {
                    if delimiter_depth == 0
                        && let Some(ref last) = last_token
                            && Token::new(last.clone(), span).can_end_statement() {
                                tokens.push(Token::new(TokenKind::Newline, span));
                                last_token = None;
                            }
                }
                TokenKind::OpenParen | TokenKind::OpenBracket | TokenKind::OpenBrace => {
                    delimiter_depth += 1;
                    last_token = Some(raw_kind.clone());
                    tokens.push(Token::new(raw_kind, span));
                }
                TokenKind::CloseParen | TokenKind::CloseBracket | TokenKind::CloseBrace => {
                    delimiter_depth = delimiter_depth.saturating_sub(1);
                    last_token = Some(raw_kind.clone());
                    tokens.push(Token::new(raw_kind, span));
                }
                TokenKind::Float(_) if matches!(last_token, Some(TokenKind::Dot)) => {
                    let slice = raw_lexer.slice();
                    let range = raw_lexer.span();
                    let split = slice.split_once('.').filter(|(a, b)| {
                        [a, b].iter().all(|s| s.bytes().all(|c| c.is_ascii_digit()))
                    });
                    let Some((a, b)) = split else {
                        last_token = Some(raw_kind.clone());
                        tokens.push(Token::new(raw_kind, span));
                        continue;
                    };
                    let (dot, end) = (range.start + a.len(), range.end);
                    let index = |s: &str| TokenKind::Int(s.parse().unwrap_or(i64::MAX));
                    tokens.push(Token::new(index(a), self.span_from_range(range.start..dot)));
                    tokens.push(Token::new(TokenKind::Dot, self.span_from_range(dot..dot + 1)));
                    tokens.push(Token::new(index(b), self.span_from_range(dot + 1..end)));
                    last_token = Some(index(b));
                }
                _ => {
                    last_token = Some(raw_kind.clone());
                    tokens.push(Token::new(raw_kind, span));
                }
            }
        }

        let eof_offset = self.source.len();
        let (eof_line, eof_col) = self.offset_to_line_col(eof_offset);
        let eof_span = Span::new(eof_offset, eof_offset, eof_line, eof_col).with_source(self.source_id);

        if let Some(ref last) = last_token
            && Token::new(last.clone(), eof_span).can_end_statement() {
                tokens.push(Token::new(TokenKind::Newline, eof_span));
            }

        tokens.push(Token::new(TokenKind::Eof, eof_span));
        Ok(tokens)
    }
}
