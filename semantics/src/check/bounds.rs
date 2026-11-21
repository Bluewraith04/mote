use mote_base::{Id, Span};
use mote_syntax as ast;

use super::FnCx;
use crate::cx::Cx;
use crate::defs::{Bound, DefId, DefKind, Payload};
use crate::types::{GenericArg, TyId, TyKind};

impl Cx<'_> {
    /// The bounds on the generic parameters of a struct, class or enum.
    pub(crate) fn type_bounds(&self, def: DefId) -> Vec<Vec<Bound>> {
        match &self.program.defs[def].kind {
            DefKind::Record(record) => record.generics.iter().map(|param| param.bounds().to_vec()).collect(),
            DefKind::Enum(en) => en.generics.iter().map(|param| param.bounds().to_vec()).collect(),
            _ => Vec::new(),
        }
    }

    /// Reports a type argument, at any depth, that does not meet the bounds of its parameter.
    pub(crate) fn check_type_bounds(&mut self, f: &FnCx, ty: TyId, span: Span) {
        match self.program.types.kind(ty).clone() {
            TyKind::Builtin { args, .. } => {
                for arg in args {
                    self.check_type_bounds(f, arg, span);
                }
            }
            TyKind::Adt { def, args } => {
                let bounds = self.type_bounds(def);
                for (arg, bound) in args.iter().zip(&bounds) {
                    if let GenericArg::Type(arg_ty) = arg {
                        self.check_bounds(f, *arg_ty, bound, &args, span);
                    }
                }
                for arg in args {
                    if let GenericArg::Type(arg_ty) = arg {
                        self.check_type_bounds(f, arg_ty, span);
                    }
                }
            }
            TyKind::Tuple(items) | TyKind::Union(items) => {
                for item in items {
                    self.check_type_bounds(f, item, span);
                }
            }
            TyKind::Array(element) | TyKind::FixedArray(element, _) | TyKind::Optional(element) => self.check_type_bounds(f, element, span),
            TyKind::Function(function) => {
                for param in &function.params {
                    self.check_type_bounds(f, param.ty, span);
                }
                self.check_type_bounds(f, function.ret, span);
            }
            _ => {}
        }
    }

    /// A type written in a body, with its type arguments checked against their bounds.
    pub(crate) fn written_type(&mut self, f: &mut FnCx, written: Id<ast::Type>) -> TyId {
        let scope = f.scope.clone();
        let ty = self.lower_type_with(&scope, Some(f), written);
        let span = self.ast(f.module)[written].span;
        self.check_type_bounds(f, ty, span);
        ty
    }

    /// Checks the types in every signature, field and variant.
    pub(crate) fn check_declared_bounds(&mut self) {
        let defs: Vec<DefId> = self.program.defs.iter().map(|(id, _)| id).collect();
        for def in defs {
            let (module, span) = (self.program.defs[def].module, self.program.defs[def].span);
            let mut types: Vec<(TyId, Span)> = Vec::new();
            match &self.program.defs[def].kind {
                DefKind::Function(function) => {
                    types.extend(function.params.iter().map(|param| (param.ty, span)));
                    types.push((function.ret, span));
                }
                DefKind::Record(record) => types.extend(record.fields.iter().map(|field| (field.ty, field.span))),
                DefKind::Enum(en) => {
                    for variant in &en.variants {
                        match &variant.payload {
                            Payload::Unit => {}
                            Payload::Tuple(items) => types.extend(items.iter().map(|&ty| (ty, variant.span))),
                            Payload::Struct(fields) => types.extend(fields.iter().map(|field| (field.ty, field.span))),
                        }
                    }
                }
                _ => continue,
            }
            if types.is_empty() {
                continue;
            }
            let scope = self.scope_for(def);
            let mut f = FnCx::new(module, scope, TyId::ERROR);
            f.bounds = match &self.program.defs[def].kind {
                DefKind::Function(_) => self.param_bounds(def),
                _ => self.type_bounds(def),
            };
            for (ty, span) in types {
                self.check_type_bounds(&f, ty, span);
            }
        }
    }
}
