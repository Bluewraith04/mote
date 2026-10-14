//! Field defaults: each default becomes a hidden function, and a literal that leaves the field out calls it.

use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::span::Span;

/// Where the spans of the added calls start: past every real offset and the generated methods'.
const SYNTHETIC: usize = 1 << 41;

/// Limit on added calls when a default holds a literal of its own type.
const LIMIT: usize = 1_000_000;

/// The hidden function that holds the default of `field` in `owner`.
fn default_fn_name(owner: &str, field: &str) -> String {
    format!("{owner}::default_{field}")
}

/// The functions for the defaults in `fields`, and the literal's omitted-field table entry.
fn collect(owner: &str, fields: &[FieldDecl], added: &mut Vec<Item>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for f in fields {
        let Some(default) = &f.default else { continue };
        let name = default_fn_name(owner, &f.name);
        added.push(Item::Function(FunctionDecl {
            name: name.clone(),
            generic_params: Vec::new(),
            params: Vec::new(),
            return_type: Some(f.ty.clone()),
            body: vec![Stmt::Return { value: Some(default.clone()), span: f.span }],
            is_pub: false,
            span: f.span,
        }));
        out.push((f.name.clone(), name));
    }
    out
}

/// Makes a function of each field default and adds the calls to every literal that leaves such a field out.
pub fn fill_field_defaults(programs: &mut [&mut Program]) {
    let mut table: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut types: HashSet<String> = HashSet::new();
    let mut bare: HashMap<String, usize> = HashMap::new();
    for p in programs.iter() {
        for item in &p.items {
            match item {
                Item::Struct(s) => drop(types.insert(s.name.clone())),
                Item::Class(c) => drop(types.insert(c.name.clone())),
                Item::Enum(e) => {
                    types.insert(e.name.clone());
                    for v in &e.variants {
                        if matches!(v.kind, EnumVariantKind::Struct(_)) {
                            *bare.entry(v.name.clone()).or_default() += 1;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    for p in programs.iter_mut() {
        let mut added = Vec::new();
        for item in &p.items {
            match item {
                Item::Struct(s) => {
                    let defaults = collect(&s.name, &s.fields, &mut added);
                    if !defaults.is_empty() {
                        table.insert(s.name.clone(), defaults);
                    }
                }
                Item::Class(c) => {
                    let defaults = collect(&c.name, &c.fields, &mut added);
                    if !defaults.is_empty() {
                        table.insert(c.name.clone(), defaults);
                    }
                }
                Item::Enum(e) => {
                    for v in &e.variants {
                        let EnumVariantKind::Struct(fields) = &v.kind else { continue };
                        let defaults = collect(&format!("{}::{}", e.name, v.name), fields, &mut added);
                        if defaults.is_empty() {
                            continue;
                        }
                        if bare.get(&v.name) == Some(&1) && !types.contains(&v.name) {
                            table.insert(v.name.clone(), defaults.clone());
                        }
                        table.insert(format!("{}.{}", e.name, v.name), defaults);
                    }
                }
                _ => {}
            }
        }
        p.items.extend(added);
    }
    if table.is_empty() {
        return;
    }
    let mut made = 0usize;
    for p in programs.iter_mut() {
        crate::ast_walk::exprs_mut(&mut p.items, &mut |e| {
            let Expr::StructInit { name, fields, span, .. } = e else { return };
            let Some(defaults) = table.get(name.as_str()) else { return };
            for (field, func) in defaults {
                if made >= LIMIT {
                    return;
                }
                if fields.iter().any(|(n, _)| n == field) {
                    continue;
                }
                let mut fresh = || {
                    made += 1;
                    Span { start: SYNTHETIC + 2 * made, end: SYNTHETIC + 2 * made + 1, ..*span }
                };
                let (callee_span, call_span) = (fresh(), fresh());
                let call = Expr::Call { callee: Box::new(Expr::Ident(func.clone(), callee_span)), args: Vec::new(), names: Vec::new(), span: call_span };
                fields.push((field.clone(), call));
            }
        });
    }
}
