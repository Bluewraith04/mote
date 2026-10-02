//! `@derive(Args)`: `parse`, `parse_or_exit` and `usage`, written as source and parsed like the other derives.

use super::json::Target;
use crate::ast::{Attribute, EnumVariant, EnumVariantKind, FieldDecl, FunctionDecl, TypeNode};
use crate::span::Span;
use crate::stable::render_type;

type Failure = (String, Span);

#[derive(Clone, Copy)]
enum Scalar {
    Int,
    Float,
    Str,
}

enum Shape {
    Flag,
    Value(Scalar, bool),
    Many(Scalar),
    Command(String, bool),
}

struct Member {
    name: String,
    long: String,
    short: Option<char>,
    help: String,
    default: Option<String>,
    positional: bool,
    shape: Shape,
}

#[derive(Default)]
struct Meta {
    help: String,
    short: Option<char>,
    default: Option<String>,
    positional: bool,
}

/// The type's methods; `item_attrs` carries the type's own `@arg(help = "…")`.
pub(crate) fn generate(target: &Target, item_attrs: &[Attribute], span: Span) -> Result<Vec<FunctionDecl>, Failure> {
    let desc = read_meta(item_attrs, Level::Item)?.help;
    let owner = target.name;
    let src = match target.variants {
        Some(variants) => enum_source(owner, variants, &desc)?,
        None => struct_source(owner, target.fields, &desc)?,
    };
    Ok(super::generated_functions(&src, span))
}

#[derive(PartialEq, Clone, Copy)]
enum Level {
    Item,
    Variant,
    Field,
}

fn read_meta(attrs: &[Attribute], level: Level) -> Result<Meta, Failure> {
    let mut meta = Meta::default();
    for a in attrs.iter().filter(|a| a.name == "arg") {
        for bare in &a.args {
            match bare.as_str() {
                "positional" if level == Level::Field => meta.positional = true,
                "help" | "short" | "default" => return Err((format!("`{bare}` needs a value, as in `{bare} = \"text\"`"), a.span)),
                other => return Err((format!("unknown `@arg` key `{other}`"), a.span)),
            }
        }
        for (key, value) in &a.values {
            match key.as_str() {
                "help" => meta.help = value.clone(),
                "short" if level == Level::Field => {
                    let mut chars = value.chars();
                    match (chars.next(), chars.next()) {
                        (Some(c), None) if c.is_ascii_alphabetic() => meta.short = Some(c),
                        _ => return Err(("`short` is one letter".to_string(), a.span)),
                    }
                }
                "default" if level == Level::Field => meta.default = Some(value.clone()),
                "positional" => return Err(("`positional` takes no value".to_string(), a.span)),
                other => return Err((format!("unknown `@arg` key `{other}`"), a.span)),
            }
        }
    }
    Ok(meta)
}

fn scalar(ty: &TypeNode) -> Option<Scalar> {
    match ty {
        TypeNode::Int(_) => Some(Scalar::Int),
        TypeNode::Float(_) => Some(Scalar::Float),
        TypeNode::String(_) => Some(Scalar::Str),
        _ => None,
    }
}

fn classify(field: &FieldDecl) -> Result<Shape, String> {
    let ty = &field.ty;
    let bad = || format!("field `{}` has type `{}`, which a command line cannot hold", field.name, render_type(ty));
    match ty {
        TypeNode::Bool(_) => Ok(Shape::Flag),
        TypeNode::Nullable(inner, _) => match (&**inner, scalar(inner)) {
            (_, Some(sc)) => Ok(Shape::Value(sc, true)),
            (TypeNode::Named(n, _), _) => Ok(Shape::Command(n.clone(), true)),
            _ => Err(bad()),
        },
        TypeNode::Generic(n, args, _) if n == "List" && args.len() == 1 => scalar(&args[0]).map(Shape::Many).ok_or_else(bad),
        TypeNode::Named(n, _) => Ok(Shape::Command(n.clone(), false)),
        _ => match scalar(ty) {
            Some(sc) => Ok(Shape::Value(sc, false)),
            None => Err(bad()),
        },
    }
}

