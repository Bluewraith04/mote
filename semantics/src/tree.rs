use mote_base::{Arena, Id, Span, Symbol};

use crate::defs::DefId;
use crate::types::TyId;

/// The typed body of one function; every node carries its type.
#[derive(Clone, Default)]
pub struct Body {
    pub locals: Arena<Local>,
    pub exprs: Arena<Expr>,
    pub stmts: Arena<Stmt>,
    pub patterns: Arena<Pattern>,
    /// The receiver first, then the parameters, as locals.
    pub params: Vec<Id<Local>>,
    pub block: Block,
    /// A function returning a `Stream` that holds `yield`.
    pub is_generator: bool,
    /// A call whose arguments are written in another order than its parameters: the `args` positions in evaluation order.
    pub arg_order: std::collections::HashMap<Id<Expr>, Vec<u32>>,
}

impl Body {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn local(&mut self, local: Local) -> Id<Local> {
        self.locals.alloc(local)
    }

    pub fn expr(&mut self, kind: ExprKind, ty: TyId, span: Span) -> Id<Expr> {
        self.exprs.alloc(Expr { kind, ty, span })
    }

    pub fn stmt(&mut self, kind: StmtKind, span: Span) -> Id<Stmt> {
        self.stmts.alloc(Stmt { kind, span })
    }

