//! Deep immutability of a type: no value of it holds a `List`, a `Map`, a `Set`, an array, a `var` field of a class or a function.

use std::collections::HashMap;

use mote_semantics::{BuiltinTy, DefKind, GenericArg, Payload, Program, TyId, TyKind};

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

/// `env` gives the generic arguments that `Param` types in `ty` stand for; `open` holds the declarations being checked.
fn check(program: &Program, ty: TyId, env: &[GenericArg], open: &mut Vec<(mote_semantics::DefId, Vec<GenericArg>)>) -> bool {
    match program.types.kind(ty) {
        TyKind::Unit | TyKind::Bool | TyKind::String | TyKind::Int(_) | TyKind::Float(_) | TyKind::Error => true,
        TyKind::Infer(_) | TyKind::Function(_) | TyKind::Array(_) | TyKind::Any => false,
        TyKind::Param { index, .. } => env.get(*index as usize).and_then(GenericArg::as_type).is_some_and(|arg| check(program, arg, &[], open)),
        TyKind::FixedArray(inner, _) | TyKind::Optional(inner) => check(program, *inner, env, open),
        TyKind::Tuple(members) | TyKind::Union(members) => members.iter().all(|&member| check(program, member, env, open)),
        TyKind::Builtin { kind, args } => {
            if !args.iter().all(|&arg| check(program, arg, env, open)) {
                return false;
            }
            match kind {
                BuiltinTy::List => false,
                BuiltinTy::Stream
                | BuiltinTy::Task
                | BuiltinTy::Sender
                | BuiltinTy::Receiver
                | BuiltinTy::Mutex
                | BuiltinTy::Once
                | BuiltinTy::AtomicInt
                | BuiltinTy::AtomicBool
                | BuiltinTy::PortEnd
                | BuiltinTy::Pointer => true,
            }
        }
        TyKind::Adt { def, args } => {
            let args: Vec<GenericArg> = args.clone();
            if !args.iter().filter_map(GenericArg::as_type).all(|arg| check(program, arg, env, open)) {
                return false;
            }
            if program.std.map == Some(*def) || program.std.set == Some(*def) {
                return false;
            }
            let item = &program.defs[*def];
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
