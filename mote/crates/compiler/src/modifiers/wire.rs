//! A `Decl` as the JSON text `std.meta.parse_decl` reads: how a declaration crosses into stage 0.

use super::Decl;
use crate::ast::*;

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn list<T>(items: &[T], f: impl Fn(&T) -> String) -> String {
    format!("[{}]", items.iter().map(f).collect::<Vec<_>>().join(","))
}

fn type_json(kind: &str, name: &str, args: &[&TypeNode]) -> String {
    format!("{{\"kind\":{},\"name\":{},\"args\":{}}}", quote(kind), quote(name), list(args, |t| type_ref(t)))
}

fn type_ref(t: &TypeNode) -> String {
    match t {
        TypeNode::Int(_) => type_json("name", "Int", &[]),
        TypeNode::Float(_) => type_json("name", "Float", &[]),
        TypeNode::Bool(_) => type_json("name", "Bool", &[]),
        TypeNode::Char(_) => type_json("name", "Char", &[]),
        TypeNode::String(_) => type_json("name", "String", &[]),
        TypeNode::Null(_) => type_json("name", "Null", &[]),
        TypeNode::Named(name, _) => type_json("name", name, &[]),
        TypeNode::Nullable(inner, _) => type_json("optional", "", &[inner]),
        TypeNode::Generic(name, args, _) => type_json("generic", name, &args.iter().collect::<Vec<_>>()),
        TypeNode::Tuple(elems, _) => type_json("tuple", "", &elems.iter().collect::<Vec<_>>()),
        TypeNode::Function(params, ret, send, _) => {
            let mut args: Vec<&TypeNode> = params.iter().collect();
            args.push(ret);
            type_json("function", if *send { "Send" } else { "" }, &args)
        }
        TypeNode::Array(elem, _, _) => type_json("array", "", &[elem]),
        TypeNode::SelfType(_) => type_json("self", "", &[]),
        TypeNode::Union(members, _) => type_json("union", "", &members.iter().collect::<Vec<_>>()),
        TypeNode::VarParam(inner, _) => type_json("var", "", &[inner]),
    }
}

fn attr(a: &Attribute) -> String {
    let values = list(&a.values, |(k, v)| format!("[{},{}]", quote(k), quote(v)));
    format!("{{\"name\":{},\"args\":{},\"values\":{}}}", quote(&a.name), list(&a.args, |s| quote(s)), values)
}

fn field(f: &FieldDecl) -> String {
    format!(
        "{{\"name\":{},\"type\":{},\"pub\":{},\"var\":{},\"doc\":{},\"default\":{}}}",
        quote(&f.name),
        type_ref(&f.ty),
        f.is_pub,
        f.is_mutable,
        quote(&f.doc),
        quote(&f.default_text)
    )
}

fn discriminant(e: &Option<Expr>) -> String {
    match e {
        Some(Expr::Int(n, _)) => n.to_string(),
        Some(Expr::Unary { op: UnaryOp::Neg, expr, .. }) => match **expr {
            Expr::Int(n, _) => (-n).to_string(),
            _ => "null".to_string(),
        },
        _ => "null".to_string(),
    }
}

fn variant(v: &EnumVariant) -> String {
    let (shape, types, fields, disc) = match &v.kind {
        EnumVariantKind::Unit { discriminant: d } => ("unit", "[]".to_string(), "[]".to_string(), discriminant(d)),
        EnumVariantKind::Tuple(ts) => ("tuple", list(ts, type_ref), "[]".to_string(), "null".to_string()),
        EnumVariantKind::Struct(fs) => ("struct", "[]".to_string(), list(fs, field), "null".to_string()),
    };
    format!(
        "{{\"name\":{},\"shape\":{},\"types\":{},\"fields\":{},\"discriminant\":{},\"doc\":{},\"attrs\":{}}}",
        quote(&v.name),
        quote(shape),
        types,
        fields,
        disc,
        quote(&v.doc),
        list(&v.attrs, attr)
    )
}

fn method(m: &FunctionDecl) -> String {
    let has_self = m.params.iter().any(|p| matches!(p.kind, Some(ParameterKind::SelfValue { .. })));
    let params: Vec<&Param> = m.params.iter().filter(|p| !matches!(p.kind, Some(ParameterKind::SelfValue { .. }))).collect();
    let param = |p: &&Param| format!("{{\"name\":{},\"type\":{}}}", quote(&p.name), p.ty.as_ref().map_or("null".to_string(), type_ref));
    format!(
        "{{\"name\":{},\"pub\":{},\"self\":{},\"params\":{},\"returns\":{}}}",
        quote(&m.name),
        m.is_pub,
        has_self,
        list(&params, param),
        m.return_type.as_ref().map_or("null".to_string(), type_ref)
    )
}

fn generic(g: &GenericParam) -> String {
    format!("{{\"name\":{},\"bounds\":{}}}", quote(&g.name), list(&g.bounds, type_ref))
}

/// The JSON text of `decl`, the shape `std.meta.parse_decl` reads.
pub fn decl_json(decl: &Decl) -> String {
    let kind = match decl.kind {
        super::DeclKind::Struct => "struct",
        super::DeclKind::Class => "class",
        super::DeclKind::Enum => "enum",
    };
    format!(
        "{{\"kind\":{},\"name\":{},\"pub\":{},\"doc\":{},\"generics\":{},\"traits\":{},\"fields\":{},\"variants\":{},\"methods\":{},\"attrs\":{}}}",
        quote(kind),
        quote(decl.name),
        decl.is_pub,
        quote(decl.doc),
        list(decl.generics, generic),
        list(decl.traits, type_ref),
        list(decl.fields, field),
        list(decl.variants.unwrap_or(&[]), variant),
        list(decl.methods, method),
        list(decl.attrs, attr)
    )
}
