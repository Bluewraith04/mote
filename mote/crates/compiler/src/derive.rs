//! `@name` / `@name(args)` attributes: a closed registry applied at parse time. `@derive` expands in place; `@stable` and `@test` / `@ignore` are only validated here.

mod args;
mod json;

use crate::ast::{Attribute, EnumVariant, FieldDecl, FunctionDecl, Item};
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::span::{Span, DERIVED_BASE};
use json::Target;

fn derived_method_names(trait_name: &str) -> Option<&'static [&'static str]> {
    match trait_name {
        "Display" => Some(&["to_string"]),
        "Debug" => Some(&["debug"]),
        "Json" => Some(&["to_json", "from_json"]),
        "Args" => Some(&["parse", "parse_or_exit", "usage"]),
        _ => None,
    }
}

fn generated_functions(src: &str, span: Span) -> Vec<FunctionDecl> {
    let mut tokens = Lexer::new(src).tokenize().expect("a derived method's source lexes");
    let base = DERIVED_BASE.wrapping_add(span.start << 24);
    for t in &mut tokens {
        t.span = Span { start: base + t.span.start, end: base + t.span.end, line: span.line, col: span.col, source: span.source };
    }
    let items = Parser::new(tokens).parse().expect("a derived method's source parses").items;
    items
        .into_iter()
        .map(|item| {
            let Item::Function(mut f) = item else { unreachable!("derived source is only `fn`s") };
            f.span = span;
            f.is_pub = true;
            f
        })
        .collect()
}

fn check_members(item: &Item, derives_args: bool, item_attrs: &[Attribute]) -> Result<(), (String, Span)> {
    let fields_of = |fields: &[FieldDecl]| fields.iter().map(|f| (f.attrs.clone(), "field")).collect::<Vec<_>>();
    let variants_of = |variants: &[EnumVariant]| {
        let mut out = Vec::new();
        for v in variants {
            out.push((v.attrs.clone(), "variant"));
            if let crate::ast::EnumVariantKind::Struct(fields) = &v.kind {
                out.extend(fields_of(fields));
            }
        }
        out
    };
    let members = match item {
        Item::Struct(s) => fields_of(&s.fields),
        Item::Class(c) => fields_of(&c.fields),
        Item::Enum(e) => variants_of(&e.variants),
        _ => Vec::new(),
    };
    for (attrs, what) in members {
        for a in &attrs {
            if a.name != "arg" {
                let message = if matches!(a.name.as_str(), "derive" | "stable" | "test" | "ignore") {
                    format!("`@{}` cannot go on a {what}", a.name)
                } else {
                    format!("`@{}` is not a known attribute", a.name)
                };
                return Err((message, a.span));
            }
            if !derives_args {
                return Err((format!("`@arg` on a {what} needs `@derive(Args)` on its type"), a.span));
            }
        }
    }
    if !derives_args
        && let Some(a) = item_attrs.iter().find(|a| a.name == "arg") {
            return Err(("`@arg` needs `@derive(Args)` on the type".to_string(), a.span));
        }
    Ok(())
}

