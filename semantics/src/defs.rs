use std::collections::HashMap;

use mote_base::{Id, Span, Symbol};
use mote_syntax as ast;

use crate::types::{BuiltinTy, FloatTy, IntTy, TyId, TyKind, Types};

pub type DefId = Id<Def>;
pub type ModuleId = Id<Module>;
pub type ExtendId = Id<ExtendDef>;

/// One source file, its syntax tree and the names it can see.
pub struct Module {
    pub path: Vec<Symbol>,
    pub ast: ast::Ast,
    pub is_std: bool,
    /// Every name the module can see, without the prelude.
    pub scope: HashMap<Symbol, Resolved>,
    /// The names another module can import.
    pub exports: HashMap<Symbol, Resolved>,
    /// The source text, and the offset its first byte has in the source map; a doc comment is a span into it.
    pub source: String,
    pub source_start: u32,
}

impl Module {
    /// The text of a doc comment: each `///` and one space after it removed, lines joined with a line break.
    pub fn doc_text(&self, doc: Option<Span>) -> String {
        let Some(doc) = doc else { return String::new() };
        let start = doc.start.saturating_sub(self.source_start) as usize;
        let end = (doc.end.saturating_sub(self.source_start) as usize).min(self.source.len());
        let Some(text) = self.source.get(start..end) else { return String::new() };
        let lines: Vec<&str> = text.lines().map(|line| line.trim_start().strip_prefix("///").map_or(line, |rest| rest.strip_prefix(' ').unwrap_or(rest))).collect();
        lines.join("\n")
    }
}

/// What a name in a module's scope stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolved {
    Def(DefId),
    Module(ModuleId),
}

/// Where a definition sits in its module's syntax tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Origin {
    pub item: Id<ast::Item>,
    /// The method or trait member inside the item.
    pub member: Option<u32>,
}

/// A named thing a program declares.
pub struct Def {
    pub name: Symbol,
    pub module: ModuleId,
    pub span: Span,
    pub is_pub: bool,
    /// The type or trait a method belongs to.
    pub owner: Option<DefId>,
    /// The `extend` block a method belongs to, if any.
    pub extend: Option<ExtendId>,
    /// `None` for a definition the compiler makes.
    pub origin: Option<Origin>,
    pub doc: Option<Span>,
    pub kind: DefKind,
}

pub enum DefKind {
    Function(Function),
    Record(Record),
    Enum(Enum),
    Trait(Trait),
    Alias(Alias),
    Global(Global),
    Const(ConstDef),
    Test,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GenericParam {
    Type {
        name: Symbol,
        /// The traits the argument must satisfy.
        bounds: Vec<Bound>,
    },
    Const {
        name: Symbol,
        ty: TyId,
    },
}

impl GenericParam {
    pub fn name(&self) -> Symbol {
        match self {
            GenericParam::Type { name, .. } | GenericParam::Const { name, .. } => *name,
        }
    }

    pub fn is_const(&self) -> bool {
        matches!(self, GenericParam::Const { .. })
    }

    pub fn bounds(&self) -> &[Bound] {
        match self {
            GenericParam::Type { bounds, .. } => bounds,
            GenericParam::Const { .. } => &[],
        }
    }
}

/// A trait with the type arguments it is applied to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Bound {
    pub def: DefId,
    pub args: Vec<TyId>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: Symbol,
    /// The element type for a variadic parameter.
    pub ty: TyId,
    pub is_var: bool,
    /// Checked in the declaring module and filled in at calls that omit the argument.
    pub default: Option<Id<ast::Expr>>,
    pub variadic: bool,
}

/// How a method takes its object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// A function or a static method.
    None,
    /// `self`.
    Read,
    /// `var self`.
    Write,
}

/// The library and symbol a declaration in `extern "lib" { ... }` binds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Link {
    pub library: Symbol,
    pub symbol: Symbol,
    /// `@blocking`: the call runs on a helper thread.
    pub blocking: bool,
}

pub struct Function {
    /// Its own generic parameters; an owner's come first in the numbering.
    pub generics: Vec<GenericParam>,
    /// Without the receiver.
    pub params: Vec<Param>,
    pub receiver: Receiver,
    pub ret: TyId,
    /// `false` for an `extern` declaration and a trait method that has no default.
    pub has_body: bool,
    /// Set for functions inside `extern "lib" { ... }`.
    pub link: Option<Link>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: Symbol,
    pub ty: TyId,
    pub is_pub: bool,
    pub is_var: bool,
    pub default: Option<Id<ast::Expr>>,
    pub doc: Option<Span>,
    pub span: Span,
}

