use std::collections::HashMap;

use mote_base::{Arena, Interner, Symbol};

use crate::defs::{Def, DefId, DefKind, ExtendDef, ExtendId, GenericParam, Module, ModuleId, Resolved, TyHead};
use crate::tree::{Body, FnRef};
use crate::types::{BuiltinTy, ConstArg, ConstVal, GenericArg, TyId, TyKind, TypeNames, Types};

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

/// Canonical definitions from the standard library resolved by module path (no `@lang` attributes).
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct StdItems {
    pub map: Option<DefId>,
    pub set: Option<DefId>,
    pub range: Option<DefId>,
    pub result: Option<DefId>,
    pub error: Option<DefId>,
    pub error_kind: Option<DefId>,
    pub display: Option<DefId>,
    pub debug: Option<DefId>,
    pub eq: Option<DefId>,
    pub ord: Option<DefId>,
    pub clone: Option<DefId>,
    pub closeable: Option<DefId>,
    pub key: Option<DefId>,
    pub type_info: Option<DefId>,
    pub field_info: Option<DefId>,
    pub variant_info: Option<DefId>,
    pub reflect_kind: Option<DefId>,
    pub reflect_shape: Option<DefId>,
    pub span: Option<DefId>,
    pub mut_span: Option<DefId>,
    pub str_view: Option<DefId>,
    pub buffer: Option<DefId>,
    pub bytes: Option<DefId>,
}

