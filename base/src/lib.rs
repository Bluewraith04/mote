//! Holds the spans, source map, interner and diagnostics every other crate shares.

mod arena;
mod diagnostic;
mod render;
mod source;
mod span;
mod symbol;

pub use arena::{Arena, Id};
pub use diagnostic::{Diagnostic, DiagnosticSink, Label, Severity};
pub use render::render;
pub use source::{Location, SourceFile, SourceMap};
pub use span::Span;
pub use symbol::{Interner, Symbol};