    pub fn pattern(&mut self, kind: PatternKind, ty: TyId, span: Span) -> Id<Pattern> {
        self.patterns.alloc(Pattern { kind, ty, span })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Block {
    pub stmts: Vec<Id<Stmt>>,
    pub span: Span,
}

/// A name bound by a `let`, a parameter or a pattern.
#[derive(Clone, Debug, PartialEq)]
pub struct Local {
    pub name: Symbol,
    pub ty: TyId,
    pub is_var: bool,
    pub kind: LocalKind,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalKind {
    Let,
    Param,
    /// `self` of a method.
    Receiver,
    /// Bound by a pattern in a `match`, `if let`, `while let` or `for`.
    Pattern,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: TyId,
    pub span: Span,
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
    /// `x ??= v`, only as a compound assignment.
    Coalesce,
}

/// A function with the type arguments it runs at.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FnRef {
    pub def: DefId,
    pub args: Vec<TyId>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Callee {
    /// A named function, method or constructor; a method's object is its first argument.
    Function(FnRef),
    /// A trait method on a type known only after instantiation: `T: Ord` calling `compare`.
    TraitMethod { method: DefId, self_ty: TyId, args: Vec<TyId> },
    /// A function value.
    Value(Id<Expr>),
}

/// An implicit conversion, written out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conversion {
    /// `T` to `T?`.
    ToOptional,
    /// A member to a union holding it.
    ToUnion,
    /// A union to a wider union.
    WidenUnion,
    /// A name to the narrower type a test just proved; nothing is checked.
    Narrow,
    /// A function, `Stream` or `Task` to a type it fits parameter by parameter or element by element.
    Adapt,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldValue {
    /// The field's position in the declaration.
    pub index: u32,
    pub value: Id<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Closure {
    pub params: Vec<Id<Local>>,
    /// The locals of enclosing scopes the body uses.
    pub captures: Vec<Id<Local>>,
    pub body: Block,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    Expr(Id<Expr>),
    Let { pattern: Id<Pattern>, value: Id<Expr> },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ArmBody {
    Value(Id<Expr>),
    Block(Block),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Arm {
    pub pattern: Id<Pattern>,
    pub guard: Option<Id<Expr>>,
    pub body: ArmBody,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub subject: Id<Expr>,
    pub arms: Vec<Arm>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Int(i128),
    Float(f64),
    Bool(bool),
    /// `null`, `None`, typed by its context.
    Null,
    String(Symbol),
    /// The parts of an interpolated string, each already a `String`.
    Concat(Vec<Id<Expr>>),
    /// A number or `Bool` as its text.
    Show(Id<Expr>),
    Local(Id<Local>),
    Global(DefId),
    /// A named function used as a value.
    Function(FnRef),
    Tuple(Vec<Id<Expr>>),
    List(Vec<Id<Expr>>),
    /// `[T]` or `[T; N]` from its elements.
    Array(Vec<Id<Expr>>),
    ArrayRepeat { value: Id<Expr>, len: Id<Expr> },
    Map(Vec<(Id<Expr>, Id<Expr>)>),
    /// A struct or class literal; `base` supplies the fields not listed.
    Record { fields: Vec<FieldValue>, base: Option<Id<Expr>> },
    /// An enum variant with its payload; a tuple payload's fields are numbered in order.
    Variant { variant: u32, fields: Vec<FieldValue> },
    Closure(Box<Closure>),
    Call { callee: Callee, args: Vec<Id<Expr>> },
    Field { object: Id<Expr>, index: u32 },
    TupleField { object: Id<Expr>, index: u32 },
    Index { object: Id<Expr>, index: Id<Expr> },
    /// `xs[a..b]`, `xs[a..b..s]`: a copy of a `List` or `String`; a missing bound is the start or end.
    Slice { object: Id<Expr>, start: Option<Id<Expr>>, end: Option<Id<Expr>>, inclusive: bool, step: Option<Id<Expr>> },
    Unary { op: UnaryOp, operand: Id<Expr> },
    Binary { op: BinaryOp, left: Id<Expr>, right: Id<Expr> },
    /// `e is T`.
    Is { expr: Id<Expr>, ty: TyId },
    /// `e as T`, where the node's type is `T`.
    Cast(Id<Expr>),
    Convert { conversion: Conversion, expr: Id<Expr> },
    /// `e!`.
    Unwrap(Id<Expr>),
    /// `e?`.
    Propagate(Id<Expr>),
    Range { start: Id<Expr>, end: Id<Expr>, inclusive: bool },
    /// A `Task` running `task`; `scoped` when written inside a `scope` of the same function.
    Spawn { task: Box<Closure>, scoped: bool },
    If { condition: Condition, then: Id<Expr>, otherwise: Id<Expr> },
    Match(Match),
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Else {
    If(Id<Stmt>),
    Block(Block),
}

#[derive(Clone, Debug, PartialEq)]
pub struct IfStmt {
    pub condition: Condition,
    pub then: Block,
    pub otherwise: Option<Else>,
}

/// How a `for` steps through its iterable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForKind {
    /// A range of integers.
    Range,
    /// A list or array, by position.
    Indexed,
    /// A stream, until it ends.
    Stream,
    /// A receiver, until the channel closes.
    Receiver,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ForLoop {
    pub pattern: Id<Pattern>,
    pub iterable: Id<Expr>,
    pub kind: ForKind,
    pub body: Block,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StmtKind {
    Let { pattern: Id<Pattern>, init: Id<Expr> },
    /// `op` is `None` for plain assignment; the target is evaluated once.
    Assign { target: Id<Expr>, op: Option<BinaryOp>, value: Id<Expr> },
    If(IfStmt),
    While { condition: Condition, body: Block },
    For(ForLoop),
    Match(Match),
    Scope(Block),
    /// `close` is the resource's `close` method; a generic body has none until instantiation.
    With { local: Id<Local>, init: Id<Expr>, body: Block, close: Option<FnRef> },
    Return(Option<Id<Expr>>),
    Break,
    Continue,
    Yield(Id<Expr>),
    Expr(Id<Expr>),
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub ty: TyId,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Literal {
    Int(i128),
    Bool(bool),
    String(Symbol),
    Null,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldPattern {
    pub index: u32,
    pub pattern: Id<Pattern>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PatternKind {
    Wildcard,
    Bind(Id<Local>),
    /// `name @ pattern`.
    At { local: Id<Local>, pattern: Id<Pattern> },
    Literal(Literal),
    Range { start: i128, end: i128, inclusive: bool },
    Or(Vec<Id<Pattern>>),
    Tuple(Vec<Id<Pattern>>),
    /// `Some(p)` of an optional.
    OptionalSome(Id<Pattern>),
    OptionalNone,
    /// A variant with its payload patterns in order; fields of a struct variant are by index.
    Variant { variant: u32, fields: Vec<FieldPattern> },
    /// A struct or class by fields, `..` leaving the rest out.
    Record { fields: Vec<FieldPattern> },
    /// `n: T` or `_: T`.
    Type { local: Option<Id<Local>>, ty: TyId },
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_builds_nodes_and_keeps_their_types() {
        let mut body = Body::new();
        let span = Span::new(0, 1);
        let one = body.expr(ExprKind::Int(1), TyId::INT, span);
        let two = body.expr(ExprKind::Int(2), TyId::INT, span);
        let sum = body.expr(ExprKind::Binary { op: BinaryOp::Add, left: one, right: two }, TyId::INT, span);
        let ret = body.stmt(StmtKind::Return(Some(sum)), span);
        body.block = Block { stmts: vec![ret], span };
        assert_eq!(body.exprs[sum].ty, TyId::INT);
        assert!(matches!(body.exprs[sum].kind, ExprKind::Binary { op: BinaryOp::Add, .. }));
        assert_eq!(body.stmts[body.block.stmts[0]].kind, StmtKind::Return(Some(sum)));
    }

    #[test]
    fn patterns_bind_locals_of_one_arena() {
        let mut fixture = crate::defs::test_support::Fixture::new();
        let name = fixture.interner.intern("n");
        let mut body = Body::new();
        let span = Span::new(0, 1);
        let local = body.local(Local { name, ty: TyId::INT, is_var: false, kind: LocalKind::Pattern, span });
        let bind = body.pattern(PatternKind::Bind(local), TyId::INT, span);
        let some = body.pattern(PatternKind::OptionalSome(bind), TyId::INT, span);
        assert_eq!(body.patterns[some].kind, PatternKind::OptionalSome(bind));
        assert_eq!(body.locals[local].kind, LocalKind::Pattern);
    }

    #[test]
    fn function_refs_compare_by_definition_and_arguments() {
        let mut fixture = crate::defs::test_support::Fixture::new();
        let def = fixture.def("f");
        let a = FnRef { def, args: vec![TyId::INT] };
        assert_eq!(a, FnRef { def, args: vec![TyId::INT] });
        assert_ne!(a, FnRef { def, args: vec![TyId::STRING] });
    }
}
