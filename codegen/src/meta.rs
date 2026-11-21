//! What the helpers of compiled code know about the program: how each type is shown.

use mote_abi::{Carries, Meta, Shown};
use mote_base::Interner;
use mote_semantics::{BuiltinTy, FloatTy, Payload, Program, TyKind};

use crate::typetable::TypeMap;

fn names(interner: &Interner, fields: impl Iterator<Item = mote_base::Symbol>) -> Vec<String> {
    fields.map(|name| interner.resolve(name).to_string()).collect()
}

/// The meta table of a program whose types are in the map.
pub fn build(program: &Program, interner: &Interner, types: &TypeMap) -> Meta {
    let mut shown = vec![Shown::Opaque; types.table.len()];
    for (ty, id) in types.entries() {
        shown[id.index()] = shown_of(program, interner, ty);
    }
    Meta { string: types.id(mote_semantics::TyId::STRING), dyn_type: None, shown, described: Vec::new() }
}

fn shown_of(program: &Program, interner: &Interner, ty: mote_semantics::TyId) -> Shown {
    match program.types.kind(ty) {
        TyKind::Unit => Shown::Null,
        TyKind::Bool => Shown::Bool,
        TyKind::String => Shown::Text,
        TyKind::Int(int) if int.signed() => Shown::Signed,
        TyKind::Int(_) => Shown::Unsigned,
        TyKind::Float(FloatTy::F32) => Shown::Float32,
        TyKind::Float(FloatTy::F64) => Shown::Float64,
        TyKind::Tuple(_) => Shown::Tuple,
        TyKind::FixedArray(..) => Shown::FixedArray,
        TyKind::Array(_) => Shown::Array,
        TyKind::Optional(_) => Shown::Optional,
        TyKind::Union(_) | TyKind::Any => Shown::Union,
        TyKind::Function(_) => Shown::Closure,
        TyKind::Builtin { kind, .. } => match kind {
            BuiltinTy::List => Shown::List,
            BuiltinTy::Stream => Shown::Closure,
            BuiltinTy::Pointer => Shown::Opaque,
            _ if kind.is_handle() => Shown::Handle,
            _ => Shown::Opaque,
        },
        TyKind::Adt { def, .. } => {
            if program.std.map == Some(*def) {
                Shown::Map
            } else if program.std.set == Some(*def) {
                Shown::Set
            } else if program.std.range == Some(*def) {
                Shown::Range
            } else {
                let definition = &program.defs[*def];
                if let Some(record) = definition.record() {
                    Shown::Record(interner.resolve(definition.name).to_string(), names(interner, record.fields.iter().map(|field| field.name)))
                } else if let Some(item) = definition.enum_def() {
                    Shown::Enum(
                        item.variants
                            .iter()
                            .map(|variant| {
                                let name = interner.resolve(variant.name).to_string();
                                match &variant.payload {
                                    Payload::Unit => (name, Carries::Nothing, Vec::new()),
                                    Payload::Tuple(types) => (name, Carries::Tuple, vec![String::new(); types.len()]),
                                    Payload::Struct(fields) => (name, Carries::Named, names(interner, fields.iter().map(|field| field.name))),
                                }
                            })
                            .collect(),
                    )
                } else {
                    Shown::Opaque
                }
            }
        }
        _ => Shown::Opaque,
    }
}
