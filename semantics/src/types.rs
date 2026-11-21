use std::collections::{HashMap, HashSet};

use mote_base::Symbol;

use crate::defs::DefId;

/// A type interned in [`Types`]; equal types have equal ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TyId(u32);

impl TyId {
    pub const ERROR: TyId = TyId(0);
    pub const UNIT: TyId = TyId(1);
    pub const BOOL: TyId = TyId(2);
    pub const STRING: TyId = TyId(3);
    pub const INT8: TyId = TyId(4);
    pub const INT16: TyId = TyId(5);
    pub const INT32: TyId = TyId(6);
    pub const INT64: TyId = TyId(7);
    pub const UINT8: TyId = TyId(8);
    pub const UINT16: TyId = TyId(9);
    pub const UINT32: TyId = TyId(10);
    pub const UINT64: TyId = TyId(11);
    pub const FLOAT32: TyId = TyId(12);
    pub const FLOAT64: TyId = TyId(13);
    /// `Int` is the 64-bit default.
    pub const INT: TyId = TyId::INT64;
    /// `Float` is the 64-bit default.
    pub const FLOAT: TyId = TyId::FLOAT64;

    pub fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IntTy {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
}

impl IntTy {
    pub const ALL: [IntTy; 8] = [IntTy::I8, IntTy::I16, IntTy::I32, IntTy::I64, IntTy::U8, IntTy::U16, IntTy::U32, IntTy::U64];

    pub fn bits(self) -> u32 {
        match self {
            IntTy::I8 | IntTy::U8 => 8,
            IntTy::I16 | IntTy::U16 => 16,
            IntTy::I32 | IntTy::U32 => 32,
            IntTy::I64 | IntTy::U64 => 64,
        }
    }

    pub fn signed(self) -> bool {
        matches!(self, IntTy::I8 | IntTy::I16 | IntTy::I32 | IntTy::I64)
    }

    pub fn min(self) -> i128 {
        if self.signed() { -(1i128 << (self.bits() - 1)) } else { 0 }
    }

    pub fn max(self) -> i128 {
        if self.signed() { (1i128 << (self.bits() - 1)) - 1 } else { (1i128 << self.bits()) - 1 }
    }

    pub fn fits(self, value: i128) -> bool {
        (self.min()..=self.max()).contains(&value)
    }

