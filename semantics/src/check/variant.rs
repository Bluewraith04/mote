use mote_base::{Id, Span, Symbol};
use mote_syntax as ast;
use mote_syntax::FieldInit;

use super::FnCx;
use crate::cx::Cx;
use crate::defs::{DefId, Payload, Resolved};
use crate::tree::{Expr, ExprKind, FieldValue};
use crate::types::{TyId, TyKind};

impl Cx<'_> {
    pub(crate) fn variant_index(&self, def: DefId, name: Symbol) -> Option<usize> {
        self.program.defs[def].enum_def()?.variants.iter().position(|variant| variant.name == name)
    }

    /// The enum type the expected type asks for, and its variant of that name.
    pub(crate) fn variant_hint(&self, expected: Option<TyId>, name: Symbol) -> Option<(TyId, usize)> {
        let mut ty = expected?;
        loop {
            match self.program.types.kind(ty) {
                TyKind::Optional(inner) => ty = *inner,
                TyKind::Adt { def, .. } => return Some((ty, self.variant_index(*def, name)?)),
                TyKind::Union(members) => {
                    let mut found = members.iter().filter_map(|&member| match self.program.types.kind(member) {
                        TyKind::Adt { def, .. } => self.variant_index(*def, name).map(|index| (member, index)),
                        _ => None,
                    });
                    let first = found.next()?;
                    return found.next().is_none().then_some(first);
                }
                _ => return None,
            }
        }
    }

    fn enum_parts(&self, ty: TyId) -> (DefId, Vec<TyId>) {
        match self.program.types.kind(ty) {
            TyKind::Adt { def, args } => (*def, args.clone()),
            _ => unreachable!("an enum type"),
        }
    }

    /// A variant built with a call: `Circle(2)`, or `Empty()` for a unit one, which is an error.
    pub(crate) fn variant_value(&mut self, f: &mut FnCx, ty: TyId, index: usize, args: &[ast::Arg], span: Span) -> Id<Expr> {
        let (def, type_args) = self.enum_parts(ty);
        let variant = self.program.defs[def].enum_def().expect("an enum").variants[index].clone();
        let name = self.text(variant.name);
        let fields = match variant.payload {
            Payload::Unit => {
                self.error(span, format!("`{name}` has no payload"));
                return self.error_expr(f, span);
            }
            Payload::Struct(_) => {
                self.error(span, format!("`{name}` is built with fields"));
                return self.error_expr(f, span);
            }
            Payload::Tuple(types) => {
                if let Some(named) = args.iter().find_map(|arg| arg.name) {
                    self.error(named.span, "a variant takes no named arguments");
                    return self.error_expr(f, span);
                }
                if args.len() != types.len() {
                    let message = format!("expected {} arguments, found {}", types.len(), args.len());
                    self.error(span, message);
                    return self.error_expr(f, span);
                }
                let mut fields = Vec::new();
                for (position, (arg, declared)) in args.iter().zip(types).enumerate() {
                    let field_ty = self.program.types.subst(declared, &type_args);
                    let value = self.expr_to(f, arg.value, field_ty);
                    fields.push(FieldValue { index: position as u32, value });
                }
                fields
            }
        };
        let Some(ty) = self.close_instance(f, ty, span) else { return self.error_expr(f, span) };
        f.body.expr(ExprKind::Variant { variant: index as u32, fields }, ty, span)
    }

    /// A variant named with no payload: `Empty`, `Shape.Empty`.
    pub(crate) fn unit_variant(&mut self, f: &mut FnCx, ty: TyId, index: usize, span: Span) -> Id<Expr> {
        let (def, _) = self.enum_parts(ty);
        let variant = &self.program.defs[def].enum_def().expect("an enum").variants[index];
        if variant.payload != Payload::Unit {
            let message = format!("`{}` has a payload", self.text(variant.name));
            self.error(span, message);
            return self.error_expr(f, span);
        }
        let Some(ty) = self.close_instance(f, ty, span) else { return self.error_expr(f, span) };
        f.body.expr(ExprKind::Variant { variant: index as u32, fields: Vec::new() }, ty, span)
    }

    /// `Shape.Named { name: "a" }` or a bare `Named { … }` where the expected type is the enum.
    pub(crate) fn variant_literal(&mut self, f: &mut FnCx, path: &ast::Path, names: &[Symbol], inits: &[FieldInit], expected: Option<TyId>, span: Span) -> Option<Id<Expr>> {
        let (ty, index) = if names.len() == 1 {
            self.variant_hint(expected, names[0])?
        } else {
            let Some(Resolved::Def(def)) = self.quiet_path(f.module, &names[..names.len() - 1]) else { return None };
            let index = self.variant_index(def, names[names.len() - 1])?;
            let Some(ty) = self.open_instance(f, def, Vec::new(), expected, path.span) else { return Some(self.error_expr(f, span)) };
            (ty, index)
        };
        let (def, type_args) = self.enum_parts(ty);
        let variant = self.program.defs[def].enum_def().expect("an enum").variants[index].clone();
        let Payload::Struct(fields) = variant.payload else {
            let message = format!("`{}` is not built with fields", self.text(variant.name));
            self.error(span, message);
            return Some(self.error_expr(f, span));
        };
        let module = self.program.defs[def].module;
        let Some((values, _)) = self.init_fields(f, &fields, module, &type_args, ty, inits, false, false, span) else { return Some(self.error_expr(f, span)) };
        let Some(ty) = self.close_instance(f, ty, span) else { return Some(self.error_expr(f, span)) };
        Some(f.body.expr(ExprKind::Variant { variant: index as u32, fields: values }, ty, span))
    }
}
