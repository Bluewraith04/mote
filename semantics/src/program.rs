use std::collections::HashMap;

use mote_base::{Arena, Interner, Symbol};

use crate::defs::{Def, DefId, DefKind, LangItem, Module, ModuleId, Resolved};
use crate::reflect::Reflected;
use crate::tree::{Body, FnRef};
use crate::types::{TyId, TyKind, TypeNames, Types};

/// A function at concrete type arguments; each has one typed body.
pub type Instance = FnRef;

/// One step of start-up, before `main`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Init {
    /// Runs the body in `global_inits` under this global, which sets it and any others of its `let`.
    Global(DefId),
    /// Runs a module's top-level statements.
    TopLevel(ModuleId),
}

/// Everything semantics knows about a program.
pub struct Program {
    pub modules: Arena<Module>,
    pub defs: Arena<Def>,
    pub types: Types,
    pub lang: HashMap<LangItem, DefId>,
    /// The module whose exports every module sees without an import.
    pub prelude: Option<ModuleId>,
    /// The checked body of each function, still over its own type parameters.
    pub checked: HashMap<DefId, Body>,
    /// The checked initializer of each global, as one `return`; a destructured `let` keeps one body under its first name that assigns every name.
    pub global_inits: HashMap<DefId, Body>,
    /// The statements of each module's top level that are not a `let` or `var`, in source order.
    pub top_level: HashMap<ModuleId, Body>,
    /// The typed body of each instance.
    pub bodies: HashMap<Instance, Body>,
    /// Instances in the order they were first needed.
    pub order: Vec<Instance>,
    /// The start-up steps in the order they run: each module after the modules it imports, its globals after the globals they read, then its statements.
    pub init: Vec<Init>,
    /// The types `std.meta` describes, in the order they were first asked for.
    pub reflected: Vec<Reflected>,
    /// The position in `reflected` of each described type.
    pub reflect_index: HashMap<TyId, u32>,
    /// The trait each default method copied into a listing type comes from.
    pub listed_defaults: HashMap<DefId, DefId>,
}

impl Default for Program {
    fn default() -> Self {
        Self::new()
    }
}

impl Program {
    pub fn new() -> Self {
        Self {
            modules: Arena::new(),
            defs: Arena::new(),
            types: Types::new(),
            lang: HashMap::new(),
            prelude: None,
            checked: HashMap::new(),
            global_inits: HashMap::new(),
            top_level: HashMap::new(),
            bodies: HashMap::new(),
            order: Vec::new(),
            init: Vec::new(),
            reflected: Vec::new(),
            reflect_index: HashMap::new(),
            listed_defaults: HashMap::new(),
        }
    }

    pub fn lang_item(&self, item: LangItem) -> Option<DefId> {
        self.lang.get(&item).copied()
    }

    /// The methods of a record, enum or trait with the name.
    pub fn methods_named(&self, def: DefId, name: Symbol) -> Vec<DefId> {
        let methods = match &self.defs[def].kind {
            DefKind::Record(record) => &record.methods,
            DefKind::Enum(en) => &en.methods,
            DefKind::Trait(item) => &item.methods,
            _ => return Vec::new(),
        };
        methods.iter().copied().filter(|&method| self.defs[method].name == name).collect()
    }

    /// The declaration whose methods a value of the type has, with the type's arguments.
    pub fn method_owner(&self, ty: TyId) -> Option<(DefId, Vec<TyId>)> {
        match self.types.kind(ty) {
            TyKind::Adt { def, args } => Some((*def, args.clone())),
            TyKind::Optional(inner) => Some((self.lang_item(LangItem::Option)?, vec![*inner])),
            TyKind::Array(element) => Some((self.lang_item(LangItem::Array)?, vec![*element])),
            _ => {
                let item = LangItem::ALL.into_iter().find(|item| item.primitive() == Some(ty))?;
                Some((self.lang_item(item)?, Vec::new()))
            }
        }
    }

    /// The method of a `Map` or `Set` type that the language itself calls: indexing, a literal and `==` run std code.
    pub fn collection_method(&self, ty: TyId, name: Symbol) -> Option<FnRef> {
        let TyKind::Adt { def, args } = self.types.kind(ty) else { return None };
        if !matches!(self.defs[*def].lang, Some(LangItem::Map | LangItem::Set)) {
            return None;
        }
        Some(FnRef { def: *self.methods_named(*def, name).first()?, args: args.clone() })
    }