/// Everything semantics knows about a program.
pub struct Program {
    pub modules: Arena<Module>,
    pub defs: Arena<Def>,
    pub types: Types,
    pub std: StdItems,
    pub extends: Arena<ExtendDef>,
    pub extend_by_head: HashMap<TyHead, Vec<ExtendId>>,
    pub const_vals: HashMap<DefId, ConstVal>,
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
    /// The concrete normalized return type of each instantiated function.
    pub instance_returns: HashMap<Instance, TyId>,
    /// Instances in the order they were first needed.
    pub order: Vec<Instance>,
    /// The start-up steps in the order they run: each module after the modules it imports, its globals after the globals they read, then its statements.
    pub init: Vec<Init>,
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
            std: StdItems::default(),
            extends: Arena::new(),
            extend_by_head: HashMap::new(),
            const_vals: HashMap::new(),
            prelude: None,
            checked: HashMap::new(),
            global_inits: HashMap::new(),
            top_level: HashMap::new(),
            bodies: HashMap::new(),
            instance_returns: HashMap::new(),
            order: Vec::new(),
            init: Vec::new(),
            listed_defaults: HashMap::new(),
        }
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

    /// The methods with the name declared on an `extend` block.
    pub fn extend_methods_named(&self, extend: ExtendId, name: Symbol) -> Vec<DefId> {
        self.extends[extend]
            .methods
            .iter()
            .copied()
            .filter(|&method| self.defs[method].name == name)
            .collect()
    }

    /// The ADT declaration whose methods a value of the type has, with its generic arguments.
    pub fn method_owner(&self, ty: TyId) -> Option<(DefId, Vec<GenericArg>)> {
        match self.types.kind(ty) {
            TyKind::Adt { def, args } => Some((*def, args.clone())),
            _ => None,
        }
    }

    /// Resolves a concrete method (inherent or from an `extend` block) on a concrete `ty`, returning `(method_def, owner_args)`.
    pub fn resolve_method(&self, ty: TyId, name: Symbol) -> Option<(DefId, Vec<GenericArg>)> {
        if let Some((owner, args)) = self.method_owner(ty)
            && let Some(&found) = self.methods_named(owner, name).first()
        {
            return Some((found, args));
        }
        let head = TyHead::of(&self.types, ty)?;
        let extends = self.extend_by_head.get(&head)?;
        for &ext_id in extends {
            let Some(&found) = self.extend_methods_named(ext_id, name).first() else {
                continue;
            };
            let ext = &self.extends[ext_id];
            if let Some(args) = match_extend_concrete(&self.types, ext, ty) {
                return Some((found, args));
            }
        }
        None
    }

    /// The method of a `Map` or `Set` type that the language itself calls: indexing, a literal and `==` run std code.
    pub fn collection_method(&self, ty: TyId, name: Symbol) -> Option<FnRef> {
        let TyKind::Adt { def, args } = self.types.kind(ty) else { return None };
        if self.std.map != Some(*def) && self.std.set != Some(*def) {
            return None;
        }
        if let Some(&method) = self.methods_named(*def, name).first() {
            return Some(FnRef { def: method, args: args.clone() });
        }
        if let Some(extends) = self.extend_by_head.get(&TyHead::Adt(*def)) {
            for &ext in extends {
                if let Some(&method) = self.extend_methods_named(ext, name).first() {
                    return Some(FnRef { def: method, args: args.clone() });
                }
            }
        }
        None
    }

    /// Whether a value of the type can change without a write to the value that holds it.
    pub fn changes_inside(&self, ty: TyId) -> bool {
        match self.types.kind(ty) {
            TyKind::Array(_) | TyKind::Any => true,
            TyKind::FixedArray(inner, _) | TyKind::Optional(inner) => self.changes_inside(*inner),
            TyKind::Tuple(members) | TyKind::Union(members) => members.iter().any(|&member| self.changes_inside(member)),
            TyKind::Builtin { kind, .. } => *kind == BuiltinTy::List,
            TyKind::Adt { def, args } => {
                if self.std.map == Some(*def) || self.std.set == Some(*def) {
                    return true;
                }
                let item = &self.defs[*def];
                match &item.kind {
                    DefKind::Record(record) if record.is_class => record.fields.iter().any(|field| field.is_var),
                    _ => args.iter().filter_map(GenericArg::as_type).any(|arg| self.changes_inside(arg)),
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

    /// A const generic argument as source writes it.
    pub fn render_const(&self, arg: &ConstArg, interner: &Interner) -> String {
        self.types.render_const(arg, &Namer { program: self, interner, qualified: false })
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

fn match_extend_concrete(types: &Types, ext: &ExtendDef, concrete: TyId) -> Option<Vec<GenericArg>> {
    let mut slots: Vec<Option<GenericArg>> = vec![None; ext.generics.len()];
    if !unify_concrete_pattern(types, ext.target, concrete, &mut slots) {
        return None;
    }
    let mut out = Vec::with_capacity(slots.len());
    for (idx, param) in ext.generics.iter().enumerate() {
        let arg = slots[idx].clone()?;
        match (param, &arg) {
            (GenericParam::Type { .. }, GenericArg::Type(_)) | (GenericParam::Const { .. }, GenericArg::Const(_)) => out.push(arg),
            _ => return None,
        }
    }
    Some(out)
}

fn unify_concrete_pattern(types: &Types, pattern: TyId, concrete: TyId, slots: &mut [Option<GenericArg>]) -> bool {
    match (types.kind(pattern), types.kind(concrete)) {
        (TyKind::Param { index, .. }, _) => {
            let idx = *index as usize;
            if let Some(Some(existing)) = slots.get(idx) {
                return existing.as_type() == Some(concrete);
            }
            if idx < slots.len() {
                slots[idx] = Some(GenericArg::Type(concrete));
                true
            } else {
                false
            }
        }
        _ if pattern == concrete => true,
        (TyKind::Optional(p), TyKind::Optional(c)) | (TyKind::Array(p), TyKind::Array(c)) => unify_concrete_pattern(types, *p, *c, slots),
        (TyKind::FixedArray(p, pn), TyKind::FixedArray(c, cn)) => {
            unify_concrete_pattern(types, *p, *c, slots) && unify_concrete_const(pn, cn, slots)
        }
        (TyKind::Tuple(p), TyKind::Tuple(c)) if p.len() == c.len() => p.iter().zip(c.iter()).all(|(&a, &b)| unify_concrete_pattern(types, a, b, slots)),
        (TyKind::Builtin { kind: pk, args: pa }, TyKind::Builtin { kind: ck, args: ca }) if pk == ck && pa.len() == ca.len() => {
            pa.iter().zip(ca.iter()).all(|(&a, &b)| unify_concrete_pattern(types, a, b, slots))
        }
        (TyKind::Adt { def: pd, args: pa }, TyKind::Adt { def: cd, args: ca }) if pd == cd && pa.len() == ca.len() => {
            pa.iter().zip(ca.iter()).all(|(a, b)| match (a, b) {
                (GenericArg::Type(pt), GenericArg::Type(ct)) => unify_concrete_pattern(types, *pt, *ct, slots),
                (GenericArg::Const(pc), GenericArg::Const(cc)) => unify_concrete_const(pc, cc, slots),
                _ => false,
            })
        }
        _ => false,
    }
}

fn unify_concrete_const(pattern: &ConstArg, concrete: &ConstArg, slots: &mut [Option<GenericArg>]) -> bool {
    match pattern {
        ConstArg::Param { index, .. } => {
            let idx = *index as usize;
            if let Some(Some(existing)) = slots.get(idx) {
                return existing.as_const() == Some(concrete);
            }
            if idx < slots.len() {
                slots[idx] = Some(GenericArg::Const(concrete.clone()));
                true
            } else {
                false
            }
        }
        _ => pattern == concrete,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::test_support::Fixture;

    #[test]
    fn std_items_can_be_recorded_and_read() {
        let mut fixture = Fixture::new();
        let map = fixture.def("Map");
        fixture.program.std.map = Some(map);
        assert_eq!(fixture.program.std.map, Some(map));
        assert_eq!(fixture.program.std.set, None);
    }

    #[test]
    fn bodies_are_kept_in_the_order_added() {
        let mut fixture = Fixture::new();
        let a = fixture.def("a");
        let b = fixture.def("b");
        let first = Instance { def: b, args: vec![] };
        let second = Instance { def: a, args: vec![GenericArg::Type(TyId::INT)] };
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
        let ty = fixture.program.types.adt_types(point, vec![TyId::INT]);
        assert_eq!(fixture.program.render(ty, &fixture.interner), "Point<Int>");
    }
}
