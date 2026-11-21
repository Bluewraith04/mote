use mote_base::{Id, Span, Symbol};

use super::{BindingKind, Block, Expr, Ident, Path, Stmt, Type};
use crate::token::{FloatLiteral, IntLiteral};

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub doc: Option<Span>,
    pub attrs: Vec<Attribute>,
    pub is_pub: bool,
    pub kind: ItemKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    Function(Function),
    NativeFunction(NativeFunction),
    Test(Test),
    Record(Record),
    Enum(Enum),
    Trait(Trait),
    TypeAlias(TypeAlias),
    Import(Import),
    Stmt(Id<Stmt>),
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attribute {
    pub name: Ident,
    pub args: Vec<AttrArg>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttrArg {
    pub key: Option<Path>,
    pub value: AttrValue,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AttrValue {
    Path(Path),
    String(Symbol),
    Int(IntLiteral),
    Float(FloatLiteral),
    Bool(bool),
}

#[derive(Clone, Debug, PartialEq)]
pub struct GenericParam {
    pub name: Ident,
    pub bounds: Vec<Id<Type>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub kind: ParamKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ParamKind {
    SelfValue { is_var: bool },
    Variadic { name: Ident, ty: Option<Id<Type>> },
    Named { is_var: bool, name: Ident, ty: Option<Id<Type>>, default: Option<Id<Expr>> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub params: Vec<Param>,
    pub ret: Option<Id<Type>>,
    pub body: Block,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeFunction {
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub params: Vec<Param>,
    pub ret: Option<Id<Type>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Test {
    pub title: Symbol,
    pub body: Block,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordKind {
    Struct,
    Class,
}

/// A `struct` or `class`.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub kind: RecordKind,
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub traits: Vec<Id<Type>>,
    pub members: Vec<Member>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Member {
    Field(Field),
    Method(Method),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub doc: Option<Span>,
    pub is_pub: bool,
    pub binding: Option<BindingKind>,
    pub name: Ident,
    pub ty: Id<Type>,
    pub default: Option<Id<Expr>>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Method {
    pub doc: Option<Span>,
    pub is_pub: bool,
    pub function: Function,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Enum {
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub traits: Vec<Id<Type>>,
    pub variants: Vec<Variant>,
    pub methods: Vec<Method>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Variant {
    pub doc: Option<Span>,
    pub attrs: Vec<Attribute>,
    pub name: Ident,
    pub payload: VariantPayload,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum VariantPayload {
    Unit,
    Discriminant(Id<Expr>),
    Tuple(Vec<Id<Type>>),
    Struct(Vec<Field>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Trait {
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub members: Vec<TraitMember>,
}

/// A trait method: a signature, with a default body when it has one.
#[derive(Clone, Debug, PartialEq)]
pub struct TraitMember {
    pub doc: Option<Span>,
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub params: Vec<Param>,
    pub ret: Option<Id<Type>>,
    pub body: Option<Block>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypeAlias {
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub target: Id<Type>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Import {
    Module { path: ModulePath, alias: Option<Ident> },
    Names { names: Vec<ImportName>, from: ModulePath },
    Glob { from: ModulePath },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImportName {
    pub name: Ident,
    pub alias: Option<Ident>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModulePath {
    pub prefix: Vec<PathPrefix>,
    pub segments: Vec<Ident>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathPrefix {
    Dot,
    DotDot,
    Super,
}
