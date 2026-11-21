use mote_base::{Id, Span};

use super::FnCx;
use crate::cx::Cx;
use crate::defs::Payload;
use crate::tree::{FieldPattern, Literal, Pattern, PatternKind};
use crate::types::{TyId, TyKind};

#[derive(Clone, Debug)]
enum Pat {
    Any,
    Con(Con, Vec<Pat>),
    Or(Vec<Pat>),
}

/// A way a value of some type is built.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Con {
    Variant(u32),
    Bool(bool),
    Some,
    None,
    Tuple,
    Record,
    Member(TyId),
    /// A literal or range of a type with too many values to list.
    Other,
}

impl Cx<'_> {
    /// Reports a `match` whose unguarded arms leave a value of the subject's type uncovered.
    pub(crate) fn check_covered(&mut self, f: &FnCx, subject: TyId, arms: &[Id<Pattern>], span: Span) {
        if self.program.types.has_error(subject) {
            return;
        }
        let mut rows = Vec::new();
        for &arm in arms {
            let row = vec![self.lower_pat(f, arm)];
            rows.push(row);
        }
        if let Some(witness) = self.uncovered(rows, &[subject]) {
            self.error(span, format!("match does not cover `{}`", witness[0]));
        }
    }

    fn lower_pat(&mut self, f: &FnCx, id: Id<Pattern>) -> Pat {
        let pattern = &f.body.patterns[id];
        let subject = pattern.ty;
        match &pattern.kind {
            PatternKind::Wildcard | PatternKind::Bind(_) | PatternKind::Error => Pat::Any,
            PatternKind::At { pattern, .. } => self.lower_pat(f, *pattern),
            PatternKind::Or(alternatives) => Pat::Or(alternatives.iter().map(|&alternative| self.lower_pat(f, alternative)).collect()),
            PatternKind::Literal(Literal::Bool(value)) => Pat::Con(Con::Bool(*value), Vec::new()),
            PatternKind::Literal(Literal::Null) | PatternKind::OptionalNone => Pat::Con(Con::None, Vec::new()),
            PatternKind::Literal(_) | PatternKind::Range { .. } => Pat::Con(Con::Other, Vec::new()),
            PatternKind::Tuple(items) => Pat::Con(Con::Tuple, items.iter().map(|&item| self.lower_pat(f, item)).collect()),
            PatternKind::OptionalSome(inner) => Pat::Con(Con::Some, vec![self.lower_pat(f, *inner)]),
            PatternKind::Variant { variant, fields } => {
                let arity = self.constructors(subject).and_then(|all| all.into_iter().find(|(con, _)| *con == Con::Variant(*variant))).map_or(fields.len(), |(_, types)| types.len());
                Pat::Con(Con::Variant(*variant), self.field_pats(f, arity, fields))
            }
            PatternKind::Record { fields } => {
                let arity = self.constructors(subject).map_or(fields.len(), |all| all[0].1.len());
                Pat::Con(Con::Record, self.field_pats(f, arity, fields))
            }
            PatternKind::Type { ty: target, .. } => self.lower_type_pat(subject, *target),
        }
    }

    /// The parts of a variant or record pattern, with `Any` for the fields it leaves out.
    fn field_pats(&mut self, f: &FnCx, arity: usize, fields: &[FieldPattern]) -> Vec<Pat> {
        let mut parts = vec![Pat::Any; arity];
        for field in fields {
            parts[field.index as usize] = self.lower_pat(f, field.pattern);
        }
        parts
    }

    fn lower_type_pat(&self, subject: TyId, target: TyId) -> Pat {
        if subject == target {
            return Pat::Any;
        }
        match (self.program.types.kind(subject), self.program.types.kind(target)) {
            (TyKind::Union(_), TyKind::Union(members)) => Pat::Or(members.iter().map(|&member| Pat::Con(Con::Member(member), Vec::new())).collect()),
            (TyKind::Optional(_), _) if target == TyId::UNIT => Pat::Con(Con::None, Vec::new()),
            (TyKind::Optional(inner), _) if *inner == target => Pat::Con(Con::Some, vec![Pat::Any]),
            _ => Pat::Con(Con::Member(target), Vec::new()),
        }
    }

    /// Every constructor of a type with the types of their parts; `None` for a type that cannot be listed.
    fn constructors(&mut self, ty: TyId) -> Option<Vec<(Con, Vec<TyId>)>> {
        match self.program.types.kind(ty).clone() {
            TyKind::Bool => Some(vec![(Con::Bool(true), Vec::new()), (Con::Bool(false), Vec::new())]),
            TyKind::Optional(inner) => Some(vec![(Con::Some, vec![inner]), (Con::None, Vec::new())]),
            TyKind::Tuple(elements) => Some(vec![(Con::Tuple, elements)]),
            TyKind::Union(members) => Some(members.iter().map(|&member| (Con::Member(member), Vec::new())).collect()),
            TyKind::Adt { def, args } => {
                let item = &self.program.defs[def];
                let shapes: Vec<(Con, Vec<TyId>)> = match item.enum_def() {
                    Some(en) => en
                        .variants
                        .iter()
                        .enumerate()
                        .map(|(index, variant)| {
                            let parts = match &variant.payload {
                                Payload::Unit => Vec::new(),
                                Payload::Tuple(types) => types.clone(),
                                Payload::Struct(fields) => fields.iter().map(|field| field.ty).collect(),
                            };
                            (Con::Variant(index as u32), parts)
                        })
                        .collect(),
                    None => vec![(Con::Record, item.record()?.fields.iter().map(|field| field.ty).collect())],
                };
                Some(shapes.into_iter().map(|(con, parts)| (con, parts.into_iter().map(|part| self.program.types.subst_generic(part, &args)).collect())).collect())
            }
            _ => None,
        }
    }

    /// A value of the types the rows do not cover, written as a pattern; `None` when every value is covered.
    fn uncovered(&mut self, rows: Vec<Vec<Pat>>, types: &[TyId]) -> Option<Vec<String>> {
        let Some((&ty, rest)) = types.split_first() else { return rows.is_empty().then(Vec::new) };
        let rows = expand(rows);
        let all = self.constructors(ty);
        let heads: Vec<Con> = rows
            .iter()
            .filter_map(|row| match &row[0] {
                Pat::Con(con, _) => Some(*con),
                _ => None,
            })
            .collect();
        if let Some(all) = &all
            && all.iter().all(|(con, _)| heads.contains(con))
        {
            for (con, parts) in all {
                let specialized: Vec<Vec<Pat>> = rows.iter().filter_map(|row| specialize(row, con, parts.len())).collect();
                let mut next: Vec<TyId> = parts.clone();
                next.extend_from_slice(rest);
                if let Some(mut witness) = self.uncovered(specialized, &next) {
                    let tail = witness.split_off(parts.len());
                    let mut out = vec![self.show_con(ty, *con, &witness)];
                    out.extend(tail);
                    return Some(out);
                }
            }
            return None;
        }
        let default: Vec<Vec<Pat>> = rows
            .iter()
            .filter(|row| matches!(row[0], Pat::Any))
            .map(|row| row[1..].to_vec())
            .collect();
        let mut witness = self.uncovered(default, rest)?;
        let head = match &all {
            Some(all) => match all.iter().find(|(con, _)| !heads.contains(con)) {
                Some((con, parts)) => self.show_con(ty, *con, &vec!["_".to_string(); parts.len()]),
                None => "_".to_string(),
            },
            None => "_".to_string(),
        };
        witness.insert(0, head);
        Some(witness)
    }

    fn show_con(&self, ty: TyId, con: Con, parts: &[String]) -> String {
        match con {
            Con::Bool(value) => value.to_string(),
            Con::Some => format!("Some({})", parts[0]),
            Con::None => "None".to_string(),
            Con::Tuple => format!("({})", parts.join(", ")),
            Con::Member(member) => format!("_: {}", self.show_ty(member)),
            Con::Other => "_".to_string(),
            Con::Record => match self.program.types.kind(ty) {
                TyKind::Adt { def, .. } => format!("{} {{ .. }}", self.text(self.program.defs[*def].name)),
                _ => "_".to_string(),
            },
            Con::Variant(index) => {
                let TyKind::Adt { def, .. } = self.program.types.kind(ty) else { return "_".to_string() };
                let variant = &self.program.defs[*def].enum_def().expect("an enum").variants[index as usize];
                let name = self.text(variant.name);
                match &variant.payload {
                    Payload::Unit => name,
                    Payload::Tuple(_) => format!("{name}({})", parts.join(", ")),
                    Payload::Struct(_) => format!("{name} {{ .. }}"),
                }
            }
        }
    }
}

/// Splits each row whose first pattern is an or-pattern into one row per alternative.
fn expand(rows: Vec<Vec<Pat>>) -> Vec<Vec<Pat>> {
    let mut out = Vec::new();
    for row in rows {
        match &row[0] {
            Pat::Or(alternatives) => {
                let rest = &row[1..];
                let split = alternatives
                    .iter()
                    .map(|alternative| {
                        let mut next = vec![alternative.clone()];
                        next.extend_from_slice(rest);
                        next
                    })
                    .collect();
                out.extend(expand(split));
            }
            _ => out.push(row),
        }
    }
    out
}

/// The row as it reads once the first value is known to be built by `con`.
fn specialize(row: &[Pat], con: &Con, arity: usize) -> Option<Vec<Pat>> {
    match &row[0] {
        Pat::Any => {
            let mut out = vec![Pat::Any; arity];
            out.extend_from_slice(&row[1..]);
            Some(out)
        }
        Pat::Con(other, parts) if other == con => {
            let mut out = parts.clone();
            out.extend_from_slice(&row[1..]);
            Some(out)
        }
        _ => None,
    }
}