/// Applies every attribute in `attrs` to the just-parsed `item`: expands `@derive` and validates the placement of `@stable`, `@test` and `@ignore`.
pub(crate) fn apply_attributes(attrs: Vec<Attribute>, item: Item) -> Result<Item, (String, Span)> {
    check_members(&item, uses(&attrs, "Args"), &attrs)?;
    if attrs.is_empty() {
        return Ok(item);
    }
    for a in &attrs {
        if !matches!(a.name.as_str(), "derive" | "stable" | "test" | "ignore" | "arg") {
            return Err((format!("`@{}` is not a known attribute", a.name), a.span));
        }
        if a.name != "arg" && !a.values.is_empty() {
            return Err((format!("`@{}` takes no `key = \"value\"`", a.name), a.span));
        }
        if a.name == "stable" && !a.args.is_empty() {
            return Err(("`@stable` takes no arguments".to_string(), a.span));
        }
        if (a.name == "test" || a.name == "ignore") && !a.args.is_empty() {
            return Err((format!("`@{}` takes no arguments", a.name), a.span));
        }
    }
    let derives: Vec<&Attribute> = attrs.iter().filter(|a| a.name == "derive").collect();
    let item = if derives.is_empty() {
        item
    } else {
        match item {
            Item::Struct(mut s) => {
                let target = Target { name: &s.name, fields: &s.fields, variants: None };
                for a in &derives {
                    apply_derive(a, &target, &attrs, !s.generic_params.is_empty(), &mut s.methods)?;
                }
                Item::Struct(s)
            }
            Item::Class(mut c) => {
                let target = Target { name: &c.name, fields: &c.fields, variants: None };
                for a in &derives {
                    apply_derive(a, &target, &attrs, !c.generic_params.is_empty(), &mut c.methods)?;
                }
                Item::Class(c)
            }
            Item::Enum(mut e) => {
                let target = Target { name: &e.name, fields: &[], variants: Some(&e.variants) };
                for a in &derives {
                    apply_derive(a, &target, &attrs, !e.generic_params.is_empty(), &mut e.methods)?;
                }
                Item::Enum(e)
            }
            _ => return Err(("`@derive` can only be used on a `struct`, `class` or `enum`".to_string(), derives[0].span)),
        }
    };
    if let Some(a) = attrs.iter().find(|a| a.name == "stable") {
        match crate::stable::describe(&item) {
            Some((_, is_pub, _, _)) if !is_pub => {
                return Err(("`@stable` is only legal on a `pub` item".to_string(), a.span));
            }
            Some(_) => {}
            None => {
                return Err((
                    "`@stable` can only be used on a `fn`, `struct`, `class`, `enum` or `trait`".to_string(),
                    a.span,
                ));
            }
        }
    }
    if let Some(a) = attrs.iter().find(|a| a.name == "test" || a.name == "ignore")
        && !matches!(item, Item::Function(_)) {
            return Err((format!("`@{}` can only be used on a `fn` or a `test` block", a.name), a.span));
        }
    Ok(item)
}

fn apply_derive(a: &Attribute, target: &Target, item_attrs: &[Attribute], generic: bool, methods: &mut Vec<FunctionDecl>) -> Result<(), (String, Span)> {
    let type_name = target.name;
    for trait_name in &a.args {
        let Some(method_names) = derived_method_names(trait_name) else {
            return Err((format!("`@derive({trait_name})`: `{trait_name}` cannot be derived"), a.span));
        };
        for method_name in method_names {
            if methods.iter().any(|m| m.name == *method_name) {
                return Err((format!("`{type_name}` already has a method named `{method_name}`, so `@derive({trait_name})` cannot add one"), a.span));
            }
        }
        if trait_name == "Json" {
            if generic {
                return Err(("`@derive(Json)` does not support a generic type".to_string(), a.span));
            }
            methods.extend(json::generate(target, a.span).map_err(|message| (message, a.span))?);
        } else if trait_name == "Args" {
            if generic {
                return Err(("`@derive(Args)` does not support a generic type".to_string(), a.span));
            }
            methods.extend(args::generate(target, item_attrs, a.span)?);
        } else {
            methods.push(synthesize(trait_name, type_name, target.fields, target.variants.is_some(), a.span));
        }
    }
    Ok(())
}

/// Whether `attrs` ask for `@derive(name)`.
pub fn uses(attrs: &[Attribute], name: &str) -> bool {
    attrs.iter().any(|a| a.name == "derive" && a.args.iter().any(|t| t == name))
}

fn synthesize(trait_name: &str, type_name: &str, fields: &[FieldDecl], is_enum: bool, span: Span) -> FunctionDecl {
    let src = match trait_name {
        "Display" if is_enum => "fn to_string(self) -> String { return __debug_raw(self) }".to_string(),
        "Display" => {
            let body = if fields.is_empty() {
                format!("\"{type_name}\"")
            } else {
                let parts: Vec<String> = fields.iter().map(|f| format!("{}: ${{self.{}}}", f.name, f.name)).collect();
                format!("\"{type_name}({})\"", parts.join(", "))
            };
            format!("fn to_string(self) -> String {{ return {body} }}")
        }
        _ => "fn debug(self) -> String { return __debug_raw(self) }".to_string(),
    };
    let tokens = Lexer::new(&src).tokenize().expect("a derived method's source lexes");
    let mut items = Parser::new(tokens).parse().expect("a derived method's source parses").items;
    let Item::Function(mut f) = items.remove(0) else { unreachable!("derived source is one `fn`") };
    f.span = span;
    f.is_pub = true;
    f
}
