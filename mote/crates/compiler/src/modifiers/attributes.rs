//! The attribute registry: one row per attribute, and the checks every attribute gets at parse time.

use crate::ast::{Attribute, FunctionDecl, Item, TypeNode};
use crate::span::Span;

/// Where an attribute may sit.
#[derive(Clone, Copy, PartialEq)]
enum Place {
    Function,
    Type,
    Trait,
}

/// What reads an attribute.
#[derive(Clone, Copy)]
enum Reader {
    /// A tool after checking (`mote test`, the stable-surface check).
    Marker,
    /// The modifier pass expands the declaration.
    Modifier,
    /// Data on a declaration or member that a modifier lists in `reads`.
    Member,
}

struct Spec {
    name: &'static str,
    places: &'static [Place],
    /// Takes bare arguments, as in `@derive(Display)`.
    args: bool,
    /// Takes `key = "value"` pairs.
    values: bool,
    /// The error when it sits somewhere else.
    placement: &'static str,
    reader: Reader,
}

const REGISTRY: &[Spec] = &[
    Spec {
        name: "stable",
        places: &[Place::Function, Place::Type, Place::Trait],
        args: false,
        values: false,
        placement: "`@stable` can only be used on a `fn`, `struct`, `class`, `enum` or `trait`",
        reader: Reader::Marker,
    },
    Spec {
        name: "test",
        places: &[Place::Function],
        args: false,
        values: false,
        placement: "`@test` can only be used on a `fn` or a `test` block",
        reader: Reader::Marker,
    },
    Spec {
        name: "ignore",
        places: &[Place::Function],
        args: false,
        values: false,
        placement: "`@ignore` can only be used on a `fn` or a `test` block",
        reader: Reader::Marker,
    },
    Spec {
        name: "modifier",
        places: &[Place::Function],
        args: false,
        values: true,
        placement: "`@modifier` can only be used on a `fn`",
        reader: Reader::Marker,
    },
    Spec {
        name: "generate",
        places: &[Place::Trait],
        args: true,
        values: false,
        placement: "`@generate` can only be used on a `trait`",
        reader: Reader::Modifier,
    },
    Spec {
        name: "arg",
        places: &[Place::Type],
        args: true,
        values: true,
        placement: "",
        reader: Reader::Member,
    },
];

fn lookup(name: &str) -> Option<&'static Spec> {
    REGISTRY.iter().find(|s| s.name == name)
}

/// Whether `name` is data on a declaration or member that only a modifier listing it in `reads` may use.
pub(crate) fn is_member_data(name: &str) -> bool {
    lookup(name).is_some_and(|s| matches!(s.reader, Reader::Member))
}

/// The attribute names `@modifier(reads = "arg, …")` lists.
pub(crate) fn reads_of(attrs: &[Attribute]) -> Vec<String> {
    attrs
        .iter()
        .filter(|a| a.name == "modifier")
        .flat_map(|a| a.values.iter().filter(|(k, _)| k == "reads").flat_map(|(_, v)| v.split(',').map(|s| s.trim().to_string())))
        .filter(|s| !s.is_empty())
        .collect()
}

fn place_of(item: &Item) -> Option<Place> {
    match item {
        Item::Function(_) => Some(Place::Function),
        Item::Struct(_) | Item::Class(_) | Item::Enum(_) => Some(Place::Type),
        Item::Trait(_) => Some(Place::Trait),
        _ => None,
    }
}

/// Every attribute on a variant.
fn member_attrs(item: &Item) -> Vec<(&[Attribute], &'static str)> {
    match item {
        Item::Enum(e) => e.variants.iter().map(|v| (v.attrs.as_slice(), "variant")).collect(),
        _ => Vec::new(),
    }
}

