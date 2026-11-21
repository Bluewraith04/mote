use mote_base::{Id, Span, Symbol};
use mote_syntax as ast;
use mote_syntax::{ExprKind, TypeKind};

use crate::cx::{Progress, Cx};
use crate::defs::{Bound, DefId, DefKind, LangItem, ModuleId, Resolved};
use crate::types::{FnParam, TyId};

/// The names a type is written against: its module, type parameters and `Self`.
pub(crate) struct Scope {
    pub module: ModuleId,
    pub params: Vec<Symbol>,
    pub self_ty: Option<TyId>,
    /// Names that stand for a fixed type: a copied default method's trait parameters.
    pub bound: Vec<(Symbol, TyId)>,
}

pub(crate) fn primitive(name: &str) -> Option<TyId> {
    Some(match name {
        "Int" | "Int64" => TyId::INT,
        "Int8" => TyId::INT8,
        "Int16" => TyId::INT16,
        "Int32" => TyId::INT32,
        "UInt8" => TyId::UINT8,
        "UInt16" => TyId::UINT16,
        "UInt32" => TyId::UINT32,
        "UInt64" => TyId::UINT64,
        "Float" | "Float64" => TyId::FLOAT,
        "Float32" => TyId::FLOAT32,
        "Bool" => TyId::BOOL,
        "String" => TyId::STRING,
        "Null" => TyId::UNIT,
        _ => return None,
    })
}