#[derive(Default)]
pub struct Record {
    pub is_class: bool,
    pub generics: Vec<GenericParam>,
    pub fields: Vec<Field>,
    pub methods: Vec<DefId>,
    /// The traits the declaration lists.
    pub traits: Vec<Bound>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Payload {
    Unit,
    Tuple(Vec<TyId>),
    Struct(Vec<Field>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Variant {
    pub name: Symbol,
    pub payload: Payload,
    pub discriminant: Option<i128>,
    pub span: Span,
}

#[derive(Clone, Default)]
pub struct Enum {
    pub generics: Vec<GenericParam>,
    pub variants: Vec<Variant>,
    pub methods: Vec<DefId>,
    pub traits: Vec<Bound>,
}

#[derive(Default)]
pub struct Trait {
    pub generics: Vec<GenericParam>,
    pub methods: Vec<DefId>,
}

pub struct Alias {
    pub generics: Vec<GenericParam>,
    pub target: TyId,
}

pub struct Global {
    pub ty: TyId,
    pub is_var: bool,
}

pub struct ConstDef {
    pub ty: TyId,
    pub ast_ty: Option<Id<ast::Type>>,
    pub expr: Id<ast::Expr>,
}

/// Key under which `extend` blocks are indexed for fast lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TyHead {
    Unit,
    Bool,
    String,
    Int(IntTy),
    Float(FloatTy),
    Builtin(BuiltinTy),
    Adt(DefId),
    Array,
    FixedArray,
    Optional,
    Tuple(usize),
}

impl TyHead {
    pub fn of(types: &Types, ty: TyId) -> Option<Self> {
        match types.kind(ty) {
            TyKind::Unit => Some(TyHead::Unit),
            TyKind::Bool => Some(TyHead::Bool),
            TyKind::String => Some(TyHead::String),
            TyKind::Int(int) => Some(TyHead::Int(*int)),
            TyKind::Float(float) => Some(TyHead::Float(*float)),
            TyKind::Builtin { kind, .. } => Some(TyHead::Builtin(*kind)),
            TyKind::Adt { def, .. } => Some(TyHead::Adt(*def)),
            TyKind::Array(_) => Some(TyHead::Array),
            TyKind::FixedArray(_, _) => Some(TyHead::FixedArray),
            TyKind::Optional(_) => Some(TyHead::Optional),
            TyKind::Tuple(els) => Some(TyHead::Tuple(els.len())),
            _ => None,
        }
    }
}

/// An `extend [<generics>] Type [: (Traits)] { methods... }` declaration.
pub struct ExtendDef {
    pub generics: Vec<GenericParam>,
    pub target: TyId,
    pub traits: Vec<Bound>,
    pub methods: Vec<DefId>,
    pub module: ModuleId,
    pub origin: Origin,
    pub span: Span,
}

impl Def {
    pub fn function(&self) -> Option<&Function> {
        match &self.kind {
            DefKind::Function(function) => Some(function),
            _ => None,
        }
    }

    pub fn record(&self) -> Option<&Record> {
        match &self.kind {
            DefKind::Record(record) => Some(record),
            _ => None,
        }
    }

    pub fn enum_def(&self) -> Option<&Enum> {
        match &self.kind {
            DefKind::Enum(def) => Some(def),
            _ => None,
        }
    }

    pub fn trait_def(&self) -> Option<&Trait> {
        match &self.kind {
            DefKind::Trait(def) => Some(def),
            _ => None,
        }
    }

    /// Whether values of this definition's type are references.
    pub fn is_class(&self) -> bool {
        matches!(&self.kind, DefKind::Record(record) if record.is_class)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use mote_base::{Interner, Span};

    use super::*;
    use crate::program::Program;

    /// A program with one empty module, for tests that need real ids.
    pub struct Fixture {
        pub program: Program,
        pub interner: Interner,
        pub module: ModuleId,
    }

    impl Fixture {
        pub fn new() -> Self {
            let mut program = Program::new();
            let module = program.modules.alloc(Module { path: Vec::new(), ast: ast::Ast::default(), is_std: false, scope: HashMap::new(), exports: HashMap::new(), source: String::new(), source_start: 0 });
            Self { program, interner: Interner::new(), module }
        }

        /// Adds an empty class under `name`.
        pub fn def(&mut self, name: &str) -> DefId {
            self.add(name, DefKind::Record(Record { is_class: true, ..Record::default() }))
        }

        pub fn add(&mut self, name: &str, kind: DefKind) -> DefId {
            let name = self.interner.intern(name);
            self.program.defs.alloc(Def { name, module: self.module, span: Span::new(0, 0), is_pub: true, owner: None, extend: None, origin: None, doc: None, kind })
        }

        pub fn def_name(&self, def: DefId) -> String {
            self.interner.resolve(self.program.defs[def].name).to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::Fixture;
    use super::*;

    #[test]
    fn builtin_type_names_are_unique_and_round_trip() {
        for item in BuiltinTy::ALL {
            assert_eq!(BuiltinTy::from_name(item.name()), Some(item));
        }
        assert_eq!(BuiltinTy::from_name("nothing"), None);
        let mut names: Vec<_> = BuiltinTy::ALL.iter().map(|item| item.name()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), BuiltinTy::ALL.len());
    }

    #[test]
    fn a_definition_answers_for_its_own_kind_only() {
        let mut fixture = Fixture::new();
        let class = fixture.def("Counter");
        let test = fixture.add("works", DefKind::Test);
        assert!(fixture.program.defs[class].record().is_some());
        assert!(fixture.program.defs[class].is_class());
        assert!(fixture.program.defs[class].function().is_none());
        assert!(fixture.program.defs[test].record().is_none());
        assert!(!fixture.program.defs[test].is_class());
    }
}