    /// The type name in source: `Int` for the 64-bit default.
    pub fn name(self) -> &'static str {
        match self {
            IntTy::I8 => "Int8",
            IntTy::I16 => "Int16",
            IntTy::I32 => "Int32",
            IntTy::I64 => "Int",
            IntTy::U8 => "UInt8",
            IntTy::U16 => "UInt16",
            IntTy::U32 => "UInt32",
            IntTy::U64 => "UInt64",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FloatTy {
    F32,
    F64,
}

impl FloatTy {
    pub fn name(self) -> &'static str {
        match self {
            FloatTy::F32 => "Float32",
            FloatTy::F64 => "Float",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FnParam {
    pub ty: TyId,
    pub is_var: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FnTy {
    pub params: Vec<FnParam>,
    pub ret: TyId,
    /// A `Send` function type, callable inside `spawn`.
    pub send: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TyKind {
    /// A type that failed to check; it matches anything and reports nothing.
    Error,
    /// `Null`.
    Unit,
    Bool,
    String,
    Int(IntTy),
    Float(FloatTy),
    /// A type parameter of a declaration, by position: the owner's first, then the function's own.
    Param { index: u32, name: Symbol },
    /// An unsolved variable of a call being checked.
    Infer(u32),
    /// A struct, class or enum with its type arguments.
    Adt { def: DefId, args: Vec<TyId> },
    Tuple(Vec<TyId>),
    /// `[T]`.
    Array(TyId),
    /// `[T; N]`.
    FixedArray(TyId, u32),
    /// `T?`.
    Optional(TyId),
    /// Two or more members, sorted by id, none a union.
    Union(Vec<TyId>),
    Function(FnTy),
}

/// Resolves names for rendering a type in a message.
pub trait TypeNames {
    fn def_name(&self, def: DefId) -> String;
    fn symbol(&self, symbol: Symbol) -> String;
}

/// Interns every type of a program.
pub struct Types {
    kinds: Vec<TyKind>,
    lookup: HashMap<TyKind, TyId>,
}

impl Default for Types {
    fn default() -> Self {
        Self::new()
    }
}

impl Types {
    pub fn new() -> Self {
        let mut types = Self { kinds: Vec::new(), lookup: HashMap::new() };
        for kind in [TyKind::Error, TyKind::Unit, TyKind::Bool, TyKind::String] {
            types.intern(kind);
        }
        for int in IntTy::ALL {
            types.intern(TyKind::Int(int));
        }
        types.intern(TyKind::Float(FloatTy::F32));
        types.intern(TyKind::Float(FloatTy::F64));
        debug_assert_eq!(types.int(IntTy::U64), TyId::UINT64);
        debug_assert_eq!(types.float(FloatTy::F64), TyId::FLOAT64);
        types
    }

    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    pub fn intern(&mut self, kind: TyKind) -> TyId {
        if let Some(&id) = self.lookup.get(&kind) {
            return id;
        }
        let id = TyId(u32::try_from(self.kinds.len()).expect("more than u32::MAX types"));
        self.kinds.push(kind.clone());
        self.lookup.insert(kind, id);
        id
    }

    pub fn kind(&self, ty: TyId) -> &TyKind {
        &self.kinds[ty.index()]
    }

    pub fn int(&self, int: IntTy) -> TyId {
        TyId(4 + IntTy::ALL.iter().position(|&candidate| candidate == int).expect("every int type is listed") as u32)
    }

    pub fn float(&self, float: FloatTy) -> TyId {
        match float {
            FloatTy::F32 => TyId::FLOAT32,
            FloatTy::F64 => TyId::FLOAT64,
        }
    }

    pub fn adt(&mut self, def: DefId, args: Vec<TyId>) -> TyId {
        self.intern(TyKind::Adt { def, args })
    }

    pub fn tuple(&mut self, elements: Vec<TyId>) -> TyId {
        self.intern(TyKind::Tuple(elements))
    }

    pub fn array(&mut self, element: TyId) -> TyId {
        self.intern(TyKind::Array(element))
    }

    pub fn fixed_array(&mut self, element: TyId, len: u32) -> TyId {
        self.intern(TyKind::FixedArray(element, len))
    }

    /// `T?`; `T??` stays a different type.
    pub fn optional(&mut self, inner: TyId) -> TyId {
        self.intern(TyKind::Optional(inner))
    }

    pub fn function(&mut self, params: Vec<FnParam>, ret: TyId, send: bool) -> TyId {
        self.intern(TyKind::Function(FnTy { params, ret, send }))
    }

    pub fn param(&mut self, index: u32, name: Symbol) -> TyId {
        self.intern(TyKind::Param { index, name })
    }

    /// Joins members into one type: nested unions flatten, order and duplicates vanish, a single member is itself.
    pub fn union(&mut self, members: &[TyId]) -> TyId {
        let mut flat = Vec::new();
        for &member in members {
            match self.kind(member) {
                TyKind::Union(inner) => flat.extend(inner.iter().copied()),
                _ => flat.push(member),
            }
        }
        if flat.contains(&TyId::ERROR) {
            return TyId::ERROR;
        }
        flat.sort();
        flat.dedup();
        match flat.as_slice() {
            [] => TyId::ERROR,
            [only] => *only,
            _ => self.intern(TyKind::Union(flat)),
        }
    }

    /// Replaces each `Param` by the argument at its index; a parameter past the arguments stays.
    pub fn subst(&mut self, ty: TyId, args: &[TyId]) -> TyId {
        self.map(ty, &|kind| match kind {
            TyKind::Param { index, .. } => args.get(*index as usize).copied(),
            _ => None,
        })
    }

    /// Replaces each solved variable by its type, as deep as the solutions go.
    pub fn solve(&mut self, ty: TyId, solved: &[Option<TyId>]) -> TyId {
        let mut ty = ty;
        while self.has_infer(ty) {
            let next = self.map(ty, &|kind| match kind {
                TyKind::Infer(variable) => solved.get(*variable as usize).copied().flatten(),
                _ => None,
            });
            if next == ty {
                break;
            }
            ty = next;
        }
        ty
    }

    /// Rebuilds the type with `leaf` answering for the nodes it replaces.
    fn map(&mut self, ty: TyId, leaf: &dyn Fn(&TyKind) -> Option<TyId>) -> TyId {
        self.map_with(ty, leaf, &mut HashMap::new())
    }

    /// `map`, answering a type met again from `done` so a type that shares parts is visited once.
    fn map_with(&mut self, ty: TyId, leaf: &dyn Fn(&TyKind) -> Option<TyId>, done: &mut HashMap<TyId, TyId>) -> TyId {
        if let Some(&found) = done.get(&ty) {
            return found;
        }
        let kind = self.kind(ty).clone();
        let result = if let Some(found) = leaf(&kind) {
            found
        } else {
            match kind {
                TyKind::Adt { def, args: inner } => {
                    let inner = self.map_all(&inner, leaf, done);
                    self.adt(def, inner)
                }
                TyKind::Tuple(elements) => {
                    let elements = self.map_all(&elements, leaf, done);
                    self.tuple(elements)
                }
                TyKind::Array(element) => {
                    let element = self.map_with(element, leaf, done);
                    self.array(element)
                }
                TyKind::FixedArray(element, len) => {
                    let element = self.map_with(element, leaf, done);
                    self.fixed_array(element, len)
                }
                TyKind::Optional(inner) => {
                    let inner = self.map_with(inner, leaf, done);
                    self.optional(inner)
                }
                TyKind::Union(members) => {
                    let members = self.map_all(&members, leaf, done);
                    self.union(&members)
                }
                TyKind::Function(function) => {
                    let params = function.params.iter().map(|param| FnParam { ty: self.map_with(param.ty, leaf, done), is_var: param.is_var }).collect();
                    let ret = self.map_with(function.ret, leaf, done);
                    self.function(params, ret, function.send)
                }
                _ => ty,
            }
        };
        done.insert(ty, result);
        result
    }

    fn map_all(&mut self, types: &[TyId], leaf: &dyn Fn(&TyKind) -> Option<TyId>, done: &mut HashMap<TyId, TyId>) -> Vec<TyId> {
        types.iter().map(|&ty| self.map_with(ty, leaf, done)).collect()
    }

    /// Whether the variable occurs anywhere in the type.
    pub fn mentions(&self, ty: TyId, variable: u32) -> bool {
        self.any(ty, &|kind| matches!(kind, TyKind::Infer(other) if *other == variable))
    }

    /// Whether any part of the type is `Error`.
    pub fn has_error(&self, ty: TyId) -> bool {
        self.any(ty, &|kind| matches!(kind, TyKind::Error))
    }

    /// Whether any part of the type is an unsolved variable.
    pub fn has_infer(&self, ty: TyId) -> bool {
        self.any(ty, &|kind| matches!(kind, TyKind::Infer(_)))
    }

    /// Whether any part of the type is a type parameter.
    pub fn has_param(&self, ty: TyId) -> bool {
        self.any(ty, &|kind| matches!(kind, TyKind::Param { .. }))
    }

    /// How many types deep the type nests.
    pub fn depth(&self, ty: TyId) -> usize {
        self.depth_with(ty, &mut HashMap::new())
    }

    fn depth_with(&self, ty: TyId, done: &mut HashMap<TyId, usize>) -> usize {
        if let Some(&found) = done.get(&ty) {
            return found;
        }
        let deepest = |types: &mut dyn Iterator<Item = TyId>, done: &mut HashMap<TyId, usize>| types.map(|inner| self.depth_with(inner, done)).max().unwrap_or(0);
        let inside = match self.kind(ty) {
            TyKind::Adt { args, .. } => deepest(&mut args.iter().copied(), done),
            TyKind::Tuple(types) | TyKind::Union(types) => deepest(&mut types.iter().copied(), done),
            TyKind::Array(inner) | TyKind::FixedArray(inner, _) | TyKind::Optional(inner) => deepest(&mut [*inner].into_iter(), done),
            TyKind::Function(function) => deepest(&mut function.params.iter().map(|param| param.ty).chain([function.ret]), done),
            _ => 0,
        };
        done.insert(ty, inside + 1);
        inside + 1
    }

    /// Whether the test holds for the type or any type inside it.
    pub fn any(&self, ty: TyId, test: &dyn Fn(&TyKind) -> bool) -> bool {
        self.any_seen(ty, test, &mut HashSet::new())
    }

    /// `any`, skipping a type already visited, which cannot change the answer.
    fn any_seen(&self, ty: TyId, test: &dyn Fn(&TyKind) -> bool, seen: &mut HashSet<TyId>) -> bool {
        if !seen.insert(ty) {
            return false;
        }
        let kind = self.kind(ty);
        if test(kind) {
            return true;
        }
        match kind {
            TyKind::Adt { args, .. } => args.iter().any(|&arg| self.any_seen(arg, test, seen)),
            TyKind::Tuple(types) | TyKind::Union(types) => types.iter().any(|&inner| self.any_seen(inner, test, seen)),
            TyKind::Array(inner) | TyKind::FixedArray(inner, _) | TyKind::Optional(inner) => self.any_seen(*inner, test, seen),
            TyKind::Function(function) => function.params.iter().any(|param| self.any_seen(param.ty, test, seen)) || self.any_seen(function.ret, test, seen),
            _ => false,
        }
    }

    /// The type as source writes it, for messages.
    pub fn render(&self, ty: TyId, names: &dyn TypeNames) -> String {
        match self.kind(ty) {
            TyKind::Error => "<error>".to_string(),
            TyKind::Unit => "Null".to_string(),
            TyKind::Bool => "Bool".to_string(),
            TyKind::String => "String".to_string(),
            TyKind::Int(int) => int.name().to_string(),
            TyKind::Float(float) => float.name().to_string(),
            TyKind::Param { name, .. } => names.symbol(*name),
            TyKind::Infer(_) => "_".to_string(),
            TyKind::Adt { def, args } => {
                let name = names.def_name(*def);
                if args.is_empty() { name } else { format!("{name}<{}>", self.render_all(args, names)) }
            }
            TyKind::Tuple(elements) if elements.len() == 1 => format!("({},)", self.render(elements[0], names)),
            TyKind::Tuple(elements) => format!("({})", self.render_all(elements, names)),
            TyKind::Array(element) => format!("[{}]", self.render(*element, names)),
            TyKind::FixedArray(element, len) => format!("[{}; {len}]", self.render(*element, names)),
            TyKind::Optional(inner) => {
                let text = self.render(*inner, names);
                if matches!(self.kind(*inner), TyKind::Union(_) | TyKind::Function(_)) { format!("({text})?") } else { format!("{text}?") }
            }
            TyKind::Union(members) => members
                .iter()
                .map(|&member| {
                    let text = self.render(member, names);
                    if matches!(self.kind(member), TyKind::Function(_)) { format!("({text})") } else { text }
                })
                .collect::<Vec<_>>()
                .join(" | "),
            TyKind::Function(function) => {
                let params: Vec<String> = function
                    .params
                    .iter()
                    .map(|param| if param.is_var { format!("var {}", self.render(param.ty, names)) } else { self.render(param.ty, names) })
                    .collect();
                let send = if function.send { "Send " } else { "" };
                format!("{send}({}) -> {}", params.join(", "), self.render(function.ret, names))
            }
        }
    }

    fn render_all(&self, types: &[TyId], names: &dyn TypeNames) -> String {
        types.iter().map(|&ty| self.render(ty, names)).collect::<Vec<_>>().join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::test_support::Fixture;

    struct Names<'a>(&'a Fixture);

    impl TypeNames for Names<'_> {
        fn def_name(&self, def: DefId) -> String {
            self.0.def_name(def)
        }

        fn symbol(&self, symbol: Symbol) -> String {
            self.0.interner.resolve(symbol).to_string()
        }
    }

    #[test]
    fn primitives_have_fixed_ids() {
        let types = Types::new();
        assert_eq!(types.kind(TyId::ERROR), &TyKind::Error);
        assert_eq!(types.kind(TyId::STRING), &TyKind::String);
        assert_eq!(types.kind(TyId::INT), &TyKind::Int(IntTy::I64));
        assert_eq!(types.kind(TyId::UINT8), &TyKind::Int(IntTy::U8));
        assert_eq!(types.kind(TyId::FLOAT32), &TyKind::Float(FloatTy::F32));
        assert_eq!(types.int(IntTy::U16), TyId::UINT16);
        assert_eq!(types.len(), 14);
    }

    #[test]
    fn equal_types_share_an_id() {
        let mut types = Types::new();
        let a = types.optional(TyId::INT);
        let b = types.optional(TyId::INT);
        let list = types.array(TyId::INT);
        assert_eq!(a, b);
        assert_ne!(a, list);
        assert_ne!(types.optional(a), a);
    }

    #[test]
    fn int_ranges_follow_the_width() {
        assert_eq!(IntTy::I8.min(), -128);
        assert_eq!(IntTy::I8.max(), 127);
        assert_eq!(IntTy::U8.max(), 255);
        assert_eq!(IntTy::U64.max(), u64::MAX as i128);
        assert_eq!(IntTy::I64.min(), i64::MIN as i128);
        assert!(IntTy::U8.fits(255));
        assert!(!IntTy::U8.fits(256));
        assert!(!IntTy::U8.fits(-1));
        assert!(IntTy::I16.fits(-32768));
    }

    #[test]
    fn a_union_is_flat_sorted_and_deduplicated() {
        let mut types = Types::new();
        let a = types.union(&[TyId::STRING, TyId::INT]);
        let b = types.union(&[TyId::INT, TyId::STRING, TyId::INT]);
        let nested = types.union(&[a, TyId::BOOL]);
        let flat = types.union(&[TyId::BOOL, TyId::INT, TyId::STRING]);
        assert_eq!(a, b);
        assert_eq!(nested, flat);
        assert_eq!(types.union(&[TyId::INT, TyId::INT]), TyId::INT);
    }

    #[test]
    fn a_union_with_error_collapses() {
        let mut types = Types::new();
        assert_eq!(types.union(&[TyId::INT, TyId::ERROR]), TyId::ERROR);
    }

    #[test]
    fn subst_replaces_parameters_by_position() {
        let mut fixture = Fixture::new();
        let name = fixture.interner.intern("T");
        let types = &mut fixture.program.types;
        let t = types.param(0, name);
        let list_of_t = types.array(t);
        let opt = types.optional(list_of_t);
        let function = types.function(vec![FnParam { ty: t, is_var: true }], t, false);
        let want_opt = {
            let array = types.array(TyId::INT);
            types.optional(array)
        };
        assert_eq!(types.subst(opt, &[TyId::INT]), want_opt);
        let want_fn = types.function(vec![FnParam { ty: TyId::INT, is_var: true }], TyId::INT, false);
        assert_eq!(types.subst(function, &[TyId::INT]), want_fn);
        assert_eq!(types.subst(t, &[]), t);
    }

    #[test]
    fn subst_renormalizes_unions() {
        let mut fixture = Fixture::new();
        let name = fixture.interner.intern("T");
        let types = &mut fixture.program.types;
        let t = types.param(0, name);
        let union = types.union(&[t, TyId::INT]);
        assert_eq!(types.subst(union, &[TyId::INT]), TyId::INT);
    }

    #[test]
    fn walks_find_errors_variables_and_parameters() {
        let mut fixture = Fixture::new();
        let name = fixture.interner.intern("T");
        let types = &mut fixture.program.types;
        let t = types.param(0, name);
        let infer = types.intern(TyKind::Infer(0));
        let inside = types.tuple(vec![TyId::INT, t]);
        let optional_error = types.optional(TyId::ERROR);
        assert!(types.has_param(inside));
        assert!(!types.has_param(TyId::INT));
        assert!(types.has_infer(infer));
        assert!(types.has_error(optional_error));
        assert!(!types.has_error(inside));
    }

    #[test]
    fn types_render_as_source_writes_them() {
        let mut fixture = Fixture::new();
        let point = fixture.def("Point");
        let name = fixture.interner.intern("T");
        let types = &mut fixture.program.types;
        let t = types.param(0, name);
        let point_ty = types.adt(point, vec![t]);
        let union = types.union(&[TyId::INT, TyId::STRING]);
        let optional_union = types.optional(union);
        let tuple = types.tuple(vec![TyId::INT, TyId::STRING]);
        let one = types.tuple(vec![TyId::INT]);
        let array = types.fixed_array(TyId::UINT8, 4);
        let function = types.function(vec![FnParam { ty: TyId::INT, is_var: false }, FnParam { ty: point_ty, is_var: true }], TyId::UNIT, true);
        let optional_function = types.optional(function);
        let names = Names(&fixture);
        let types = &fixture.program.types;
        assert_eq!(types.render(point_ty, &names), "Point<T>");
        assert_eq!(types.render(optional_union, &names), "(String | Int)?");
        assert_eq!(types.render(tuple, &names), "(Int, String)");
        assert_eq!(types.render(one, &names), "(Int,)");
        assert_eq!(types.render(array, &names), "[UInt8; 4]");
        assert_eq!(types.render(function, &names), "Send (Int, var Point<T>) -> Null");
        assert_eq!(types.render(optional_function, &names), "(Send (Int, var Point<T>) -> Null)?");
        assert_eq!(types.render(TyId::FLOAT32, &names), "Float32");
        assert_eq!(types.render(TyId::FLOAT, &names), "Float");
    }
}
