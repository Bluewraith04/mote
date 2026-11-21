use mote_base::{Id, Span};

use super::{Block, ConstDecl, Expr, Function, Ident, Pattern, Type};

#[derive(Clone, Debug, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StmtKind {
    Binding(Binding),
    Const(ConstDecl),
    Assign { target: Id<Expr>, op: AssignOp, value: Id<Expr> },
    If(If),
    ComptimeIf(If),
    While { condition: Condition, body: Block },
    For { pattern: Id<Pattern>, iterable: Id<Expr>, body: Block },
    ComptimeFor { pattern: Id<Pattern>, iterable: Id<Expr>, body: Block },
    Match(Match),
    Scope(Block),
    ComptimeBlock(Block),
    With { name: Ident, init: Id<Expr>, body: Block },
    Spawn(SpawnBody),
    Return(Option<Id<Expr>>),
    Break(Option<Id<Expr>>),
    Continue,
    Yield(Id<Expr>),
    Function(Function),
    Expr(Id<Expr>),
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingKind {
    Let,
    Var,
}

/// A `let` or `var`: `let x: T = e` or `let (a, b) = e`.
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub kind: BindingKind,
    pub pattern: Id<Pattern>,
    pub ty: Option<Id<Type>>,
    pub init: Id<Expr>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignOp {
    Assign,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Coalesce,
}

/// What an `if` or `while` tests: an expression, or a pattern to match.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Condition {
    Expr(Id<Expr>),
    Let { pattern: Id<Pattern>, value: Id<Expr> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct If {
    pub condition: Condition,
    pub then: Block,
    pub otherwise: Option<Else>,
}

/// The `else` of an `if` statement; `If` is the id of the `else if` statement.
#[derive(Clone, Debug, PartialEq)]
pub enum Else {
    If(Id<Stmt>),
    Block(Block),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub subject: Id<Expr>,
    pub arms: Vec<MatchArm>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MatchArm {
    pub pattern: Id<Pattern>,
    pub guard: Option<Id<Expr>>,
    pub body: ArmBody,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ArmBody {
    Block(Block),
    Stmt(Id<Stmt>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum SpawnBody {
    Block(Block),
    Expr(Id<Expr>),
}