    /// Whether a value of the type can change without a write to the value that holds it.
    pub fn changes_inside(&self, ty: TyId) -> bool {
        match self.types.kind(ty) {
            TyKind::Array(_) | TyKind::FixedArray(..) => true,
            TyKind::Optional(inner) => self.changes_inside(*inner),
            TyKind::Tuple(members) | TyKind::Union(members) => members.iter().any(|&member| self.changes_inside(member)),
            TyKind::Adt { def, args } => {
                let item = &self.defs[*def];
                match item.lang {
                    Some(LangItem::List | LangItem::Map | LangItem::Set | LangItem::Array) => return true,
                    Some(lang) if lang.is_handle() => return false,
                    _ => {}
                }
                match &item.kind {
                    DefKind::Record(record) if record.is_class => record.fields.iter().any(|field| field.is_var),
                    _ => args.iter().any(|&arg| self.changes_inside(arg)),
                }
            }
            _ => false,
        }
    }

    /// Stores the body of an instance; each is added once.
    pub fn add_body(&mut self, instance: Instance, body: Body) {
        assert!(!self.bodies.contains_key(&instance), "instance already has a body");
        self.order.push(instance.clone());
        self.bodies.insert(instance, body);
    }

    pub fn body(&self, instance: &Instance) -> Option<&Body> {
        self.bodies.get(instance)
    }

    /// A type as source writes it, for messages.
    pub fn render(&self, ty: TyId, interner: &Interner) -> String {
        self.types.render(ty, &Namer { program: self, interner, qualified: false })
    }

    /// A type with the module path of each declaration the prelude and the `main` module do not export, for `typeof`.
    pub fn render_source(&self, ty: TyId, interner: &Interner) -> String {
        self.types.render(ty, &Namer { program: self, interner, qualified: true })
    }
}

struct Namer<'a> {
    program: &'a Program,
    interner: &'a Interner,
    qualified: bool,
}

impl TypeNames for Namer<'_> {
    fn def_name(&self, def: DefId) -> String {
        let item = &self.program.defs[def];
        let name = self.interner.resolve(item.name);
        let path: Vec<&str> = self.program.modules[item.module].path.iter().map(|&segment| self.interner.resolve(segment)).collect();
        let in_prelude = self.program.prelude.is_some_and(|prelude| self.program.modules[prelude].exports.values().any(|found| *found == Resolved::Def(def)));
        if !self.qualified || in_prelude || path == ["main"] {
            return name.to_string();
        }
        format!("{}.{name}", path.join("."))
    }

    fn symbol(&self, symbol: Symbol) -> String {
        self.interner.resolve(symbol).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::test_support::Fixture;

    #[test]
    fn lang_items_are_looked_up_by_kind() {
        let mut fixture = Fixture::new();
        let list = fixture.def("List");
        fixture.program.lang.insert(LangItem::List, list);
        assert_eq!(fixture.program.lang_item(LangItem::List), Some(list));
        assert_eq!(fixture.program.lang_item(LangItem::Map), None);
    }

    #[test]
    fn bodies_are_kept_in_the_order_added() {
        let mut fixture = Fixture::new();
        let a = fixture.def("a");
        let b = fixture.def("b");
        let first = Instance { def: b, args: vec![] };
        let second = Instance { def: a, args: vec![TyId::INT] };
        fixture.program.add_body(first.clone(), Body::new());
        fixture.program.add_body(second.clone(), Body::new());
        assert_eq!(fixture.program.order, [first.clone(), second]);
        assert!(fixture.program.body(&first).is_some());
        assert!(fixture.program.body(&Instance { def: a, args: vec![] }).is_none());
    }

    #[test]
    #[should_panic(expected = "instance already has a body")]
    fn an_instance_has_one_body() {
        let mut fixture = Fixture::new();
        let a = fixture.def("a");
        let instance = Instance { def: a, args: vec![] };
        fixture.program.add_body(instance.clone(), Body::new());
        fixture.program.add_body(instance, Body::new());
    }

    #[test]
    fn render_names_adts_through_the_interner() {
        let mut fixture = Fixture::new();
        let point = fixture.def("Point");
        let ty = fixture.program.types.adt(point, vec![TyId::INT]);
        assert_eq!(fixture.program.render(ty, &fixture.interner), "Point<Int>");
    }
}
