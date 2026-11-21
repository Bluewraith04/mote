use std::collections::HashSet;

use mote_base::Span;

use crate::cx::Cx;
use crate::defs::{DefKind, LangItem, Payload};
use crate::types::{TyId, TyKind};

impl Cx<'_> {
    /// Whether a value of the type may enter another task.
    pub(crate) fn is_sendable(&mut self, ty: TyId) -> bool {
        self.sendable(ty, &mut Vec::new())
    }

    fn sendable(&mut self, ty: TyId, seen: &mut Vec<TyId>) -> bool {
        let kind = self.program.types.kind(ty).clone();
        match kind {
            TyKind::Error | TyKind::Infer(_) | TyKind::Unit | TyKind::Bool | TyKind::String | TyKind::Int(_) | TyKind::Float(_) => true,
            TyKind::Param { .. } => false,
            TyKind::Array(element) | TyKind::FixedArray(element, _) => self.plain_element(element),
            TyKind::Optional(inner) => self.sendable(inner, seen),
            TyKind::Tuple(members) | TyKind::Union(members) => members.iter().all(|&member| self.sendable(member, seen)),
            TyKind::Function(function) => function.send,
            TyKind::Adt { def, args } => {
                match self.program.defs[def].lang {
                    Some(lang) if lang.is_handle() => return true,
                    Some(LangItem::List | LangItem::Map | LangItem::Set | LangItem::Array | LangItem::Stream | LangItem::Pointer) => return false,
                    _ => {}
                }
                if seen.contains(&ty) {
                    return true;
                }
                seen.push(ty);
                let fields: Vec<TyId> = match &self.program.defs[def].kind {
                    DefKind::Record(record) if record.is_class && record.fields.iter().any(|field| field.is_var) => return false,
                    DefKind::Record(record) => record.fields.iter().map(|field| field.ty).collect(),
                    DefKind::Enum(item) => item
                        .variants
                        .iter()
                        .flat_map(|variant| match &variant.payload {
                            Payload::Unit => Vec::new(),
                            Payload::Tuple(types) => types.clone(),
                            Payload::Struct(fields) => fields.iter().map(|field| field.ty).collect(),
                        })
                        .collect(),
                    _ => return false,
                };
                fields.into_iter().all(|field| {
                    let field = self.program.types.subst(field, &args);
                    self.sendable(field, seen)
                })
            }
        }
    }

    /// Whether an array element is copied by value: a number, `Bool`, `String` or an array of those.
    fn plain_element(&mut self, ty: TyId) -> bool {
        match self.program.types.kind(ty).clone() {
            TyKind::Bool | TyKind::String | TyKind::Int(_) | TyKind::Float(_) => true,
            TyKind::Array(element) | TyKind::FixedArray(element, _) => self.plain_element(element),
            _ => false,
        }
    }

    /// Reports each local a `spawn` block or a `Send` lambda captures that cannot enter another task.
    pub(crate) fn check_captures(&mut self, f: &super::FnCx, captures: &[mote_base::Id<crate::tree::Local>], span: Span, what: &str) {
        for &local in captures {
            let (name, ty, is_var) = {
                let item = &f.body.locals[local];
                (item.name, item.ty, item.is_var)
            };
            if is_var {
                self.error(span, format!("{what} that captures a `var` is not Sendable"));
                return;
            }
            if !self.is_sendable(ty) {
                let message = format!("`{}` is not Sendable: {}", self.text(name), self.show_ty(ty));
                self.error(span, message);
                return;
            }
        }
    }

    fn holds_function_or_stream(&self, ty: TyId) -> bool {
        self.program.types.any(ty, &|kind| match kind {
            TyKind::Function(_) => true,
            TyKind::Adt { def, .. } => self.program.defs[*def].lang == Some(LangItem::Stream),
            _ => false,
        })
    }

    /// What is wrong with a channel, `Mutex` or `Once` type that a type mentions.
    fn handle_problem(&mut self, ty: TyId) -> Option<String> {
        let kind = self.program.types.kind(ty).clone();
        match kind {
            TyKind::Adt { def, args } => {
                match (self.program.defs[def].lang, args.first().copied()) {
                    (Some(LangItem::Sender | LangItem::Receiver), Some(element)) if matches!(self.program.types.kind(element), TyKind::Optional(_)) => {
                        return Some("a channel cannot carry optional messages".to_string());
                    }
                    (Some(LangItem::Sender | LangItem::Receiver), Some(element)) if !self.program.types.has_param(element) && !self.is_sendable(element) => {
                        return Some(format!("a channel cannot carry {}", self.show_ty(element)));
                    }
                    (Some(LangItem::Mutex | LangItem::Once), Some(held)) if self.holds_function_or_stream(held) => {
                        return Some("a `Mutex` or `Once` cannot hold a function or a Stream".to_string());
                    }
                    (Some(LangItem::Once), Some(held)) if !self.program.types.has_param(held) && self.program.changes_inside(held) => {
                        return Some(format!("a `Once` cannot hold {}", self.show_ty(held)));
                    }
                    _ => {}
                }
                args.into_iter().find_map(|arg| self.handle_problem(arg))
            }
            TyKind::Optional(inner) | TyKind::Array(inner) | TyKind::FixedArray(inner, _) => self.handle_problem(inner),
            TyKind::Tuple(members) | TyKind::Union(members) => members.into_iter().find_map(|member| self.handle_problem(member)),
            TyKind::Function(function) => function.params.iter().find_map(|param| self.handle_problem(param.ty)).or_else(|| self.handle_problem(function.ret)),
            _ => None,
        }
    }

    /// Checks every type a body or signature mentions for a channel, `Mutex` or `Once` it cannot have.
    pub(crate) fn check_handle_types(&mut self) {
        let mut seen = HashSet::new();
        let defs: Vec<_> = self.program.defs.iter().map(|(id, _)| id).collect();
        for def in defs {
            let mut found: Vec<(TyId, Span)> = Vec::new();
            let item = &self.program.defs[def];
            match &item.kind {
                DefKind::Function(function) => {
                    found.extend(function.params.iter().map(|param| (param.ty, item.span)));
                    found.push((function.ret, item.span));
                }
                DefKind::Record(record) => found.extend(record.fields.iter().map(|field| (field.ty, field.span))),
                DefKind::Enum(en) => {
                    for variant in &en.variants {
                        match &variant.payload {
                            Payload::Unit => {}
                            Payload::Tuple(types) => found.extend(types.iter().map(|&ty| (ty, variant.span))),
                            Payload::Struct(fields) => found.extend(fields.iter().map(|field| (field.ty, field.span))),
                        }
                    }
                }
                _ => {}
            }
            let bodies = [self.program.checked.get(&def), self.program.global_inits.get(&def)];
            for body in bodies.into_iter().flatten() {
                found.extend(body.exprs.iter().map(|(_, expr)| (expr.ty, expr.span)));
                found.extend(body.locals.iter().map(|(_, local)| (local.ty, local.span)));
            }
            self.report_handle_types(found, &mut seen);
        }
        let modules: Vec<_> = self.program.modules.iter().map(|(id, _)| id).collect();
        for module in modules {
            let Some(body) = self.program.top_level.get(&module) else { continue };
            let found = body.exprs.iter().map(|(_, expr)| (expr.ty, expr.span)).collect();
            self.report_handle_types(found, &mut seen);
        }
    }

    fn report_handle_types(&mut self, found: Vec<(TyId, Span)>, seen: &mut HashSet<TyId>) {
        for (ty, span) in found {
            if seen.insert(ty)
                && let Some(message) = self.handle_problem(ty)
            {
                self.error(span, message);
            }
        }
    }
}
