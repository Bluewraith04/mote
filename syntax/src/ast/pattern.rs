use mote_base::{Id, Span, Symbol};

use super::{Ident, Path, Type};
use crate::token::IntLiteral;

#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PatternKind {
    Wildcard,
    Literal(LiteralPattern),
    Range { start: Id<Pattern>, end: Id<Pattern>, inclusive: bool },
    Binding(Ident),
    At { name: Ident, pattern: Id<Pattern> },
    Or(Vec<Id<Pattern>>),
    Tuple(Vec<Id<Pattern>>),
    Path(Path),
    TupleStruct { path: Path, elements: Vec<Id<Pattern>> },
    Struct { path: Path, fields: Vec<FieldPattern>, rest: bool },
    Type { name: Option<Ident>, ty: Id<Type> },
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LiteralPattern {
    Int { negative: bool, literal: IntLiteral },
    String(Symbol),
    Bool(bool),
    Null,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldPattern {
    pub name: Ident,
    pub pattern: Option<Id<Pattern>>,
}
