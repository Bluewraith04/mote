use std::collections::HashMap;

use mote_base::{Id, Span, Symbol};
use mote_syntax as ast;

use crate::types::TyId;

pub type DefId = Id<Def>;
pub type ModuleId = Id<Module>;

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

/// A declaration the standard library marks so the compiler can find it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LangItem {
    Bool,
    String,
    Int8,
    Int16,
    Int32,
    Int,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    Float32,
    Float,
    Option,
    Result,
    List,
    Array,
    Map,
    Set,
    Range,
    Stream,
    Task,
    Sender,
    Receiver,
    Mutex,
    Once,
    AtomicInt,
    AtomicBool,
    PortEnd,
    Pointer,
    Display,
    Debug,
    Eq,
    Ord,
    Clone,
    Closeable,
    Key,
    Error,
    ErrorKind,
}

impl LangItem {
    pub const ALL: [LangItem; 38] = [
        LangItem::Bool,
        LangItem::String,
        LangItem::Int8,
        LangItem::Int16,
        LangItem::Int32,
        LangItem::Int,
        LangItem::UInt8,
        LangItem::UInt16,
        LangItem::UInt32,
        LangItem::UInt64,
        LangItem::Float32,
        LangItem::Float,
        LangItem::Option,
        LangItem::Result,
        LangItem::List,
        LangItem::Array,
        LangItem::Map,
        LangItem::Set,
        LangItem::Range,
        LangItem::Stream,
        LangItem::Task,
        LangItem::Sender,
        LangItem::Receiver,
        LangItem::Mutex,
        LangItem::Once,
        LangItem::AtomicInt,
        LangItem::AtomicBool,
        LangItem::PortEnd,
        LangItem::Pointer,
        LangItem::Display,
        LangItem::Debug,
        LangItem::Eq,
        LangItem::Ord,
        LangItem::Clone,
        LangItem::Closeable,
        LangItem::Key,
        LangItem::Error,
        LangItem::ErrorKind,
    ];

    /// The name a `@lang` attribute gives it.
    pub fn name(self) -> &'static str {
        match self {
            LangItem::Bool => "bool",
            LangItem::String => "string",
            LangItem::Int8 => "int8",
            LangItem::Int16 => "int16",
            LangItem::Int32 => "int32",
            LangItem::Int => "int",
            LangItem::UInt8 => "uint8",
            LangItem::UInt16 => "uint16",
            LangItem::UInt32 => "uint32",
            LangItem::UInt64 => "uint64",
            LangItem::Float32 => "float32",
            LangItem::Float => "float",
            LangItem::Option => "option",
            LangItem::Result => "result",
            LangItem::List => "list",
            LangItem::Array => "array",
            LangItem::Map => "map",
            LangItem::Set => "set",
            LangItem::Range => "range",
            LangItem::Stream => "stream",
            LangItem::Task => "task",
            LangItem::Sender => "sender",
            LangItem::Receiver => "receiver",
            LangItem::Mutex => "mutex",
            LangItem::Once => "once",
            LangItem::AtomicInt => "atomic_int",
            LangItem::AtomicBool => "atomic_bool",
            LangItem::PortEnd => "port_end",
            LangItem::Pointer => "pointer",
            LangItem::Display => "display",
            LangItem::Debug => "debug",
            LangItem::Eq => "eq",
            LangItem::Ord => "ord",
            LangItem::Clone => "clone",
            LangItem::Closeable => "closeable",
            LangItem::Key => "key",
            LangItem::Error => "error",
            LangItem::ErrorKind => "error_kind",
        }
    }

    /// Whether values are handles that tasks share and that never change by a write through the holder.
    pub fn is_handle(self) -> bool {
        matches!(self, LangItem::Task | LangItem::Sender | LangItem::Receiver | LangItem::Mutex | LangItem::Once | LangItem::AtomicInt | LangItem::AtomicBool | LangItem::PortEnd)
    }

    /// The built-in type a declaration with this item stands for.
    pub fn primitive(self) -> Option<TyId> {
        Some(match self {
            LangItem::Bool => TyId::BOOL,
            LangItem::String => TyId::STRING,
            LangItem::Int8 => TyId::INT8,
            LangItem::Int16 => TyId::INT16,
            LangItem::Int32 => TyId::INT32,
            LangItem::Int => TyId::INT,
            LangItem::UInt8 => TyId::UINT8,
            LangItem::UInt16 => TyId::UINT16,
            LangItem::UInt32 => TyId::UINT32,
            LangItem::UInt64 => TyId::UINT64,
            LangItem::Float32 => TyId::FLOAT32,
            LangItem::Float => TyId::FLOAT,
            _ => return None,
        })
    }

    pub fn from_name(name: &str) -> Option<LangItem> {
        LangItem::ALL.into_iter().find(|item| item.name() == name)
    }
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
    pub lang: Option<LangItem>,
    /// The type or trait a method belongs to.
    pub owner: Option<DefId>,
    /// `None` for a definition the compiler makes.
    pub origin: Option<Origin>,
    pub kind: DefKind,
}

pub enum DefKind {
    Function(Function),
    Record(Record),
    Enum(Enum),
    Trait(Trait),
    Alias(Alias),
    Global(Global),
    Test,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenericParam {
    pub name: Symbol,
    /// The traits the argument must satisfy.
    pub bounds: Vec<Bound>,
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

/// The library and symbol a `native fn` binds, from `@link`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Link {
    pub library: Symbol,
    pub symbol: Symbol,
    /// `@blocking`: the call runs on a helper thread.
    pub blocking: bool,
}

pub struct Function {
    /// Its own type parameters; an owner's come first in the numbering.
    pub generics: Vec<GenericParam>,
    /// Without the receiver.
    pub params: Vec<Param>,
    pub receiver: Receiver,
    pub ret: TyId,
    /// `false` for a `native fn` and a trait method that has no default.
    pub has_body: bool,
    pub native: bool,
    /// Set by `@link`; a `native fn` needs this or `intrinsic`.
    pub link: Option<Link>,
    /// Set by `@intrinsic`: the name of an operation the compiler or runtime provides.
    pub intrinsic: Option<Symbol>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: Symbol,
    pub ty: TyId,
    pub is_pub: bool,
    pub is_var: bool,
    pub default: Option<Id<ast::Expr>>,
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

#[derive(Default)]
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
            self.program.defs.alloc(Def { name, module: self.module, span: Span::new(0, 0), is_pub: true, lang: None, owner: None, origin: None, kind })
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
    fn lang_names_are_unique_and_round_trip() {
        for item in LangItem::ALL {
            assert_eq!(LangItem::from_name(item.name()), Some(item));
        }
        assert_eq!(LangItem::from_name("nothing"), None);
        let mut names: Vec<_> = LangItem::ALL.iter().map(|item| item.name()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), LangItem::ALL.len());
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
