use crate::span::Span;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// A parsed source file.
pub struct Program {
    pub items: Vec<Item>,
    pub span: Span,
    /// Every `@stable` item this program declared, with its signature already rendered.
    pub stable_marks: Vec<StableMark>,
    /// Every discovered test: a `test "name" { }` block or a `@test` function. `fn_name` is the function's own name before mangling.
    pub test_marks: Vec<TestMark>,
}

/// One discovered test.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TestMark {
    pub display_name: String,
    pub fn_name: String,
    pub ignored: bool,
    pub span: Span,
}

/// One `@stable pub` item: its name, its kind and a span-free signature that changes only with the item's shape.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StableMark {
    pub name: String,
    pub kind: String,
    pub signature: String,
    pub span: Span,
}

/// An `@name` or `@name(arg, key = "text", …)` preceding an item, a field or a variant.
/// `args` are the bare identifiers inside the parens; `values` are the `key = "text"` pairs, in order.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Attribute {
    pub name: String,
    pub args: Vec<String>,
    pub values: Vec<(String, String)>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// A top-level item.
pub enum Item {
    Function(FunctionDecl),
    /// A bodyless `native fn` — a builtin signature declared in `builtin_natives.mote`.
    NativeFunction(NativeFnDecl),
    Struct(StructDecl),
    Enum(EnumDecl),
    Class(ClassDecl),
    Trait(TraitDecl),
    TypeAlias(TypeAliasDecl),
    Import(ImportDecl),
    TopLevelStmt(Stmt),
}

