//! `@derive(Json)`: `to_json` and static `from_json`, written as source and parsed like the other derives.

use crate::ast::{EnumVariant, EnumVariantKind, FieldDecl, FunctionDecl, TypeNode};
use crate::span::Span;
use crate::stable::render_type;

/// What the derive reads from the declaration it sits on.
pub struct Target<'a> {
    pub name: &'a str,
    pub fields: &'a [FieldDecl],
    /// `Some` for an enum.
    pub variants: Option<&'a [EnumVariant]>,
}

/// The two methods, and the private helpers a union or tuple field needs.
pub(crate) fn generate(target: &Target, span: Span) -> Result<Vec<FunctionDecl>, String> {
    let mut g = Gen { owner: target.name, helpers: Vec::new(), ids: 0 };
    let (to, from) = match target.variants {
        Some(variants) => (g.enum_to(variants)?, g.enum_from(variants)?),
        None => (g.struct_to(target.fields)?, g.struct_from(target.fields)?),
    };
    let src = format!("{to}\n{from}\n{}", g.helpers.join("\n"));
    let mut fns = super::generated_functions(&src, span);
    for f in &mut fns {
        f.is_pub = !f.name.starts_with("__json_");
    }
    Ok(fns)
}

struct Gen<'a> {
    owner: &'a str,
    helpers: Vec<String>,
    ids: usize,
}

fn is_json(name: &str) -> bool {
    name == "Json" || name.ends_with(".Json")
}

fn is_string(t: &TypeNode) -> bool {
    matches!(t, TypeNode::String(_))
}

fn describe(t: &TypeNode) -> String {
    match t {
        TypeNode::Int(_) => "an int".into(),
        TypeNode::Float(_) => "a float".into(),
        TypeNode::Bool(_) => "a bool".into(),
        TypeNode::String(_) => "a string".into(),
        TypeNode::Generic(n, _, _) if n == "Map" => "an object".into(),
        TypeNode::Generic(..) | TypeNode::Tuple(..) => "an array".into(),
        TypeNode::Nullable(inner, _) => format!("{} or null", describe(inner)),
        other => render_type(other),
    }
}

fn list_words(words: &[String]) -> String {
    match words.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        Some((last, _)) => last.clone(),
        None => String::new(),
    }
}

