//! Deep immutability of a type: no value of it holds a `List`, a `Map`, a `Set`, an array, a `var` field of a class or a function.

use std::collections::HashMap;

use mote_semantics::{DefKind, LangItem, Payload, Program, TyId, TyKind};

#[derive(Default)]
pub(super) struct Immutable {
    known: HashMap<TyId, bool>,
}

impl Immutable {
    pub(super) fn of(&mut self, program: &Program, ty: TyId) -> bool {
        if let Some(&known) = self.known.get(&ty) {
            return known;
        }
        let answer = check(program, ty, &[], &mut Vec::new());
        self.known.insert(ty, answer);
        answer
    }
}

/// `env` gives the type arguments that `Param` types in `ty` stand for; `open` holds the declarations being checked.
fn check(program: &Program, ty: TyId, env: &[TyId], open: &mut Vec<(mote_semantics::DefId, Vec<TyId>)>) -> bool {
    match program.types.kind(ty) {
        TyKind::Unit | TyKind::Bool | TyKind::String | TyKind::Int(_) | TyKind::Float(_) | TyKind::Error => true,
        TyKind::Infer(_) | TyKind::Function(_) | TyKind::Array(_) | TyKind::FixedArray(..) => false,
        TyKind::Param { index, .. } => env.get(*index as usize).is_some_and(|&arg| check(program, arg, &[], open)),
        TyKind::Optional(inner) => check(program, *inner, env, open),
        TyKind::Tuple(members) | TyKind::Union(members) => members.iter().all(|&member| check(program, member, env, open)),
        TyKind::Adt { def, args } => {
            let args: Vec<TyId> = args.clone();
            if !args.iter().all(|&arg| check(program, arg, env, open)) {
                return false;
            }
            let item = &program.defs[*def];
            match item.lang {
                Some(LangItem::List | LangItem::Map | LangItem::Set | LangItem::Array) => return false,
                Some(lang) if lang.is_handle() => return true,
                _ => {}
            }
            let key = (*def, args.clone());
            if open.contains(&key) {
                return true;
            }
            open.push(key);
            let inner = match &item.kind {
                DefKind::Record(record) => record.fields.iter().all(|field| (!record.is_class || !field.is_var) && check(program, field.ty, &args, open)),
                DefKind::Enum(en) => en.variants.iter().all(|variant| match &variant.payload {
                    Payload::Unit => true,
                    Payload::Tuple(types) => types.iter().all(|&member| check(program, member, &args, open)),
                    Payload::Struct(fields) => fields.iter().all(|field| check(program, field.ty, &args, open)),
                }),
                _ => false,
            };
            open.pop();
            inner
        }
    }
}
