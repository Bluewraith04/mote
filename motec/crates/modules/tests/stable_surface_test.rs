//! Removing or changing the signature of anything `@stable` in `std` fails this test.

use compiler::lexer::Lexer;
use compiler::parser::Parser;

fn current_surface() -> String {
    let mut lines: Vec<String> = Vec::new();
    for (module, src) in modules::embedded::all() {
        let tokens = Lexer::new(src).tokenize().unwrap_or_else(|e| panic!("std.{module} failed to lex: {e:?}"));
        let program = Parser::new(tokens).parse().unwrap_or_else(|e| panic!("std.{module} failed to parse: {e:?}"));
        for mark in &program.stable_marks {
            lines.push(format!("std.{module}::{} [{}] {}", mark.name, mark.kind, mark.signature));
        }
    }
    lines.sort();
    lines.join("\n")
}

const SNAPSHOT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/stable_surface.snapshot");

#[test]
fn stable_surface_matches_the_committed_snapshot() {
    let current = current_surface();
    let committed = std::fs::read_to_string(SNAPSHOT_PATH).unwrap_or_default();
    assert_eq!(
        current.trim(),
        committed.trim(),
        "\nthe @stable std surface changed since tests/stable_surface.snapshot was committed.\n\
         If this is an intentional, reviewed change, update the snapshot file to match `current_surface()`'s output.\n"
    );
}
