use mote_base::{Id, Span};

use super::{Arg, Expr, Ident, Path};

#[derive(Clone, Debug, PartialEq)]
pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GenericArg {
    Type(Id<Type>),
    Expr(Id<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum TypeKind {
    Named(Path),
    Generic { path: Path, args: Vec<GenericArg> },
    Builtin { name: Ident, generic_args: Vec<GenericArg>, args: Vec<Arg> },
    Nullable(Id<Type>),
    Tuple(Vec<Id<Type>>),
    Array { element: Id<Type>, len: Option<Id<Expr>> },
    Function { send: bool, params: Vec<FnTypeParam>, ret: Id<Type> },
    Union(Vec<Id<Type>>),
    SelfType,
    Paren(Id<Type>),
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FnTypeParam {
    pub is_var: bool,
    pub ty: Id<Type>,
}