impl Item {
    pub fn span(&self) -> Span {
        match self {
            Item::Function(f) => f.span,
            Item::NativeFunction(f) => f.span,
            Item::Struct(s) => s.span,
            Item::Enum(e) => e.span,
            Item::Class(c) => c.span,
            Item::Trait(t) => t.span,
            Item::TypeAlias(a) => a.span,
            Item::Import(i) => i.span,
            Item::TopLevelStmt(s) => s.span(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// A module path as written in an `import`.
pub struct ModulePath {
    pub segments: Vec<String>,
    pub is_relative: bool,
    pub relative_depth: usize,
    pub span: Span,
}

impl ModulePath {
    pub fn new(segments: Vec<String>, is_relative: bool, relative_depth: usize, span: Span) -> Self {
        Self { segments, is_relative, relative_depth, span }
    }

    pub fn to_dotted_string(&self) -> String {
        let prefix = if self.relative_depth > 0 {
            ".".repeat(self.relative_depth)
        } else {
            String::new()
        };
        format!("{}{}", prefix, self.segments.join("."))
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// One name imported by a selective `import`.
pub struct ImportSymbol {
    pub name: String,
    pub alias: Option<String>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// An `import` declaration.
pub struct ImportDecl {
    pub path: ModulePath,
    pub from_path: Option<ModulePath>,
    pub alias: Option<String>,
    pub symbols: Vec<ImportSymbol>,
    pub is_pub: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// A generic parameter with its trait bounds.
pub struct GenericParam {
    pub name: String,
    pub bounds: Vec<TypeNode>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// How a parameter is passed or declared.
pub enum ParameterKind {
    Regular {
        name: String,
        ty: Option<TypeNode>,
        default_value: Option<Expr>,
        is_mut: bool,
    },
    SelfValue {
        /// `var self`: the method may write its receiver.
        is_var: bool,
    },
    /// `...xs: T`: collects a call's trailing arguments into a `List<T>` (`Any` when untyped). Must be the last parameter and excludes defaults.
    Variadic {
        name: String,
        ty: Option<TypeNode>,
    },
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// A function parameter.
pub struct Param {
    pub name: String,
    pub ty: Option<TypeNode>,
    pub kind: Option<ParameterKind>,
    pub is_mut: bool,
    pub span: Span,
}

impl Param {
    pub fn regular(name: String, ty: Option<TypeNode>, span: Span) -> Self {
        Self {
            name: name.clone(),
            ty: ty.clone(),
            kind: Some(ParameterKind::Regular {
                name,
                ty,
                default_value: None,
                is_mut: false,
            }),
            is_mut: false,
            span,
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// A function or method declaration.
pub struct FunctionDecl {
    pub name: String,
    pub generic_params: Vec<GenericParam>,
    pub params: Vec<Param>,
    pub return_type: Option<TypeNode>,
    pub body: Vec<Stmt>,
    pub is_pub: bool,
    pub span: Span,
}

/// A bodyless `native fn` declaration; no `body` and no `generic_params` (a generic one is a parse error).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NativeFnDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Option<TypeNode>,
    pub is_pub: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// A struct or class field.
pub struct FieldDecl {
    pub name: String,
    pub ty: TypeNode,
    pub is_mutable: bool,
    /// Readable outside the declaring module.
    pub is_pub: bool,
    /// `@arg(…)` written before the field.
    pub attrs: Vec<Attribute>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// A `struct` declaration.
pub struct StructDecl {
    pub name: String,
    pub generic_params: Vec<GenericParam>,
    /// Traits listed on the declaration line: `struct C: (A, B)`.
    pub traits: Vec<TypeNode>,
    pub fields: Vec<FieldDecl>,
    pub methods: Vec<FunctionDecl>,
    pub is_pub: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// The payload shape of an enum variant.
pub enum EnumVariantKind {
    Unit {
        discriminant: Option<Expr>,
    },
    Tuple(Vec<TypeNode>),
    Struct(Vec<FieldDecl>),
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// An enum variant.
pub struct EnumVariant {
    pub name: String,
    pub kind: EnumVariantKind,
    /// `@arg(…)` written before the variant.
    pub attrs: Vec<Attribute>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// An `enum` declaration.
pub struct EnumDecl {
    pub name: String,
    pub generic_params: Vec<GenericParam>,
    /// Traits listed on the declaration line: `enum C: (A, B)`.
    pub traits: Vec<TypeNode>,
    pub variants: Vec<EnumVariant>,
    pub methods: Vec<FunctionDecl>,
    pub is_pub: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// A `class` declaration.
pub struct ClassDecl {
    pub name: String,
    pub generic_params: Vec<GenericParam>,
    /// Traits listed on the declaration line: `class C: (A, B)`.
    pub traits: Vec<TypeNode>,
    pub fields: Vec<FieldDecl>,
    pub methods: Vec<FunctionDecl>,
    pub is_pub: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// A method signature in a trait.
pub struct TraitMember {
    pub name: String,
    pub generic_params: Vec<GenericParam>,
    pub params: Vec<Param>,
    pub return_type: Option<TypeNode>,
    pub default_body: Option<Vec<Stmt>>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// A `trait` declaration.
pub struct TraitDecl {
    pub name: String,
    pub generic_params: Vec<GenericParam>,
    pub members: Vec<TraitMember>,
    pub is_pub: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// A `type` alias declaration.
pub struct TypeAliasDecl {
    pub name: String,
    pub generic_params: Vec<GenericParam>,
    pub target: TypeNode,
    pub is_pub: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// A written type.
pub enum TypeNode {
    Int(Span),
    Float(Span),
    Bool(Span),
    Char(Span),
    String(Span),
    Null(Span),
    Named(String, Span),
    Nullable(Box<TypeNode>, Span),
    Generic(String, Vec<TypeNode>, Span),
    Tuple(Vec<TypeNode>, Span),
    /// `(A) -> R`; the bool is `true` for `Send (A) -> R`.
    Function(Vec<TypeNode>, Box<TypeNode>, bool, Span),
    Array(Box<TypeNode>, Option<Box<Expr>>, Span),
    SelfType(Span),
    /// `A | B`.
    Union(Vec<TypeNode>, Span),
    /// `var T`: a `var` parameter in a function type.
    VarParam(Box<TypeNode>, Span),
}

impl TypeNode {
    pub fn span(&self) -> Span {
        match self {
            TypeNode::Int(s)
            | TypeNode::Float(s)
            | TypeNode::Bool(s)
            | TypeNode::Char(s)
            | TypeNode::String(s)
            | TypeNode::Null(s)
            | TypeNode::Named(_, s)
            | TypeNode::Nullable(_, s)
            | TypeNode::Generic(_, _, s)
            | TypeNode::Tuple(_, s)
            | TypeNode::Function(_, _, _, s)
            | TypeNode::Array(_, _, s)
            | TypeNode::SelfType(s)
            | TypeNode::Union(_, s)
            | TypeNode::VarParam(_, s) => *s,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
/// A compound assignment operator.
pub enum AssignOp {
    Assign,
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
    ModAssign,
    BitAndAssign,
    BitOrAssign,
    BitXorAssign,
    ShlAssign,
    ShrAssign,
    NullCoalesceAssign,
}

impl AssignOp {
    /// The operator `x op= y` applies; `None` for `=` and `??=`.
    pub(crate) fn binary_op(self) -> Option<BinaryOp> {
        Some(match self {
            AssignOp::AddAssign => BinaryOp::Add,
            AssignOp::SubAssign => BinaryOp::Sub,
            AssignOp::MulAssign => BinaryOp::Mul,
            AssignOp::DivAssign => BinaryOp::Div,
            AssignOp::ModAssign => BinaryOp::Mod,
            AssignOp::BitAndAssign => BinaryOp::BitAnd,
            AssignOp::BitOrAssign => BinaryOp::BitOr,
            AssignOp::BitXorAssign => BinaryOp::BitXor,
            AssignOp::ShlAssign => BinaryOp::Shl,
            AssignOp::ShrAssign => BinaryOp::Shr,
            AssignOp::Assign | AssignOp::NullCoalesceAssign => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// A statement.
pub enum Stmt {
    Let {
        name: String,
        ty: Option<TypeNode>,
        init: Expr,
        /// `true` only for a top-level `pub let`, a global other modules may import.
        is_pub: bool,
        span: Span,
    },
    Var {
        name: String,
        ty: Option<TypeNode>,
        init: Expr,
        /// See `Let::is_pub`.
        is_pub: bool,
        span: Span,
    },
    Assign {
        target: Expr,
        value: Expr,
        span: Span,
    },
    CompoundAssign {
        target: Expr,
        op: AssignOp,
        value: Expr,
        span: Span,
    },
    Expr {
        expr: Expr,
        span: Span,
    },
    If {
        cond: Expr,
        then_branch: Vec<Stmt>,
        else_branch: Option<Vec<Stmt>>,
        span: Span,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
        span: Span,
    },
    ForIn {
        var_name: String,
        /// The iterable. A `Range` literal is lowered as a counter loop; anything else uses the `iter()` / `next()` protocol.
        iter: Expr,
        body: Vec<Stmt>,
        span: Span,
    },
    Match {
        expr: Expr,
        arms: Vec<MatchArm>,
        span: Span,
    },
    /// `scope { … }`: blocks at the closing brace until every task spawned inside it has finished.
    ScopeBlock {
        body: Vec<Stmt>,
        span: Span,
    },
    SpawnBlock {
        body: Vec<Stmt>,
        span: Span,
    },
    Return {
        value: Option<Expr>,
        span: Span,
    },
    Break {
        value: Option<Expr>,
        span: Span,
    },
    Continue {
        span: Span,
    },
    /// `yield e` inside a generator function.
    Yield {
        value: Expr,
        span: Span,
    },
    Block {
        body: Vec<Stmt>,
        span: Span,
    },
    /// `let (a, b) = expr` / `var (a, b) = expr`: flat tuple destructuring; `_` binds nothing. Function bodies only.
    TupleLet {
        names: Vec<String>,
        is_mut: bool,
        is_pub: bool,
        init: Expr,
        span: Span,
    },
    /// `with name = expr { … }`: binds a `Closeable` resource and calls `.close()` on every exit from the block. Function bodies only.
    WithBlock {
        name: String,
        init: Expr,
        body: Vec<Stmt>,
        span: Span,
    },
}

/// Whether `body` contains a `yield` in its own frame, which makes a function a generator.
pub(crate) fn stmts_yield(body: &[Stmt]) -> bool {
    body.iter().any(|s| match s {
        Stmt::Yield { .. } => true,
        Stmt::If { then_branch, else_branch, .. } => {
            stmts_yield(then_branch) || else_branch.as_deref().is_some_and(stmts_yield)
        }
        Stmt::While { body, .. }
        | Stmt::ForIn { body, .. }
        | Stmt::ScopeBlock { body, .. }
        | Stmt::Block { body, .. }
        | Stmt::WithBlock { body, .. } => stmts_yield(body),
        Stmt::Match { arms, .. } => arms.iter().any(|a| stmts_yield(&a.body)),
        _ => false,
    })
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::Let { span, .. }
            | Stmt::Var { span, .. }
            | Stmt::Assign { span, .. }
            | Stmt::CompoundAssign { span, .. }
            | Stmt::Expr { span, .. }
            | Stmt::If { span, .. }
            | Stmt::While { span, .. }
            | Stmt::ForIn { span, .. }
            | Stmt::Match { span, .. }
            | Stmt::ScopeBlock { span, .. }
            | Stmt::SpawnBlock { span, .. }
            | Stmt::Return { span, .. }
            | Stmt::Break { span, .. }
            | Stmt::Continue { span, .. }
            | Stmt::Yield { span, .. }
            | Stmt::Block { span, .. }
            | Stmt::TupleLet { span, .. }
            | Stmt::WithBlock { span, .. } => *span,
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// A field in a struct pattern.
pub struct FieldPattern {
    pub name: String,
    pub pattern: Option<Pattern>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// The payload pattern of an enum variant.
pub enum EnumPatternPayload {
    None,
    Tuple(Vec<Pattern>),
    Struct {
        fields: Vec<FieldPattern>,
        has_rest: bool,
    },
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// A pattern.
pub enum Pattern {
    Wildcard(Span),
    Literal(Box<Expr>, Span),
    Identifier {
        name: String,
        is_mut: bool,
        subpattern: Option<Box<Pattern>>,
        span: Span,
    },
    Tuple(Vec<Pattern>, Span),
    Struct {
        target: TypeNode,
        fields: Vec<FieldPattern>,
        has_rest: bool,
        span: Span,
    },
    Enum {
        target: TypeNode,
        variant: Option<String>,
        payload: EnumPatternPayload,
        span: Span,
    },
    /// `n: T` or `_: T`: matches a value of type `T`, binding it as `n`.
    Type {
        name: Option<String>,
        target: TypeNode,
        span: Span,
    },
    Or(Vec<Pattern>, Span),
    Range {
        start: Box<Pattern>,
        end: Box<Pattern>,
        inclusive: bool,
        span: Span,
    },
}

impl Pattern {
    pub fn span(&self) -> Span {
        match self {
            Pattern::Wildcard(s)
            | Pattern::Literal(_, s)
            | Pattern::Identifier { span: s, .. }
            | Pattern::Tuple(_, s)
            | Pattern::Struct { span: s, .. }
            | Pattern::Enum { span: s, .. }
            | Pattern::Type { span: s, .. }
            | Pattern::Or(_, s)
            | Pattern::Range { span: s, .. } => *s,
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// One arm of a `match`.
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
/// A binary operator.
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
/// A unary operator.
pub enum UnaryOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// An expression.
pub enum Expr {
    Int(i64, Span),
    Float(f64, Span),
    Bool(bool, Span),
    String(String, Span),
    Char(char, Span),
    Null(Span),
    Ident(String, Span),
    SelfValue(Span),
    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
        span: Span,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
        span: Span,
    },
    TypeTest {
        expr: Box<Expr>,
        target_type: TypeNode,
        span: Span,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
        span: Span,
    },
    MemberAccess {
        object: Box<Expr>,
        member: String,
        span: Span,
    },
    StaticAccess {
        target: TypeNode,
        member: String,
        span: Span,
    },
    /// `a?.f` / `a?.m(args)`: `body` reads `temp`, bound to `a`'s payload, and runs only when `a` is not `None`.
    OptionalChain {
        object: Box<Expr>,
        temp: String,
        body: Box<Expr>,
        span: Span,
    },
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
    Range {
        start: Option<Box<Expr>>,
        end: Option<Box<Expr>>,
        inclusive: bool,
        span: Span,
    },
    Ternary {
        cond: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
        span: Span,
    },
    NullCoalesce {
        left: Box<Expr>,
        right: Box<Expr>,
        span: Span,
    },
    ListLiteral {
        elements: Vec<Expr>,
        span: Span,
    },
    MapLiteral {
        entries: Vec<(Expr, Expr)>,
        span: Span,
    },
    TupleLiteral {
        elements: Vec<Expr>,
        span: Span,
    },
    StructInit {
        name: String,
        target_type: Option<TypeNode>,
        fields: Vec<(String, Expr)>,
        span: Span,
    },
    Lambda {
        params: Vec<Param>,
        return_type: Option<TypeNode>,
        body: Vec<Stmt>,
        span: Span,
    },
    If {
        cond: Box<Expr>,
        then_branch: Vec<Stmt>,
        else_branch: Vec<Stmt>,
        span: Span,
    },
    Match {
        expr: Box<Expr>,
        arms: Vec<MatchArm>,
        span: Span,
    },
    /// `spawn { … }` as an expression: its `Task<T>` handle is the value.
    Spawn {
        body: Vec<Stmt>,
        span: Span,
    },
    Try {
        expr: Box<Expr>,
        span: Span,
    },
    Unwrap {
        expr: Box<Expr>,
        span: Span,
    },
    Block {
        body: Vec<Stmt>,
        span: Span,
    },
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Int(_, s)
            | Expr::Float(_, s)
            | Expr::Bool(_, s)
            | Expr::String(_, s)
            | Expr::Char(_, s)
            | Expr::Null(s)
            | Expr::Ident(_, s)
            | Expr::SelfValue(s)
            | Expr::Binary { span: s, .. }
            | Expr::Unary { span: s, .. }
            | Expr::TypeTest { span: s, .. }
            | Expr::Call { span: s, .. }
            | Expr::MemberAccess { span: s, .. }
            | Expr::StaticAccess { span: s, .. }
            | Expr::OptionalChain { span: s, .. }
            | Expr::Index { span: s, .. }
            | Expr::Range { span: s, .. }
            | Expr::Ternary { span: s, .. }
            | Expr::NullCoalesce { span: s, .. }
            | Expr::ListLiteral { span: s, .. }
            | Expr::MapLiteral { span: s, .. }
            | Expr::TupleLiteral { span: s, .. }
            | Expr::StructInit { span: s, .. }
            | Expr::Lambda { span: s, .. }
            | Expr::If { span: s, .. }
            | Expr::Match { span: s, .. }
            | Expr::Spawn { span: s, .. }
            | Expr::Try { span: s, .. }
            | Expr::Unwrap { span: s, .. }
            | Expr::Block { span: s, .. } => *s,
        }
    }
}

impl BinaryOp {
    /// The operator as written in source.
    pub fn symbol(&self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Mod => "%",
            BinaryOp::Eq => "==",
            BinaryOp::NotEq => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::LtEq => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::GtEq => ">=",
            BinaryOp::And => "&&",
            BinaryOp::Or => "||",
            BinaryOp::BitAnd => "&",
            BinaryOp::BitOr => "|",
            BinaryOp::BitXor => "^",
            BinaryOp::Shl => "<<",
            BinaryOp::Shr => ">>",
        }
    }
}
