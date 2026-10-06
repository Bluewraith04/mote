//! The Mote compiler front end: lexer, parser, type checker and bytecode generator.
/// Source locations.
pub mod span;
/// Token kinds and string-literal parts.
pub mod token;
/// The lexer.
pub mod lexer;
/// The syntax tree.
pub mod ast;
mod ast_walk;
/// The parser.
pub mod parser;
pub mod derive;
pub mod stable;
pub mod impls;
pub mod builtin_natives;
pub mod foreign;
pub mod prelude;
/// The type representation shared by the checker and code generator.
pub mod types;
pub mod stage;
/// The type checker.
pub mod type_check;
/// Register allocation for one call frame.
pub mod reg_alloc;
pub mod regions;
/// Bytecode generation.
pub mod codegen;
/// Rendering of errors with source context.
pub mod diagnostics;
/// The compile pipeline: lex, parse, check and generate.
pub mod driver;

pub use span::Span;
pub use token::{StrPart, Token, TokenKind};
pub use lexer::Lexer;
pub use ast::Program;
pub use parser::Parser;
pub use types::Type;
pub use stage::TypeTable;
pub use type_check::TypeChecker;
pub use codegen::{CodeGenerator, CompiledProgram, SourceFile};
pub use diagnostics::DiagnosticFormatter;
pub use driver::Compiler;
