//! Turns source text into an abstract syntax tree.

mod ast;
#[cfg(test)]
mod grammar_tests;
mod lexer;
mod parser;
mod token;

pub use ast::*;
pub use parser::parse;
pub use lexer::{lex, Comment, CommentKind, Lexed};
pub use token::{FloatLiteral, FloatSuffix, IntLiteral, IntSuffix, Radix, Token, TokenKind, KEYWORDS};
