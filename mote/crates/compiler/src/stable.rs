//! `@stable`: span-free signature rendering for the std-surface snapshot.

use crate::ast::{
    ClassDecl, EnumDecl, EnumVariantKind, FunctionDecl, Item, Param, ParameterKind, StructDecl, TraitDecl, TypeNode,
};

pub(crate) fn render_type(t: &TypeNode) -> String {
    match t {
        TypeNode::Int(_) => "Int".to_string(),
        TypeNode::Float(_) => "Float".to_string(),
        TypeNode::Bool(_) => "Bool".to_string(),
        TypeNode::Char(_) => "Char".to_string(),
        TypeNode::String(_) => "String".to_string(),
        TypeNode::Null(_) => "Null".to_string(),
        TypeNode::Named(name, _) => name.clone(),
        TypeNode::Nullable(inner, _) if matches!(**inner, TypeNode::Union(..)) => format!("({})?", render_type(inner)),
        TypeNode::Nullable(inner, _) => format!("{}?", render_type(inner)),
        TypeNode::Generic(name, args, _) => {
            let inner: Vec<String> = args.iter().map(render_type).collect();
            format!("{}<{}>", name, inner.join(", "))
        }
        TypeNode::Tuple(elems, _) => {
            let inner: Vec<String> = elems.iter().map(render_type).collect();
            format!("({})", inner.join(", "))
        }
        TypeNode::Function(params, ret, send, _) => {
            let inner: Vec<String> = params.iter().map(render_type).collect();
            format!("{}({}) -> {}", if *send { "Send " } else { "" }, inner.join(", "), render_type(ret))
        }
        TypeNode::Array(elem, _, _) => format!("[{}]", render_type(elem)),
        TypeNode::SelfType(_) => "Self".to_string(),
        TypeNode::VarParam(inner, _) => format!("var {}", render_type(inner)),
        TypeNode::Union(members, _) => members
            .iter()
            .map(|m| if matches!(m, TypeNode::Function(..)) { format!("({})", render_type(m)) } else { render_type(m) })
            .collect::<Vec<_>>()
            .join(" | "),
    }
}

fn render_param(p: &Param) -> String {
    if let Some(ParameterKind::SelfValue { is_var }) = &p.kind {
        return if *is_var { "var self" } else { "self" }.to_string();
    }
    let ty = p.ty.as_ref().map(render_type).unwrap_or_else(|| "Any".to_string());
    format!("{}{}: {}", if p.is_mut { "var " } else { "" }, p.name, ty)
}

fn render_generics(names: &[String]) -> String {
    if names.is_empty() { String::new() } else { format!("<{}>", names.join(", ")) }
}

fn render_fn_like(name: &str, generics: &[String], params: &[Param], ret: Option<&TypeNode>) -> String {
    let params: Vec<String> = params.iter().map(render_param).collect();
    let ret = ret.map(render_type).unwrap_or_else(|| "Null".to_string());
    format!("fn {}{}({}) -> {}", name, render_generics(generics), params.join(", "), ret)
}

fn render_function(f: &FunctionDecl) -> String {
    let generics: Vec<String> = f.generic_params.iter().map(|g| g.name.clone()).collect();
    render_fn_like(&f.name, &generics, &f.params, f.return_type.as_ref())
}

fn render_traits(traits: &[TypeNode]) -> String {
    if traits.is_empty() {
        return String::new();
    }
    let names: Vec<String> = traits.iter().map(render_type).collect();
    format!(": ({})", names.join(", "))
}

fn render_struct_like(kind: &str, name: &str, generics: &[String], traits: &[TypeNode], fields: &[crate::ast::FieldDecl]) -> String {
    let fields: Vec<String> = fields
        .iter()
        .map(|f| format!("{}{}: {}", if f.is_mutable { "var " } else { "" }, f.name, render_type(&f.ty)))
        .collect();
    format!("{} {}{}{} {{ {} }}", kind, name, render_generics(generics), render_traits(traits), fields.join(", "))
}

fn render_struct(s: &StructDecl) -> String {
    let generics: Vec<String> = s.generic_params.iter().map(|g| g.name.clone()).collect();
    render_struct_like("struct", &s.name, &generics, &s.traits, &s.fields)
}

fn render_class(c: &ClassDecl) -> String {
    let generics: Vec<String> = c.generic_params.iter().map(|g| g.name.clone()).collect();
    render_struct_like("class", &c.name, &generics, &c.traits, &c.fields)
}