impl Cx<'_> {
    pub(crate) fn lower_type(&mut self, scope: &Scope, ty: Id<ast::Type>) -> TyId {
        let ast = self.ast(scope.module);
        let node = &ast[ty];
        match &node.kind {
            TypeKind::Named(path) => self.lower_path(scope, path, None, node.span),
            TypeKind::Generic { path, args } => {
                let args = args.iter().map(|&arg| self.lower_type(scope, arg)).collect();
                self.lower_path(scope, path, Some(args), node.span)
            }
            TypeKind::Nullable(inner) => {
                let inner = self.lower_type(scope, *inner);
                self.program.types.optional(inner)
            }
            TypeKind::Tuple(elements) => {
                let elements = elements.iter().map(|&element| self.lower_type(scope, element)).collect();
                self.program.types.tuple(elements)
            }
            TypeKind::Array { element, len } => {
                let element = self.lower_type(scope, *element);
                let Some(len) = len else { return self.program.types.array(element) };
                match &ast[*len].kind {
                    ExprKind::Int(literal) if literal.value <= u128::from(u32::MAX) => self.program.types.fixed_array(element, literal.value as u32),
                    _ => {
                        self.error(ast[*len].span, "an array length is an integer literal");
                        TyId::ERROR
                    }
                }
            }
            TypeKind::Function { send, params, ret } => {
                let params = params.iter().map(|param| FnParam { ty: self.lower_type(scope, param.ty), is_var: param.is_var }).collect();
                let ret = self.lower_type(scope, *ret);
                self.program.types.function(params, ret, *send)
            }
            TypeKind::Union(members) => {
                if let Some(&member) = members.iter().find(|&&member| matches!(ast[member].kind, TypeKind::Nullable(_))) {
                    self.error(ast[member].span, "a union member is not an optional; write `(A | B)?`");
                    return TyId::ERROR;
                }
                let members: Vec<TyId> = members.iter().map(|&member| self.lower_type(scope, member)).collect();
                self.program.types.union(&members)
            }
            TypeKind::SelfType => scope.self_ty.unwrap_or_else(|| {
                self.error(node.span, "`Self` is only inside a type");
                TyId::ERROR
            }),
            TypeKind::Paren(inner) => self.lower_type(scope, *inner),
            TypeKind::Error => TyId::ERROR,
        }
    }

    /// The trait a bound or a trait list names.
    pub(super) fn trait_ref(&mut self, scope: &Scope, ty: Id<ast::Type>) -> Option<Bound> {
        let module = scope.module;
        let node = &self.ast(module)[ty];
        let (path, written) = match &node.kind {
            TypeKind::Named(path) => (path, &[][..]),
            TypeKind::Generic { path, args } => (path, args.as_slice()),
            _ => {
                self.error(node.span, "expected a trait");
                return None;
            }
        };
        let last = path.segments.last().expect("a path has a segment");
        let def = match self.resolve_path(module, path)? {
            Resolved::Def(def) if matches!(self.program.defs[def].kind, DefKind::Trait(_)) => def,
            _ => {
                self.error(path.span, format!("`{}` is not a trait", self.text(last.name)));
                return None;
            }
        };
        let arity = match &self.program.defs[def].kind {
            DefKind::Trait(item) => item.generics.len(),
            _ => 0,
        };
        let name = self.text(last.name);
        if written.len() != arity {
            let message = match (written.len(), arity) {
                (0, _) => format!("write the type arguments of `{name}`"),
                (_, 0) => format!("`{name}` takes no type arguments"),
                (_, 1) => format!("`{name}` takes 1 type argument"),
                (_, count) => format!("`{name}` takes {count} type arguments"),
            };
            self.error(node.span, message);
            return None;
        }
        let args = written.iter().map(|&arg| self.lower_type(scope, arg)).collect();
        Some(Bound { def, args })
    }

    /// Follows a dotted name through modules; reports what it cannot find.
    pub(crate) fn resolve_path(&mut self, module: ModuleId, path: &ast::Path) -> Option<Resolved> {
        let first = path.segments[0];
        let Some(mut resolved) = self.lookup(module, first.name) else {
            self.error(first.span, format!("unknown name `{}`", self.text(first.name)));
            return None;
        };
        for segment in &path.segments[1..] {
            let Resolved::Module(target) = resolved else {
                self.error(segment.span, format!("`{}` does not follow a module", self.text(segment.name)));
                return None;
            };
            let Some(&next) = self.program.modules[target].exports.get(&segment.name) else {
                self.missing_name(target, *segment);
                return None;
            };
            resolved = next;
        }
        Some(resolved)
    }

    fn lower_path(&mut self, scope: &Scope, path: &ast::Path, args: Option<Vec<TyId>>, span: Span) -> TyId {
        let first = path.segments[0];
        let single = path.segments.len() == 1;
        if single {
            if let Some(index) = scope.params.iter().position(|&param| param == first.name) {
                if args.is_some() {
                    self.error(span, format!("`{}` takes no type arguments", self.text(first.name)));
                }
                return self.program.types.param(index as u32, first.name);
            }
            if let Some(&(_, ty)) = scope.bound.iter().find(|(name, _)| *name == first.name) {
                if args.is_some() {
                    self.error(span, format!("`{}` takes no type arguments", self.text(first.name)));
                }
                return ty;
            }
            if self.lookup(scope.module, first.name).is_none()
                && let Some(ty) = primitive(&self.text(first.name))
            {
                if args.is_some() {
                    self.error(span, format!("`{}` takes no type arguments", self.text(first.name)));
                }
                return ty;
            }
        }
        match self.resolve_path(scope.module, path) {
            Some(Resolved::Def(def)) => self.def_type(def, args, span),
            Some(Resolved::Module(_)) => {
                let last = path.segments.last().expect("a path has a segment");
                self.error(span, format!("`{}` is a module, not a type", self.text(last.name)));
                TyId::ERROR
            }
            None => TyId::ERROR,
        }
    }

    pub(crate) fn def_type(&mut self, def: DefId, args: Option<Vec<TyId>>, span: Span) -> TyId {
        let name = self.text(self.program.defs[def].name);
        let arity = match &self.program.defs[def].kind {
            DefKind::Record(record) => record.generics.len(),
            DefKind::Enum(en) => en.generics.len(),
            DefKind::Alias(alias) => alias.generics.len(),
            DefKind::Trait(_) => {
                self.error(span, format!("`{name}` is a trait, not a type"));
                return TyId::ERROR;
            }
            _ => {
                self.error(span, format!("`{name}` is not a type"));
                return TyId::ERROR;
            }
        };
        let args = args.unwrap_or_default();
        if args.len() != arity {
            let message = match (args.len(), arity) {
                (0, _) => format!("write the type arguments of `{name}`"),
                (_, 0) => format!("`{name}` takes no type arguments"),
                (_, 1) => format!("`{name}` takes 1 type argument"),
                (_, count) => format!("`{name}` takes {count} type arguments"),
            };
            self.error(span, message);
            return TyId::ERROR;
        }
        if let Some(lang) = self.program.defs[def].lang {
            if lang == LangItem::Option {
                return self.program.types.optional(args[0]);
            }
            if let Some(ty) = lang.primitive() {
                return ty;
            }
        }
        if matches!(self.program.defs[def].kind, DefKind::Alias(_)) {
            if self.aliases.get(&def) == Some(&Progress::Resolving) {
                self.error(span, format!("type alias `{name}` refers to itself"));
                return TyId::ERROR;
            }
            self.ensure_alias(def);
            let DefKind::Alias(alias) = &self.program.defs[def].kind else { unreachable!("checked above") };
            let target = alias.target;
            return self.program.types.subst(target, &args);
        }
        self.program.types.adt(def, args)
    }
}
