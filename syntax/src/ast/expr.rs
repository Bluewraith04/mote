use mote_base::{Id, Span, Symbol};

use super::{Block, Condition, GenericArg, Ident, Pattern, Path, SpawnBody, Type};
use crate::token::{FloatLiteral, IntLiteral};

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Int(IntLiteral),
    Float(FloatLiteral),
    String(Symbol),
    Interpolated { head: Symbol, parts: Vec<InterpPart> },
    Bool(bool),
    Null,
    Ident(Symbol),
    SelfValue,
    Paren(Id<Expr>),
    Tuple(Vec<Id<Expr>>),
    List(Vec<Id<Expr>>),
    ArrayRepeat { value: Id<Expr>, len: Id<Expr> },
    Map(Vec<MapEntry>),
    StructInit { path: Path, args: Vec<GenericArg>, fields: Vec<FieldInit> },
    Lambda { params: Vec<LambdaParam>, ret: Option<Id<Type>>, body: LambdaBody },
    Call { callee: Id<Expr>, args: Vec<Arg> },
    Builtin { name: Ident, generic_args: Vec<GenericArg>, args: Vec<Arg> },
    Member { object: Id<Expr>, name: Ident, args: Vec<GenericArg> },
    TupleIndex { object: Id<Expr>, index: IntLiteral },
    OptionalMember { object: Id<Expr>, name: Ident, args: Vec<GenericArg> },
    Index { object: Id<Expr>, index: Id<Expr> },
    Slice { object: Id<Expr>, start: Option<Id<Expr>>, end: Option<Id<Expr>>, inclusive: bool, step: Option<Id<Expr>> },
    TypeApply { path: Path, args: Vec<GenericArg> },
    Comptime(Block),
    ComptimeIf { condition: Condition, then: Id<Expr>, otherwise: ElseExpr },
    Unwrap(Id<Expr>),
    Propagate(Id<Expr>),
    Unary { op: UnaryOp, operand: Id<Expr> },
    Binary { op: BinaryOp, left: Id<Expr>, right: Id<Expr> },
    TypeTest { expr: Id<Expr>, ty: Id<Type> },
    Cast { expr: Id<Expr>, ty: Id<Type> },
    Range { start: Id<Expr>, end: Id<Expr>, inclusive: bool },
    Spawn(SpawnBody),
    If { condition: Condition, then: Id<Expr>, otherwise: ElseExpr },
    Match { subject: Id<Expr>, arms: Vec<ValueArm> },
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Coalesce,
}

/// A hole of an interpolated string and the text that follows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InterpPart {
    pub value: Id<Expr>,
    pub text: Symbol,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapEntry {
    pub key: Id<Expr>,
    pub value: Id<Expr>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FieldInit {
    Named { name: Ident, value: Id<Expr> },
    Shorthand(Ident),
    Spread(Id<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arg {
    pub name: Option<Ident>,
    pub value: Id<Expr>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LambdaParam {
    pub is_var: bool,
    pub name: Ident,
    pub ty: Option<Id<Type>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LambdaBody {
    Expr(Id<Expr>),
    Block(Block),
}

/// The `else` of a value `if`; `If` is the id of the `else if` expression.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ElseExpr {
    If(Id<Expr>),
    Value(Id<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValueArm {
    pub pattern: Id<Pattern>,
    pub guard: Option<Id<Expr>>,
    pub value: Id<Expr>,
    pub span: Span,
}