fn kebab(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c == '_' {
            out.push('-');
        } else if c.is_ascii_uppercase() {
            let prev = i.checked_sub(1).map(|p| chars[p]);
            let next = chars.get(i + 1).copied();
            let after_word = prev.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit());
            let acronym_end = prev.is_some_and(|p| p.is_ascii_uppercase()) && next.is_some_and(|n| n.is_ascii_lowercase());
            if after_word || acronym_end {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn members(fields: &[FieldDecl], in_variant: bool) -> Result<Vec<Member>, Failure> {
    let mut out: Vec<Member> = Vec::new();
    for f in fields {
        let at = f.attrs.first().map_or(f.span, |a| a.span);
        let meta = read_meta(&f.attrs, Level::Field)?;
        let shape = classify(f).map_err(|m| (m, f.span))?;
        let long = kebab(&f.name);
        match &shape {
            Shape::Flag if meta.positional => return Err((format!("`{}` is a `Bool`, so it cannot be positional", f.name), at)),
            Shape::Command(..) if meta.positional => return Err((format!("`{}` is a subcommand, so it cannot be positional", f.name), at)),
            Shape::Command(..) if in_variant => return Err((format!("`{}`: a subcommand cannot hold a subcommand", f.name), f.span)),
            _ => {}
        }
        if let Some(text) = &meta.default {
            let Shape::Value(sc, false) = shape else {
                return Err(("`default` goes on an `Int`, `Float` or `String` option that is not optional".to_string(), at));
            };
            if meta.positional {
                return Err(("`default` goes on an option, not a positional".to_string(), at));
            }
            let valid = match sc {
                Scalar::Int => text.parse::<i64>().is_ok(),
                Scalar::Float => text.parse::<f64>().is_ok_and(|x| x.is_finite()),
                Scalar::Str => true,
            };
            if !valid {
                return Err((format!("`default` is `{text}`, which `{}` cannot read", f.name), at));
            }
        }
        if meta.short.is_some() && meta.positional {
            return Err(("a positional has no `short` name".to_string(), at));
        }
        if !meta.positional && !matches!(shape, Shape::Command(..)) {
            if f.name == "help" {
                return Err(("`help` is reserved for `--help`".to_string(), f.span));
            }
            if meta.short == Some('h') {
                return Err(("`-h` is reserved for `--help`".to_string(), at));
            }
            if out.iter().any(|m| !m.positional && m.long == long) {
                return Err((format!("two fields are the option `--{long}`"), f.span));
            }
            if let Some(c) = meta.short
                && out.iter().any(|m| m.short == Some(c)) {
                    return Err((format!("two fields use `-{c}`"), at));
                }
        }
        out.push(Member { name: f.name.clone(), long, short: meta.short, help: meta.help, default: meta.default, positional: meta.positional, shape });
    }
    let positionals: Vec<&Member> = out.iter().filter(|m| m.positional).collect();
    if let Some(i) = positionals.iter().position(|m| matches!(m.shape, Shape::Many(_)))
        && i + 1 != positionals.len() {
            return Err(("a `List` positional takes every argument left, so it must be the last positional".to_string(), fields[0].span));
        }
    let commands = out.iter().filter(|m| matches!(m.shape, Shape::Command(..))).count();
    if commands > 1 {
        return Err(("a command line has one subcommand field".to_string(), fields[0].span));
    }
    if commands == 1 && !positionals.is_empty() {
        return Err(("a type with a subcommand has no positional arguments".to_string(), fields[0].span));
    }
    Ok(out)
}

fn lit(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn conv(sc: Scalar) -> &'static str {
    match sc {
        Scalar::Int => "|__l, __t| __args.__int(__l, __t)",
        Scalar::Float => "|__l, __t| __args.__float(__l, __t)",
        Scalar::Str => "|__l, __t| __args.__str(__l, __t)",
    }
}

fn label(m: &Member) -> String {
    m.long.to_uppercase()
}

fn reads(ms: &[Member]) -> (String, String, String) {
    let mut stmts = String::new();
    let mut pre = String::new();
    let mut inits = Vec::new();
    let mut next_pos = 0usize;
    for m in ms {
        let n = &m.name;
        let v = format!("__v_{n}");
        let long = &m.long;
        let line = if m.positional {
            let lab = lit(&format!("<{}>", label(m)));
            match &m.shape {
                Shape::Value(sc, false) => {
                    next_pos += 1;
                    format!("let {v} = __args.__pos(__s, {}, {lab}, {})?", next_pos - 1, conv(*sc))
                }
                Shape::Value(sc, true) => {
                    next_pos += 1;
                    format!("let {v} = __args.__pos_maybe(__s, {}, {lab}, {})?", next_pos - 1, conv(*sc))
                }
                Shape::Many(sc) => format!("let {v} = __args.__pos_many(__s, {next_pos}, {lab}, {})?", conv(*sc)),
                _ => unreachable!("members() rejects the other shapes"),
            }
        } else {
            match &m.shape {
                Shape::Flag => format!("let {v} = __args.__flag(__s, \"{long}\")"),
                Shape::Value(sc, false) => match &m.default {
                    Some(d) => format!("let {v} = __args.__or(__s, \"{long}\", {}, {})?", lit(d), conv(*sc)),
                    None => format!("let {v} = __args.__req(__s, \"{long}\", {})?", conv(*sc)),
                },
                Shape::Value(sc, true) => format!("let {v} = __args.__maybe(__s, \"{long}\", {})?", conv(*sc)),
                Shape::Many(sc) => format!("let {v} = __args.__many(__s, \"{long}\", {})?", conv(*sc)),
                Shape::Command(ty, false) => format!("let {v} = {ty}.__args_from(argv, __s.stop)?"),
                Shape::Command(ty, true) => {
                    pre.push_str(&format!("var {v}: {ty}? = null\n"));
                    format!("if __s.stop < argv.len() {{ {v} = {ty}.__args_from(argv, __s.stop)? }}")
                }
            }
        };
        stmts.push_str(&line);
        stmts.push('\n');
        inits.push(format!("{n}: {v}"));
    }
    if !ms.iter().any(|m| matches!(m.shape, Shape::Many(_) if m.positional) || matches!(m.shape, Shape::Command(..))) {
        stmts.push_str(&format!("__args.__no_extra(__s, {next_pos})?\n"));
    }
    (pre, stmts, inits.join(", "))
}

fn opt_table(ms: &[Member]) -> String {
    let opts: Vec<String> = ms
        .iter()
        .filter(|m| !m.positional && !matches!(m.shape, Shape::Command(..)))
        .map(|m| {
            let takes = !matches!(m.shape, Shape::Flag);
            let short = m.short.map(String::from).unwrap_or_default();
            format!("__args.__opt(\"{}\", \"{short}\", {takes})", m.long)
        })
        .collect();
    format!("[{}]", opts.join(", "))
}

fn has_command(ms: &[Member]) -> bool {
    ms.iter().any(|m| matches!(m.shape, Shape::Command(..)))
}

fn table(rows: &[(usize, String, String)]) -> String {
    let width = rows.iter().map(|(i, l, _)| i + l.len()).max().unwrap_or(0) + 2;
    let lines: Vec<String> = rows
        .iter()
        .map(|(indent, left, help)| {
            let text = format!("{}{left}", " ".repeat(*indent));
            if help.is_empty() { text } else { format!("{text:<width$}{help}") }
        })
        .collect();
    lines.join("\n")
}

fn option_left(m: &Member) -> String {
    let name = match m.short {
        Some(c) => format!("-{c}, --{}", m.long),
        None => format!("    --{}", m.long),
    };
    match &m.shape {
        Shape::Flag => name,
        _ => format!("{name} <{}>", label(m)),
    }
}

fn help_text(m: &Member) -> String {
    match &m.default {
        Some(d) if m.help.is_empty() => format!("[default: {d}]"),
        Some(d) => format!("{} [default: {d}]", m.help),
        None => m.help.clone(),
    }
}

fn positional_left(m: &Member) -> String {
    match &m.shape {
        Shape::Value(_, false) => format!("<{}>", label(m)),
        Shape::Value(_, true) => format!("[{}]", label(m)),
        _ => format!("[{}]...", label(m)),
    }
}

fn options_of(ms: &[Member]) -> Vec<&Member> {
    ms.iter().filter(|m| !m.positional && !matches!(m.shape, Shape::Command(..))).collect()
}

fn synopsis(ms: &[Member]) -> String {
    let mut s = String::from(" [OPTIONS]");
    for m in ms {
        match &m.shape {
            _ if m.positional => s.push_str(&format!(" {}", positional_left(m))),
            Shape::Command(_, false) => s.push_str(" <COMMAND>"),
            Shape::Command(_, true) => s.push_str(" [COMMAND]"),
            _ => {}
        }
    }
    s
}

fn common(owner: &str, rest_of_line: &str) -> String {
    format!(
        "fn parse(argv: List<String>) -> Result<{owner}, Error> {{ return {owner}.__args_from(argv, 1) }}
fn parse_or_exit(argv: List<String>) -> {owner} {{
let __p = __args.__program(argv)
if __args.__wants_help(argv) {{ __args.__print_help({owner}.usage(__p)) }}
match {owner}.parse(argv) {{
Ok(__v) => {{ return __v }}
Err(__e) => {{ __args.__fail({owner}.__args_line(__p), __e) }}
}}
return {owner}.parse(argv).unwrap()
}}
fn __args_line(program: String) -> String {{ return \"Usage: \" + program + {} }}
",
        lit(rest_of_line)
    )
}

fn struct_source(owner: &str, fields: &[FieldDecl], desc: &str) -> Result<String, Failure> {
    let ms = members(fields, false)?;
    let mut parts: Vec<String> = Vec::new();
    if !desc.is_empty() {
        parts.push(desc.to_string());
    }
    let args: Vec<(usize, String, String)> = ms.iter().filter(|m| m.positional).map(|m| (2, positional_left(m), m.help.clone())).collect();
    if !args.is_empty() {
        parts.push(format!("Arguments:\n{}", table(&args)));
    }
    let mut opts: Vec<(usize, String, String)> = options_of(&ms).iter().map(|m| (2, option_left(m), help_text(m))).collect();
    opts.push((2, "-h, --help".to_string(), "print this help".to_string()));
    parts.push(format!("Options:\n{}", table(&opts)));
    let mut usage = format!("{owner}.__args_line(program) + {}", lit(&format!("\n\n{}", parts.join("\n\n"))));
    if let Some(Member { shape: Shape::Command(ty, _), .. }) = ms.iter().find(|m| matches!(m.shape, Shape::Command(..))) {
        usage.push_str(&format!(" + \"\\n\\n\" + {ty}.__args_cmds()"));
    }
    usage.push_str(" + \"\\n\"");
    let (pre, stmts, inits) = reads(&ms);
    let scan = format!("let __s = __args.__scan(argv, start, {}, {})?", opt_table(&ms), has_command(&ms));
    Ok(format!(
        "{}fn usage(program: String) -> String {{ return {usage} }}
fn __args_from(argv: List<String>, start: Int) -> Result<{owner}, Error> {{
{pre}{scan}
{stmts}return Ok({owner} {{ {inits} }})
}}",
        common(owner, &synopsis(&ms))
    ))
}

fn enum_source(owner: &str, variants: &[EnumVariant], desc: &str) -> Result<String, Failure> {
    let mut names: Vec<String> = Vec::new();
    let mut rows: Vec<(usize, String, String)> = Vec::new();
    let mut arms = String::new();
    for v in variants {
        let fields: &[FieldDecl] = match &v.kind {
            EnumVariantKind::Unit { .. } => &[],
            EnumVariantKind::Struct(fields) => fields,
            EnumVariantKind::Tuple(_) => return Err((format!("variant `{}` has unnamed fields, so it cannot be a subcommand", v.name), v.span)),
        };
        let cmd = kebab(&v.name);
        if names.contains(&cmd) {
            return Err((format!("two variants are the command `{cmd}`"), v.span));
        }
        let help = read_meta(&v.attrs, Level::Variant)?.help;
        let ms = members(fields, true)?;
        rows.push((2, cmd.clone(), help));
        for m in ms.iter().filter(|m| m.positional) {
            rows.push((6, positional_left(m), m.help.clone()));
        }
        for m in options_of(&ms) {
            rows.push((6, option_left(m).trim_start().to_string(), help_text(m)));
        }
        let (_, stmts, inits) = reads(&ms);
        let value = if fields.is_empty() { format!("{owner}.{}", v.name) } else { format!("{owner}.{} {{ {inits} }}", v.name) };
        arms.push_str(&format!(
            "if __name == \"{cmd}\" {{\nlet __s = __args.__scan(argv, start + 1, {}, false)?\n{stmts}return Ok({value})\n}}\n",
            opt_table(&ms)
        ));
        names.push(cmd);
    }
    let list = format!("[{}]", names.iter().map(|n| lit(n)).collect::<Vec<_>>().join(", "));
    let cmds = format!("Commands:\n{}", table(&rows));
    let head = if desc.is_empty() { String::new() } else { format!("{desc}\n\n") };
    Ok(format!(
        "{}fn usage(program: String) -> String {{ return {owner}.__args_line(program) + {} + {owner}.__args_cmds() + \"\\n\" }}
fn __args_cmds() -> String {{ return {} }}
fn __args_from(argv: List<String>, start: Int) -> Result<{owner}, Error> {{
if start >= argv.len() {{ return Err(__args.__no_command({list})) }}
let __name = argv[start]
{arms}return Err(__args.__bad_command(__name, {list}))
}}",
        common(owner, " <COMMAND>"),
        lit(&format!("\n\n{head}")),
        lit(&cmds)
    ))
}