/// Checks the attributes written before `item` and on its members: known, well-formed and well placed.
pub(crate) fn check(attrs: &[Attribute], item: &Item) -> Result<(), (String, Span)> {
    if let Some(a) = attrs.iter().find(|a| a.name == "derive") {
        return Err(("there is no `@derive`: list the trait, `struct P: (Display)`".to_string(), a.span));
    }
    for (member_attrs, what) in member_attrs(item) {
        for a in member_attrs {
            match lookup(&a.name) {
                Some(spec) if matches!(spec.reader, Reader::Member) => {}
                Some(_) => return Err((format!("`@{}` cannot go on a {what}", a.name), a.span)),
                None => return Err((format!("`@{}` is not a known attribute", a.name), a.span)),
            }
        }
    }
    for a in attrs {
        let Some(spec) = lookup(&a.name) else {
            return Err((format!("`@{}` is not a known attribute", a.name), a.span));
        };
        if !spec.values && !a.values.is_empty() {
            return Err((format!("`@{}` takes no `key = \"value\"`", a.name), a.span));
        }
        if !spec.args && !a.args.is_empty() {
            return Err((format!("`@{}` takes no arguments", a.name), a.span));
        }
        if a.name != "generate" && a.args.iter().any(|arg| arg.contains('.')) {
            return Err((format!("`@{}` takes plain names, not paths", a.name), a.span));
        }
        if a.name == "generate" && a.args.len() != 1 {
            return Err(("`@generate` takes the path of one modifier function".to_string(), a.span));
        }
        if a.name == "modifier" {
            if let Some((key, _)) = a.values.iter().find(|(k, _)| k != "reads") {
                return Err((format!("`@modifier` takes `reads = \"…\"`, not `{key}`"), a.span));
            }
            if let Some(name) = reads_of(std::slice::from_ref(a)).iter().find(|n| !is_member_data(n)) {
                return Err((format!("`reads` names `{name}`, which is not an attribute a modifier can read"), a.span));
            }
        }
    }
    let place = place_of(item);
    for a in attrs {
        let spec = lookup(&a.name).expect("checked above");
        if matches!(spec.reader, Reader::Member) {
            continue;
        }
        if !place.is_some_and(|p| spec.places.contains(&p)) {
            return Err((spec.placement.to_string(), a.span));
        }
    }
    if let Some(a) = attrs.iter().find(|a| a.name == "stable")
        && let Some((_, is_pub, _, _)) = crate::stable::describe(item)
        && !is_pub {
            return Err(("`@stable` is only legal on a `pub` item".to_string(), a.span));
        }
    Ok(())
}

/// Whether `t` is a name or a path ending in `name` (`Decl`, `meta.Decl`).
fn names(t: &TypeNode, name: &str) -> bool {
    matches!(t, TypeNode::Named(n, _) if n.rsplit('.').next() == Some(name))
}

/// A `@modifier` is `pub fn(Decl) -> Result<Additions, Diagnostic>`.
pub(crate) fn check_modifier_signature(f: &FunctionDecl) -> Result<(), (String, Span)> {
    if !f.is_pub {
        return Err(("`@modifier` needs a `pub fn`".to_string(), f.span));
    }
    let shape = "`@modifier` takes a `Decl` and returns `Result<Additions, Diagnostic>`";
    let [param] = f.params.as_slice() else { return Err((shape.to_string(), f.span)) };
    let result_ok = matches!(&f.return_type, Some(TypeNode::Generic(r, args, _))
        if r.rsplit('.').next() == Some("Result") && matches!(args.as_slice(), [a, d] if names(a, "Additions") && names(d, "Diagnostic")));
    if !param.ty.as_ref().is_some_and(|t| names(t, "Decl")) || !result_ok {
        return Err((shape.to_string(), f.span));
    }
    Ok(())
}

/// Keeps the attributes on a type or trait declaration for the modifier pass; any other item drops them.
pub(crate) fn attach(attrs: Vec<Attribute>, item: Item) -> Item {
    match item {
        Item::Struct(mut s) => {
            s.attrs = attrs;
            Item::Struct(s)
        }
        Item::Class(mut c) => {
            c.attrs = attrs;
            Item::Class(c)
        }
        Item::Enum(mut e) => {
            e.attrs = attrs;
            Item::Enum(e)
        }
        Item::Trait(mut t) => {
            t.attrs = attrs;
            Item::Trait(t)
        }
        other => other,
    }
}