fn render_enum(e: &EnumDecl) -> String {
    let generics: Vec<String> = e.generic_params.iter().map(|g| g.name.clone()).collect();
    let variants: Vec<String> = e
        .variants
        .iter()
        .map(|v| match &v.kind {
            EnumVariantKind::Unit { .. } => v.name.clone(),
            EnumVariantKind::Tuple(tys) => {
                let inner: Vec<String> = tys.iter().map(render_type).collect();
                format!("{}({})", v.name, inner.join(", "))
            }
            EnumVariantKind::Struct(fields) => {
                let inner: Vec<String> = fields.iter().map(|f| format!("{}: {}", f.name, render_type(&f.ty))).collect();
                format!("{} {{ {} }}", v.name, inner.join(", "))
            }
        })
        .collect();
    format!("enum {}{}{} {{ {} }}", e.name, render_generics(&generics), render_traits(&e.traits), variants.join(" "))
}

fn render_trait(t: &TraitDecl) -> String {
    let generics: Vec<String> = t.generic_params.iter().map(|g| g.name.clone()).collect();
    let members: Vec<String> = t
        .members
        .iter()
        .map(|m| {
            let member_generics: Vec<String> = m.generic_params.iter().map(|g| g.name.clone()).collect();
            render_fn_like(&m.name, &member_generics, &m.params, m.return_type.as_ref())
        })
        .collect();
    format!("trait {}{} {{ {} }}", t.name, render_generics(&generics), members.join("; "))
}

/// A module's public surface, one declaration per line, no bodies or spans.
pub fn interface(program: &crate::ast::Program) -> String {
    use crate::ast::Stmt;
    let methods = |ms: &[FunctionDecl], all: bool| -> String {
        ms.iter().filter(|m| all || m.is_pub).map(|m| format!("\n    {}", render_function(m))).collect()
    };
    let mut out = String::new();
    for item in &program.items {
        let line = match item {
            Item::Function(f) if f.is_pub => render_function(f),
            Item::Struct(s) if s.is_pub => format!("{}{}", render_struct(s), methods(&s.methods, false)),
            Item::Class(c) if c.is_pub => format!("{}{}", render_class(c), methods(&c.methods, false)),
            Item::Enum(e) if e.is_pub => format!("{}{}", render_enum(e), methods(&e.methods, false)),
            Item::Trait(t) if t.is_pub => render_trait(t),
            Item::TypeAlias(a) if a.is_pub => {
                let generics: Vec<String> = a.generic_params.iter().map(|g| g.name.clone()).collect();
                format!("type {}{} = {}", a.name, render_generics(&generics), render_type(&a.target))
            }
            Item::Import(i) if i.is_pub => {
                let names: Vec<String> = i.symbols.iter().map(|s| match &s.alias {
                    Some(a) => format!("{} as {a}", s.name),
                    None => s.name.clone(),
                }).collect();
                if i.glob {
                    format!("pub import {{ * }} from {}", i.path.to_dotted_string())
                } else if names.is_empty() {
                    format!("pub import {}", i.path.to_dotted_string())
                } else {
                    format!("pub import {{ {} }} from {}", names.join(", "), i.path.to_dotted_string())
                }
            }
            Item::TopLevelStmt(Stmt::Let { name, ty, is_pub: true, .. }) => {
                format!("let {name}: {}", ty.as_ref().map(render_type).unwrap_or_else(|| "Any".to_string()))
            }
            Item::TopLevelStmt(Stmt::Var { name, ty, is_pub: true, .. }) => {
                format!("var {name}: {}", ty.as_ref().map(render_type).unwrap_or_else(|| "Any".to_string()))
            }
            _ => continue,
        };
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// `(kind, is_pub, name, signature)` for an item that can carry `@stable`, else `None`.
pub fn describe(item: &Item) -> Option<(&'static str, bool, String, String)> {
    match item {
        Item::Function(f) => Some(("fn", f.is_pub, f.name.clone(), render_function(f))),
        Item::Struct(s) => Some(("struct", s.is_pub, s.name.clone(), render_struct(s))),
        Item::Class(c) => Some(("class", c.is_pub, c.name.clone(), render_class(c))),
        Item::Enum(e) => Some(("enum", e.is_pub, e.name.clone(), render_enum(e))),
        Item::Trait(t) => Some(("trait", t.is_pub, t.name.clone(), render_trait(t))),
        _ => None,
    }
}