impl Gen<'_> {
    fn id(&mut self) -> usize {
        self.ids += 1;
        self.ids
    }

    fn enc(&mut self, e: &str, ty: &TypeNode) -> Result<String, String> {
        Ok(match ty {
            TypeNode::Int(_) => format!("__json.Json.Int({e})"),
            TypeNode::Float(_) => format!("__json.Json.Float({e})"),
            TypeNode::Bool(_) => format!("__json.Json.Bool({e})"),
            TypeNode::String(_) => format!("__json.Json.Str({e})"),
            TypeNode::Named(n, _) if is_json(n) => e.to_string(),
            TypeNode::Named(..) | TypeNode::SelfType(_) => format!("{e}.to_json()"),
            TypeNode::Nullable(inner, _) => format!("__json.__to_nullable({e}, {})", self.enc_lambda(inner)?),
            TypeNode::Generic(n, args, _) if n == "List" && args.len() == 1 => format!("__json.__to_list({e}, {})", self.enc_lambda(&args[0])?),
            TypeNode::Generic(n, args, _) if n == "Set" && args.len() == 1 => format!("__json.__to_list({e}.items(), {})", self.enc_lambda(&args[0])?),
            TypeNode::Generic(n, args, _) if n == "Map" && args.len() == 2 && is_string(&args[0]) => format!("__json.__to_map({e}, {})", self.enc_lambda(&args[1])?),
            TypeNode::Tuple(elems, _) => {
                let id = self.id();
                let items: Result<Vec<String>, String> = elems.iter().enumerate().map(|(i, t)| self.enc(&format!("__t.{i}"), t)).collect();
                self.helpers.push(format!("fn __json_enc{id}(__t: {}) -> __json.Json {{ return __json.Json.Array([{}]) }}", render_type(ty), items?.join(", ")));
                format!("{}.__json_enc{id}({e})", self.owner)
            }
            TypeNode::Union(members, _) => {
                let id = self.id();
                let mut arms = String::new();
                for (i, m) in members.iter().enumerate() {
                    arms.push_str(&format!("__v{i}: {} => {{ return {} }}\n", render_type(m), self.enc(&format!("__v{i}"), m)?));
                }
                self.helpers.push(format!("fn __json_enc{id}(__u: {}) -> __json.Json {{ match __u {{\n{arms}}} }}", render_type(ty)));
                format!("{}.__json_enc{id}({e})", self.owner)
            }
            _ => return Err(render_type(ty)),
        })
    }

    fn enc_lambda(&mut self, ty: &TypeNode) -> Result<String, String> {
        let x = format!("__x{}", self.id());
        Ok(format!("|{x}| {}", self.enc(&x, ty)?))
    }

    fn dec(&mut self, e: &str, ty: &TypeNode) -> Result<String, String> {
        Ok(match ty {
            TypeNode::Int(_) => format!("__json.__int({e})"),
            TypeNode::Float(_) => format!("__json.__float({e})"),
            TypeNode::Bool(_) => format!("__json.__bool({e})"),
            TypeNode::String(_) => format!("__json.__str({e})"),
            TypeNode::Named(n, _) if is_json(n) => format!("Ok({e})"),
            TypeNode::Named(n, _) => format!("{n}.from_json({e})"),
            TypeNode::SelfType(_) => format!("{}.from_json({e})", self.owner),
            TypeNode::Nullable(inner, _) => format!("__json.__nullable({e}, {})", self.dec_lambda(inner)?),
            TypeNode::Generic(n, args, _) if n == "List" && args.len() == 1 => format!("__json.__list({e}, {})", self.dec_lambda(&args[0])?),
            TypeNode::Generic(n, args, _) if n == "Set" && args.len() == 1 => format!("__json.__set({e}, {})", self.dec_lambda(&args[0])?),
            TypeNode::Generic(n, args, _) if n == "Map" && args.len() == 2 && is_string(&args[0]) => format!("__json.__map({e}, {})", self.dec_lambda(&args[1])?),
            TypeNode::Tuple(elems, _) => {
                let id = self.id();
                let mut body = format!("let __xs = __json.__elements(__j, {})?\n", elems.len());
                let mut names = Vec::new();
                for (i, t) in elems.iter().enumerate() {
                    body.push_str(&format!("let __e{i} = __json.__in(\"[{i}]\", {})?\n", self.dec(&format!("__xs.get({i})"), t)?));
                    names.push(format!("__e{i}"));
                }
                self.helpers.push(format!("fn __json_dec{id}(__j: __json.Json) -> Result<{}, Error> {{\n{body}return Ok(({}))\n}}", render_type(ty), names.join(", ")));
                format!("{}.__json_dec{id}({e})", self.owner)
            }
            TypeNode::Union(members, _) => {
                let id = self.id();
                let mut body = String::new();
                for m in members {
                    body.push_str(&format!("match {} {{ Ok(__v) => {{ return Ok(__v) }} Err(__e) => {{}} }}\n", self.dec("__j", m)?));
                }
                let wanted: Vec<String> = members.iter().map(describe).collect();
                body.push_str(&format!("return Err(__json.__wrong(\"{}\", __j))\n", list_words(&wanted)));
                self.helpers.push(format!("fn __json_dec{id}(__j: __json.Json) -> Result<{}, Error> {{\n{body}}}", render_type(ty)));
                format!("{}.__json_dec{id}({e})", self.owner)
            }
            _ => return Err(render_type(ty)),
        })
    }

    fn dec_lambda(&mut self, ty: &TypeNode) -> Result<String, String> {
        let x = format!("__x{}", self.id());
        Ok(format!("|{x}| {}", self.dec(&x, ty)?))
    }

    fn no_form(what: &str, ty: String) -> String {
        format!("`@derive(Json)` has no JSON form for {what} of type `{ty}`")
    }

    fn members(&mut self, fields: &[FieldDecl], value: impl Fn(&str) -> String) -> Result<String, String> {
        let mut out = Vec::new();
        for f in fields {
            let e = self.enc(&value(&f.name), &f.ty).map_err(|ty| Self::no_form(&format!("field `{}`", f.name), ty))?;
            out.push(format!("__json.member(\"{}\", {e})", f.name));
        }
        Ok(format!("__json.Json.Object([{}])", out.join(", ")))
    }

    fn read_fields(&mut self, fields: &[FieldDecl], from: &str, path: &str) -> Result<(String, String), String> {
        let wrap = |inner: String| if path.is_empty() { inner } else { format!("__json.__in(\"{path}\", {inner})") };
        let (mut stmts, mut inits) = (String::new(), Vec::new());
        for f in fields {
            let fetch = if matches!(f.ty, TypeNode::Nullable(..)) { "__field_or_null" } else { "__field" };
            let dec = self.dec(&format!("__v_{}", f.name), &f.ty).map_err(|ty| Self::no_form(&format!("field `{}`", f.name), ty))?;
            stmts.push_str(&format!("let __v_{} = {}?\n", f.name, wrap(format!("__json.{fetch}({from}, \"{}\")", f.name))));
            stmts.push_str(&format!("let __f_{} = {}?\n", f.name, wrap(format!("__json.__in(\"{}\", {dec})", f.name))));
            inits.push(format!("{}: __f_{}", f.name, f.name));
        }
        Ok((stmts, inits.join(", ")))
    }

    fn struct_to(&mut self, fields: &[FieldDecl]) -> Result<String, String> {
        let body = self.members(fields, |n| format!("self.{n}"))?;
        Ok(format!("fn to_json(self) -> __json.Json {{ return {body} }}"))
    }

    fn struct_from(&mut self, fields: &[FieldDecl]) -> Result<String, String> {
        let (stmts, inits) = self.read_fields(fields, "__j", "")?;
        let owner = self.owner;
        Ok(format!("fn from_json(__j: __json.Json) -> Result<{owner}, Error> {{\n{stmts}return Ok({owner} {{ {inits} }})\n}}"))
    }

    fn enum_to(&mut self, variants: &[EnumVariant]) -> Result<String, String> {
        let owner = self.owner;
        let mut arms = String::new();
        for v in variants {
            let name = &v.name;
            let tagged = |payload: String| format!("__json.Json.Object([__json.member(\"{name}\", {payload})])");
            match &v.kind {
                EnumVariantKind::Unit { .. } => arms.push_str(&format!("{owner}.{name} => {{ return __json.Json.Str(\"{name}\") }}\n")),
                EnumVariantKind::Tuple(tys) => {
                    let binds: Vec<String> = (0..tys.len()).map(|i| format!("__p{i}")).collect();
                    let mut parts = Vec::new();
                    for (i, t) in tys.iter().enumerate() {
                        parts.push(self.enc(&format!("__p{i}"), t).map_err(|ty| Self::no_form(&format!("payload {} of `{name}`", i + 1), ty))?);
                    }
                    let payload = if parts.len() == 1 { parts.remove(0) } else { format!("__json.Json.Array([{}])", parts.join(", ")) };
                    arms.push_str(&format!("{owner}.{name}({}) => {{ return {} }}\n", binds.join(", "), tagged(payload)));
                }
                EnumVariantKind::Struct(fields) => {
                    let binds: Vec<String> = fields.iter().map(|f| format!("{}: __b_{}", f.name, f.name)).collect();
                    let payload = self.members(fields, |n| format!("__b_{n}"))?;
                    arms.push_str(&format!("{owner}.{name} {{ {} }} => {{ return {} }}\n", binds.join(", "), tagged(payload)));
                }
            }
        }
        Ok(format!("fn to_json(self) -> __json.Json {{ match self {{\n{arms}}} }}"))
    }

    fn enum_from(&mut self, variants: &[EnumVariant]) -> Result<String, String> {
        let owner = self.owner;
        let units: Vec<&EnumVariant> = variants.iter().filter(|v| matches!(v.kind, EnumVariantKind::Unit { .. })).collect();
        let mut body = String::new();
        if !units.is_empty() {
            body.push_str("match __j {\n__json.Json.Str(__s) => {\n");
            for v in &units {
                body.push_str(&format!("if __s == \"{0}\" {{ return Ok({owner}.{0}) }}\n", v.name));
            }
            body.push_str(&format!("return Err(__json.__unknown_variant(__s, \"{owner}\"))\n}}\n_ => {{}}\n}}\n"));
        }
        body.push_str("let __m = __json.__variant(__j)?\n");
        for v in variants {
            let name = &v.name;
            match &v.kind {
                EnumVariantKind::Unit { .. } => {}
                EnumVariantKind::Tuple(tys) if tys.len() == 1 => {
                    let dec = self.dec("__m.value", &tys[0]).map_err(|ty| Self::no_form(&format!("payload 1 of `{name}`"), ty))?;
                    body.push_str(&format!("if __m.key == \"{name}\" {{ return Ok({owner}.{name}(__json.__in(\"{name}\", {dec})?)) }}\n"));
                }
                EnumVariantKind::Tuple(tys) => {
                    let mut block = format!("let __xs = __json.__in(\"{name}\", __json.__elements(__m.value, {}))?\n", tys.len());
                    let mut names = Vec::new();
                    for (i, t) in tys.iter().enumerate() {
                        let dec = self.dec(&format!("__xs.get({i})"), t).map_err(|ty| Self::no_form(&format!("payload {} of `{name}`", i + 1), ty))?;
                        block.push_str(&format!("let __e{i} = __json.__in(\"{name}\", __json.__in(\"[{i}]\", {dec}))?\n"));
                        names.push(format!("__e{i}"));
                    }
                    body.push_str(&format!("if __m.key == \"{name}\" {{\n{block}return Ok({owner}.{name}({}))\n}}\n", names.join(", ")));
                }
                EnumVariantKind::Struct(fields) => {
                    let (stmts, inits) = self.read_fields(fields, "__m.value", name)?;
                    body.push_str(&format!("if __m.key == \"{name}\" {{\n{stmts}return Ok({owner}.{name} {{ {inits} }})\n}}\n"));
                }
            }
        }
        body.push_str(&format!("return Err(__json.__unknown_variant(__m.key, \"{owner}\"))"));
        Ok(format!("fn from_json(__j: __json.Json) -> Result<{owner}, Error> {{\n{body}\n}}"))
    }
}
