use mote_base::{Id, Symbol};
use mote_syntax as ast;
use mote_syntax::{Ident, LiteralPattern, PatternKind};

use super::FnCx;
use crate::cx::Cx;
use crate::defs::{DefId, Field, Payload, Resolved};
use crate::tree::{FieldPattern, Literal, Local, LocalKind, Pattern, PatternKind as Tree};
use crate::types::{TyId, TyKind};

/// What a pattern binds: how its locals are made and which names the alternatives of an or-pattern share.
pub(crate) struct PatternCx {
    is_var: bool,
    kind: LocalKind,
    bound: Vec<(Symbol, Id<Local>)>,
    reuse: Option<Vec<(Symbol, Id<Local>)>>,
}

impl PatternCx {
    pub fn new(is_var: bool, kind: LocalKind) -> Self {
        Self { is_var, kind, bound: Vec::new(), reuse: None }
    }

    /// The names the pattern bound, in order.
    pub fn names(&self) -> &[(Symbol, Id<Local>)] {
        &self.bound
    }
}

impl Cx<'_> {
    /// A pattern of a `match` arm or an `if let`: its names are read-only.
    pub(crate) fn arm_pattern(&mut self, f: &mut FnCx, id: Id<ast::Pattern>, ty: TyId) -> Id<Pattern> {
        let mut pcx = PatternCx::new(false, LocalKind::Pattern);
        self.pattern(f, &mut pcx, id, ty)
    }

    fn bind_local(&mut self, f: &mut FnCx, pcx: &mut PatternCx, name: Ident, ty: TyId) -> Id<Local> {
        let found = pcx.reuse.as_ref().map(|reuse| reuse.iter().find(|(bound, _)| *bound == name.name).map(|&(_, local)| local));
        let local = |f: &mut FnCx, pcx: &PatternCx| f.body.local(Local { name: name.name, ty, is_var: pcx.is_var, kind: pcx.kind, span: name.span });
        match found {
            Some(Some(local)) => {
                if f.body.locals[local].ty != ty && ty != TyId::ERROR {
                    self.error(name.span, format!("`{}` has different types in the alternatives", self.text(name.name)));
                }
                pcx.bound.push((name.name, local));
                local
            }
            Some(None) => {
                self.error(name.span, format!("`{}` is not bound by every alternative", self.text(name.name)));
                local(f, pcx)
            }
            None => {
                let made = local(f, pcx);
                self.declare(f, made);
                pcx.bound.push((name.name, made));
                made
            }
        }
    }

    pub(crate) fn pattern(&mut self, f: &mut FnCx, pcx: &mut PatternCx, id: Id<ast::Pattern>, ty: TyId) -> Id<Pattern> {
        let ast = self.ast(f.module);
        let node = &ast[id];
        let span = node.span;
        let kind = match &node.kind {
            PatternKind::Wildcard => Tree::Wildcard,
            PatternKind::Binding(name) => match self.bare_variant(*name, ty) {
                Some(kind) => kind,
                None => Tree::Bind(self.bind_local(f, pcx, *name, ty)),
            },
            PatternKind::Literal(literal) => self.literal_pattern(literal, ty, span),
            PatternKind::Range { start, end, inclusive } => self.range_pattern(f, *start, *end, *inclusive, ty, span),
            PatternKind::At { name, pattern } => {
                let local = self.bind_local(f, pcx, *name, ty);
                Tree::At { local, pattern: self.pattern(f, pcx, *pattern, ty) }
            }
            PatternKind::Or(alternatives) => self.or_pattern(f, pcx, alternatives, ty),
            PatternKind::Tuple(items) => match self.program.types.kind(ty).clone() {
                TyKind::Tuple(elements) if elements.len() == items.len() => Tree::Tuple(items.iter().zip(elements).map(|(&item, element)| self.pattern(f, pcx, item, element)).collect()),
                TyKind::Error => Tree::Error,
                _ => {
                    let message = format!("a tuple pattern of {} elements does not match {}", items.len(), self.show_ty(ty));
                    self.error(span, message);
                    Tree::Error
                }
            },
            PatternKind::Path(path) => self.unit_pattern(f, path, ty),
            PatternKind::TupleStruct { path, elements } => self.tuple_pattern(f, pcx, path, elements, ty, span),
            PatternKind::Struct { path, fields, rest } => self.struct_pattern(f, pcx, path, fields, *rest, ty, span),
            PatternKind::Type { name, ty: written } => {
                let target = self.written_type(f, *written);
                if !self.type_can_match(ty, target) {
                    let message = format!("{} is never {}", self.show_ty(ty), self.show_ty(target));
                    self.error(span, message);
                }
                let local = name.map(|name| self.bind_local(f, pcx, name, target));
                Tree::Type { local, ty: target }
            }
            PatternKind::Error => Tree::Error,
        };
        f.body.pattern(kind, ty, span)
    }

    /// A bare name that is a variant of the subject's type: `None` of an optional or a unit variant.
    fn bare_variant(&mut self, name: Ident, ty: TyId) -> Option<Tree> {
        match self.program.types.kind(ty).clone() {
            TyKind::Optional(_) if self.text(name.name) == "None" => Some(Tree::OptionalNone),
            TyKind::Adt { def, .. } => {
                let en = self.program.defs[def].enum_def()?;
                let Some(index) = en.variants.iter().position(|variant| variant.name == name.name) else {
                    if self.text(name.name).starts_with(|c: char| c.is_uppercase()) {
                        self.error(name.span, format!("{} has no variant `{}`", self.show_ty(ty), self.text(name.name)));
                        return Some(Tree::Error);
                    }
                    return None;
                };
                if en.variants[index].payload != Payload::Unit {
                    self.error(name.span, format!("`{}` has a payload", self.text(name.name)));
                    return Some(Tree::Error);
                }
                Some(Tree::Variant { variant: index as u32, fields: Vec::new() })
            }
            _ => None,
        }
    }

    fn literal_pattern(&mut self, literal: &LiteralPattern, ty: TyId, span: mote_base::Span) -> Tree {
        let kind = self.program.types.kind(ty).clone();
        if kind == TyKind::Error {
            return Tree::Error;
        }
        let (matches, what) = match literal {
            LiteralPattern::Int { .. } => (matches!(kind, TyKind::Int(_)), "number"),
            LiteralPattern::String(_) => (kind == TyKind::String, "string"),
            LiteralPattern::Bool(_) => (kind == TyKind::Bool, "bool"),
            LiteralPattern::Null => (matches!(kind, TyKind::Optional(_)), "null"),
        };
        if !matches {
            let message = format!("a {what} pattern does not match {}", self.show_ty(ty));
            self.error(span, message);
            return Tree::Error;
        }
        match *literal {
            LiteralPattern::Int { negative, literal } => match self.pattern_int(negative, literal.value, ty, span) {
                Some(value) => Tree::Literal(Literal::Int(value)),
                None => Tree::Error,
            },
            LiteralPattern::String(text) => Tree::Literal(Literal::String(text)),
            LiteralPattern::Bool(value) => Tree::Literal(Literal::Bool(value)),
            LiteralPattern::Null => Tree::OptionalNone,
        }
    }

    /// An integer literal pattern's value, if it fits the subject's type.
    fn pattern_int(&mut self, negative: bool, magnitude: u128, ty: TyId, span: mote_base::Span) -> Option<i128> {
        let Ok(magnitude) = i128::try_from(magnitude) else {
            self.error(span, "integer literal is too large");
            return None;
        };
        let value = if negative { -magnitude } else { magnitude };
        if let TyKind::Int(int) = self.program.types.kind(ty)
            && !int.fits(value)
        {
            let message = format!("{value} does not fit {}", int.name());
            self.error(span, message);
            return None;
        }
        Some(value)
    }

    fn range_pattern(&mut self, f: &FnCx, start: Id<ast::Pattern>, end: Id<ast::Pattern>, inclusive: bool, ty: TyId, span: mote_base::Span) -> Tree {
        let kind = self.program.types.kind(ty).clone();
        if kind == TyKind::Error {
            return Tree::Error;
        }
        if !matches!(kind, TyKind::Int(_)) {
            let message = format!("a range pattern does not match {}", self.show_ty(ty));
            self.error(span, message);
            return Tree::Error;
        }
        let ast = self.ast(f.module);
        let mut bounds = [0i128; 2];
        for (slot, id) in bounds.iter_mut().zip([start, end]) {
            let PatternKind::Literal(LiteralPattern::Int { negative, literal }) = ast[id].kind else {
                self.error(ast[id].span, "a range pattern takes integer literals");
                return Tree::Error;
            };
            let Some(value) = self.pattern_int(negative, literal.value, ty, ast[id].span) else { return Tree::Error };
            *slot = value;
        }
        let [start, end] = bounds;
        if start > end || (start == end && !inclusive) {
            self.error(span, "this range matches nothing");
            return Tree::Error;
        }
        Tree::Range { start, end, inclusive }
    }

    fn or_pattern(&mut self, f: &mut FnCx, pcx: &mut PatternCx, alternatives: &[Id<ast::Pattern>], ty: TyId) -> Tree {
        let ast = self.ast(f.module);
        let start = pcx.bound.len();
        let mut checked = Vec::new();
        let mut first: Vec<(Symbol, Id<Local>)> = Vec::new();
        for (position, &alternative) in alternatives.iter().enumerate() {
            if position == 0 {
                checked.push(self.pattern(f, pcx, alternative, ty));
                first = pcx.bound[start..].to_vec();
                continue;
            }
            let saved_reuse = pcx.reuse.replace(first.clone());
            let saved_bound = std::mem::take(&mut pcx.bound);
            checked.push(self.pattern(f, pcx, alternative, ty));
            let got = std::mem::replace(&mut pcx.bound, saved_bound);
            pcx.reuse = saved_reuse;
            for (name, _) in &first {
                if !got.iter().any(|(bound, _)| bound == name) {
                    self.error(ast[alternative].span, format!("`{}` is not bound by every alternative", self.text(*name)));
                }
            }
        }
        Tree::Or(checked)
    }

    /// Whether a value of the subject type can be of the target type.
    pub(crate) fn type_can_match(&self, subject: TyId, target: TyId) -> bool {
        match self.program.types.kind(target) {
            TyKind::Union(members) => members.iter().all(|&member| self.holds_member(subject, member)),
            _ => self.holds_member(subject, target),
        }
    }

    fn holds_member(&self, subject: TyId, member: TyId) -> bool {
        if subject == member || subject == TyId::ERROR || member == TyId::ERROR {
            return true;
        }
        match self.program.types.kind(subject) {
            TyKind::Union(members) => members.contains(&member),
            TyKind::Optional(inner) => member == TyId::UNIT || self.holds_member(*inner, member),
            _ => false,
        }
    }

    /// The variant a pattern path names in the subject's enum.
    fn pattern_variant(&mut self, f: &FnCx, path: &ast::Path, ty: TyId) -> Option<(DefId, Vec<TyId>, usize)> {
        let (def, args) = match self.program.types.kind(ty).clone() {
            TyKind::Error => return None,
            TyKind::Adt { def, args } if self.program.defs[def].enum_def().is_some() => (def, args),
            _ => {
                let message = format!("{} has no variants", self.show_ty(ty));
                self.error(path.span, message);
                return None;
            }
        };
        let last = *path.segments.last().expect("a path has a segment");
        if path.segments.len() > 1 {
            let prefix = ast::Path { segments: path.segments[..path.segments.len() - 1].to_vec(), span: path.span };
            if self.resolve_path(f.module, &prefix)? != Resolved::Def(def) {
                let names: Vec<String> = prefix.segments.iter().map(|segment| self.text(segment.name)).collect();
                let message = format!("expected {}, found {}", self.show_ty(ty), names.join("."));
                self.error(path.span, message);
                return None;
            }
        }
        let index = self.program.defs[def].enum_def().expect("an enum").variants.iter().position(|variant| variant.name == last.name);
        if index.is_none() {
            let message = format!("{} has no variant `{}`", self.show_ty(ty), self.text(last.name));
            self.error(last.span, message);
        }
        Some((def, args, index?))
    }

    fn unit_pattern(&mut self, f: &FnCx, path: &ast::Path, ty: TyId) -> Tree {
        let Some((def, _, index)) = self.pattern_variant(f, path, ty) else { return Tree::Error };
        let variant = &self.program.defs[def].enum_def().expect("an enum").variants[index];
        if variant.payload != Payload::Unit {
            let message = format!("`{}` has a payload", self.text(variant.name));
            self.error(path.span, message);
            return Tree::Error;
        }
        Tree::Variant { variant: index as u32, fields: Vec::new() }
    }

    fn tuple_pattern(&mut self, f: &mut FnCx, pcx: &mut PatternCx, path: &ast::Path, elements: &[Id<ast::Pattern>], ty: TyId, span: mote_base::Span) -> Tree {
        if let TyKind::Optional(inner) = *self.program.types.kind(ty)
            && path.segments.len() == 1
            && self.text(path.segments[0].name) == "Some"
        {
            if elements.len() != 1 {
                self.error(span, "`Some` takes one pattern");
                return Tree::Error;
            }
            return Tree::OptionalSome(self.pattern(f, pcx, elements[0], inner));
        }
        let Some((def, args, index)) = self.pattern_variant(f, path, ty) else { return Tree::Error };
        let variant = &self.program.defs[def].enum_def().expect("an enum").variants[index];
        let Payload::Tuple(types) = variant.payload.clone() else {
            let message = format!("`{}` has no tuple payload", self.text(variant.name));
            self.error(span, message);
            return Tree::Error;
        };
        if types.len() != elements.len() {
            let message = format!("`{}` has {} fields", self.text(variant.name), types.len());
            self.error(span, message);
            return Tree::Error;
        }
        let fields = elements
            .iter()
            .zip(types)
            .enumerate()
            .map(|(position, (&element, declared))| {
                let field_ty = self.program.types.subst(declared, &args);
                FieldPattern { index: position as u32, pattern: self.pattern(f, pcx, element, field_ty) }
            })
            .collect();
        Tree::Variant { variant: index as u32, fields }
    }

    #[allow(clippy::too_many_arguments)]
    fn struct_pattern(&mut self, f: &mut FnCx, pcx: &mut PatternCx, path: &ast::Path, written: &[ast::FieldPattern], rest: bool, ty: TyId, span: mote_base::Span) -> Tree {
        let names: Vec<Symbol> = path.segments.iter().map(|segment| segment.name).collect();
        if let Some(Resolved::Def(def)) = self.quiet_path(f.module, &names)
            && self.program.defs[def].record().is_some()
        {
            let TyKind::Adt { def: subject, args } = self.program.types.kind(ty).clone() else {
                if ty != TyId::ERROR {
                    let message = format!("expected {}, found {}", self.show_ty(ty), self.text(self.program.defs[def].name));
                    self.error(span, message);
                }
                return Tree::Error;
            };
            if subject != def {
                let message = format!("expected {}, found {}", self.show_ty(ty), self.text(self.program.defs[def].name));
                self.error(span, message);
                return Tree::Error;
            }
            let fields = self.program.defs[def].record().expect("a record").fields.clone();
            let outside = self.program.defs[def].module != f.module;
            return Tree::Record { fields: self.field_patterns(f, pcx, &fields, &args, written, rest, outside, span) };
        }
        let Some((def, args, index)) = self.pattern_variant(f, path, ty) else { return Tree::Error };
        let variant = &self.program.defs[def].enum_def().expect("an enum").variants[index];
        let Payload::Struct(fields) = variant.payload.clone() else {
            let message = format!("`{}` has no named fields", self.text(variant.name));
            self.error(span, message);
            return Tree::Error;
        };
        Tree::Variant { variant: index as u32, fields: self.field_patterns(f, pcx, &fields, &args, written, rest, false, span) }
    }

    #[allow(clippy::too_many_arguments)]
    fn field_patterns(&mut self, f: &mut FnCx, pcx: &mut PatternCx, fields: &[Field], args: &[TyId], written: &[ast::FieldPattern], rest: bool, outside: bool, span: mote_base::Span) -> Vec<FieldPattern> {
        let mut seen = vec![false; fields.len()];
        let mut out = Vec::new();
        for entry in written {
            let text = self.text(entry.name.name);
            let Some(index) = fields.iter().position(|field| field.name == entry.name.name) else {
                self.error(entry.name.span, format!("no field `{text}`"));
                continue;
            };
            if seen[index] {
                self.error(entry.name.span, format!("`{text}` is given twice"));
                continue;
            }
            seen[index] = true;
            if outside && !fields[index].is_pub {
                self.error(entry.name.span, format!("`{text}` is private"));
                continue;
            }
            let field_ty = self.program.types.subst(fields[index].ty, args);
            let pattern = match entry.pattern {
                Some(inner) => self.pattern(f, pcx, inner, field_ty),
                None => {
                    let local = self.bind_local(f, pcx, entry.name, field_ty);
                    f.body.pattern(Tree::Bind(local), field_ty, entry.name.span)
                }
            };
            out.push(FieldPattern { index: index as u32, pattern });
        }
        if !rest {
            for (index, field) in fields.iter().enumerate() {
                if !seen[index] {
                    self.error(span, format!("the pattern leaves out `{}`", self.text(field.name)));
                }
            }
        }
        out
    }

    /// Whether the pattern matches every value of its type.
    pub(crate) fn irrefutable(&self, f: &FnCx, id: Id<Pattern>) -> bool {
        let pattern = &f.body.patterns[id];
        match &pattern.kind {
            Tree::Wildcard | Tree::Bind(_) | Tree::Error => true,
            Tree::At { pattern, .. } => self.irrefutable(f, *pattern),
            Tree::Tuple(items) => items.iter().all(|&item| self.irrefutable(f, item)),
            Tree::Record { fields } => fields.iter().all(|field| self.irrefutable(f, field.pattern)),
            Tree::Variant { fields, .. } => {
                let single = matches!(self.program.types.kind(pattern.ty), TyKind::Adt { def, .. } if self.program.defs[*def].enum_def().is_some_and(|en| en.variants.len() == 1));
                single && fields.iter().all(|field| self.irrefutable(f, field.pattern))
            }
            Tree::Or(alternatives) => alternatives.iter().any(|&alternative| self.irrefutable(f, alternative)),
            _ => false,
        }
    }
}
