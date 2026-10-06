use crate::codegen::{CodeGenerator, CompiledProgram};
use crate::diagnostics::DiagnosticFormatter;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::type_check::TypeChecker;

/// Compiles and runs a single source file.
pub struct Compiler;

impl Compiler {
    /// Lexes, parses and type-checks one source text into the checker's stage output.
    pub fn check(source: &str, file_name: &str) -> Result<crate::stage::Checked, String> {
        let mut lexer = Lexer::new(source);
        crate::span::name_source(lexer.source_id(), file_name);
        let tokens = lexer.tokenize().map_err(|(err, span)| {
            DiagnosticFormatter::format_error(source, file_name, &err, &span)
        })?;

        let mut parser = Parser::new(tokens);
        let mut program = parser.parse().map_err(|(err, span)| {
            DiagnosticFormatter::format_error(source, file_name, &err, &span)
        })?;

        let mut items = crate::prelude::synthesized_items();
        items.append(&mut program.items);
        program.items = items;
        crate::impls::copy_trait_defaults(&mut [&mut program]);

        let mut checker = TypeChecker::new();
        if let Err(errors) = checker.check_program(&program) {
            let mut full_err = String::new();
            for (err, span) in errors {
                full_err.push_str(&DiagnosticFormatter::format_error(source, file_name, &err, &span));
            }
            return Err(full_err);
        }
        Ok(checker.finish(vec![program]))
    }

    pub fn compile(source: &str, file_name: &str) -> Result<CompiledProgram, String> {
        let checked = Self::check(source, file_name)?;
        CodeGenerator::new().compile_single(checked).map_err(|(err, span)| {
            DiagnosticFormatter::format_error(source, file_name, &err, &span)
        })
    }
}
