//! What the helpers of compiled code know about the program: how each type is shown and what `std.meta` describes.

use mote_abi::{Carries, Described, Meta, Shown};
use mote_base::Interner;
use mote_semantics::{FloatTy, LangItem, Payload, Program, TyKind};

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
    let number = |ty| program.reflect_index[&ty];
    let described = program
        .reflected
        .iter()
        .map(|entry| Described {
            kind: entry.kind,
            ty: types.id(entry.ty),
            boxed: types.try_boxed(entry.ty).expect("a described type has a box"),
            args: entry.args.iter().map(|&ty| number(ty)).collect(),
            fields: entry.fields.iter().map(|field| number(field.ty)).collect(),
            variants: entry.variants.iter().map(|variant| variant.fields.iter().map(|field| number(field.ty)).collect()).collect(),
        })
        .collect();
    Meta { string: types.id(mote_semantics::TyId::STRING), dyn_type: types.dyn_type(), shown, described }
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
        TyKind::Union(_) => Shown::Union,
        TyKind::Function(_) => Shown::Closure,
        TyKind::Adt { def, .. } => {
            let definition = &program.defs[*def];
            match definition.lang {
                Some(LangItem::List) => Shown::List,
                Some(LangItem::Array) => Shown::Array,
                Some(LangItem::Map) => Shown::Map,
                Some(LangItem::Set) => Shown::Set,
                Some(LangItem::Range) => Shown::Range,
                Some(LangItem::Stream) => Shown::Closure,
                Some(item) if item.is_handle() => Shown::Handle,
                _ => {
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
        }
        _ => Shown::Opaque,
    }
}
