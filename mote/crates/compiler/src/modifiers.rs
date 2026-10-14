//! Compile-time modifiers: the registry of them and the pass that applies them after parsing.

pub(crate) mod attributes;
mod wire;

pub use wire::decl_json;

use crate::ast::*;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::span::{Span, DERIVED_BASE};

/// A diagnostic a modifier or the pass reports: the message and where it points.
pub type Diagnostic = (String, Span);

/// Which kind of declaration a modifier reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeclKind {
    Struct,
    Class,
    Enum,
}

/// What a modifier reads: one declaration as written, names unresolved.
pub struct Decl<'a> {
    pub kind: DeclKind,
    pub name: &'a str,
    pub is_pub: bool,
    /// The `///` lines before the declaration.
    pub doc: &'a str,
    pub generics: &'a [GenericParam],
    pub traits: &'a [TypeNode],
    pub fields: &'a [FieldDecl],
    /// `Some` for an enum.
    pub variants: Option<&'a [EnumVariant]>,
    /// The methods already on the declaration, so a modifier can refuse a clash.
    pub methods: &'a [FunctionDecl],
    /// Every attribute still on the declaration, `@arg(…)` among them.
    pub attrs: &'a [Attribute],
    pub span: Span,
}

/// What a modifier answers: `fn` sources that become methods of the declaration.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Additions {
    /// Methods other modules may call.
    pub public: String,
    /// Helpers for the generated methods, private to the declaring module.
    pub private: String,
}

/// A modifier written in Mote that a trait names with `@generate`.
#[derive(Clone, Debug)]
pub struct Generator {
    pub trait_name: String,
    /// The `std` module that defines the trait (`traits`), empty for any other module.
    pub module: String,
    /// The trait's required methods.
    pub methods: Vec<String>,
    /// The host's handle for running it.
    pub id: usize,
}

/// What the pass asks of the module system: which trait a name is, and to run its modifier.
pub trait Host {
    /// The trait `name` stands for in the module being expanded, if it has a modifier.
    fn find(&mut self, name: &str, at: Span) -> Result<Option<Generator>, Diagnostic>;
    /// Runs the modifier on the declaration `decl_json` describes: its additions, or why it refused.
    fn run(&mut self, generator: &Generator, decl_json: &str) -> Result<Additions, String>;
    /// The member attributes the modifier reads (`@modifier(reads = "arg")`).
    fn reads(&mut self, generator: &Generator) -> Vec<String>;
}

/// The alias a modifier's generated code reaches its own module through.
fn alias_of(module: &str) -> String {
    format!("__m_{}", module.replace('.', "_"))
}

/// Rewrites `__modifier.` in generated source to the alias of `module`; says whether any was found.
fn hygiene(src: &str, module: &str) -> (String, bool) {
    let used = src.contains("__modifier.");
    (src.replace("__modifier.", &format!("{}.", alias_of(module))), used)
}

/// Parses generated `fn` source into methods whose token spans map to the request.
fn parse_methods(src: &str, name: &str, span: Span, public: bool) -> Result<Vec<FunctionDecl>, Diagnostic> {
    let broken = |why: String| (format!("`{name}` generated code that does not parse: {why}"), span);
    let mut tokens = Lexer::new(src).tokenize().map_err(|(e, _)| broken(e))?;
    let base = DERIVED_BASE.wrapping_add(span.start << 24);
    for t in &mut tokens {
        t.span = Span { start: base + t.span.start, end: base + t.span.end, line: span.line, col: span.col, source: span.source };
    }
    let items = Parser::new(tokens).parse().map_err(|(e, _)| broken(e))?.items;
    items
        .into_iter()
        .map(|item| {
            let Item::Function(mut f) = item else { return Err((format!("`{name}` may only add `fn`s"), span)) };
            f.span = span;
            f.is_pub = public;
            Ok(f)
        })
        .collect()
}

/// Member data (`@arg`) on the declaration that no modifier it asked for reads.
fn unread_member_data(p: &Parts, read: &[String]) -> Result<(), Diagnostic> {
    let mut sets: Vec<(&[Attribute], &str)> = vec![(&p.attrs[..], "type")];
    for v in p.variants.unwrap_or(&[]) {
        sets.push((&v.attrs[..], "variant"));
    }
    for (attrs, what) in sets {
        if let Some(a) = attrs.iter().find(|a| attributes::is_member_data(&a.name) && !read.contains(&a.name)) {
            return Err((format!("`@{}` on a {what} is not read by any trait `{}` lists", a.name, p.name), a.span));
        }
    }
    Ok(())
}

/// The pieces of one type declaration the pass works on.
struct Parts<'a> {
    kind: DeclKind,
    name: &'a str,
    is_pub: bool,
    doc: &'a str,
    generics: &'a [GenericParam],
    traits: &'a [TypeNode],
    fields: &'a [FieldDecl],
    variants: Option<&'a [EnumVariant]>,
    span: Span,
    attrs: &'a mut Vec<Attribute>,
    methods: &'a mut Vec<FunctionDecl>,
}

/// What the pass collects across a program.
struct Pass<'h> {
    applied: Vec<AppliedModifier>,
    /// Modules whose alias import the generated code needs.
    imports: Vec<(String, Span)>,
    host: &'h mut dyn Host,
}

impl Pass<'_> {
    /// Applies each listed trait that has a modifier and none of its methods written.
    fn declaration(&mut self, mut p: Parts) -> Result<bool, Diagnostic> {
        let mut read: Vec<String> = Vec::new();
        let mut listed = Vec::new();
        for t in p.traits {
            let TypeNode::Named(name, at) = t else { continue };
            let Some(g) = self.host.find(name, *at)? else { continue };
            if g.methods.iter().any(|m| p.methods.iter().any(|x| x.name == *m)) {
                continue;
            }
            read.extend(self.host.reads(&g));
            listed.push((name.clone(), *at, g));
        }
        let any = !listed.is_empty();
        for (name, at, g) in listed {
            self.apply(&mut p, &name, at, g)?;
        }
        unread_member_data(&p, &read)?;
        Ok(any)
    }

    /// Runs one modifier on the declaration and adds what it answers.
    fn apply(&mut self, p: &mut Parts, name: &str, at: Span, g: Generator) -> Result<(), Diagnostic> {
        let label = format!("listing `{name}`");
        let clash = |p: &Parts, method: &str| (format!("`{}` already has a method named `{method}`, so {label} cannot add one", p.name), at);
        let additions = {
            let decl = Decl {
                kind: p.kind,
                name: p.name,
                is_pub: p.is_pub,
                doc: p.doc,
                generics: p.generics,
                traits: p.traits,
                fields: p.fields,
                variants: p.variants,
                methods: p.methods,
                attrs: p.attrs,
                span: p.span,
            };
            self.host.run(&g, &wire::decl_json(&decl)).map_err(|why| (format!("{label}: {why}"), at))?
        };
        let module = g.module.clone();
        let mut code = Vec::new();
        for (src, public) in [(additions.public, true), (additions.private, false)] {
            if src.is_empty() {
                continue;
            }
            let (src, uses_alias) = hygiene(&src, &module);
            if uses_alias {
                if module.is_empty() {
                    return Err((format!("{label}: generated code can name its modifier's module only when that is a `std` module"), at));
                }
                if !self.imports.iter().any(|(m, _)| *m == module) {
                    self.imports.push((module.clone(), at));
                }
            }
            let parsed = parse_methods(&src, name, at, public)?;
            if let Some(f) = parsed.iter().find(|f| p.methods.iter().any(|m| m.name == f.name)) {
                return Err(clash(p, &f.name));
            }
            p.methods.extend(parsed);
            code.push(src);
        }
        self.applied.push(AppliedModifier {
            name: name.to_string(),
            module,
            span: at,
            target: p.name.to_string(),
            insert_at: p.span.end.saturating_sub(1),
            code: code.join("\n"),
        });
        Ok(())
    }
}

/// `source` with each application's generated code written into its declaration.
pub fn render(source: &str, applied: &[AppliedModifier]) -> String {
    let mut sorted: Vec<&AppliedModifier> = applied.iter().collect();
    sorted.sort_by_key(|a| a.insert_at);
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0;
    for a in sorted {
        let at = a.insert_at.clamp(cursor, source.len());
        out.push_str(&source[cursor..at]);
        out.push_str(&format!("\n    // generated by {} (requested at line {})\n", a.name, a.span.line));
        for line in a.code.lines() {
            out.push_str("    ");
            out.push_str(line);
            out.push('\n');
        }
        cursor = at;
    }
    out.push_str(&source[cursor..]);
    out
}

/// The JSON text of the `struct`, `class` or `enum` named `name` in `program`.
pub fn declaration_json(program: &Program, name: &str) -> Option<String> {
    let decl = |kind, name, is_pub, doc, generics, traits, fields, variants, methods, attrs, span| {
        wire::decl_json(&Decl { kind, name, is_pub, doc, generics, traits, fields, variants, methods, attrs, span })
    };
    program.items.iter().find_map(|item| match item {
        Item::Struct(s) if s.name == name => {
            Some(decl(DeclKind::Struct, &s.name[..], s.is_pub, &s.doc[..], &s.generic_params[..], &s.traits[..], &s.fields[..], None, &s.methods[..], &s.attrs[..], s.span))
        }
        Item::Class(c) if c.name == name => {
            Some(decl(DeclKind::Class, &c.name[..], c.is_pub, &c.doc[..], &c.generic_params[..], &c.traits[..], &c.fields[..], None, &c.methods[..], &c.attrs[..], c.span))
        }
        Item::Enum(e) if e.name == name => {
            Some(decl(DeclKind::Enum, &e.name[..], e.is_pub, &e.doc[..], &e.generic_params[..], &e.traits[..], &[], Some(&e.variants[..]), &e.methods[..], &e.attrs[..], e.span))
        }
        _ => None,
    })
}

/// The modifier pass: expands every listed trait that has a modifier into methods.
pub fn expand(program: &mut Program, host: &mut dyn Host) -> Result<(), Diagnostic> {
    let mut pass = Pass { applied: Vec::new(), imports: Vec::new(), host };
    let items = std::mem::take(&mut program.items);
    let mut out = Vec::with_capacity(items.len());
    for mut item in items {
        let expanded = match &mut item {
            Item::Struct(s) => {
                let StructDecl { name, is_pub, doc, generic_params, traits, fields, methods, attrs, span } = s;
                pass.declaration(Parts { kind: DeclKind::Struct, name, is_pub: *is_pub, doc: &doc[..], generics: generic_params, traits, fields, variants: None, span: *span, attrs, methods })?
            }
            Item::Class(c) => {
                let ClassDecl { name, is_pub, doc, generic_params, traits, fields, methods, attrs, span } = c;
                pass.declaration(Parts { kind: DeclKind::Class, name, is_pub: *is_pub, doc: &doc[..], generics: generic_params, traits, fields, variants: None, span: *span, attrs, methods })?
            }
            Item::Enum(e) => {
                let EnumDecl { name, is_pub, doc, generic_params, traits, variants, methods, attrs, span } = e;
                pass.declaration(Parts { kind: DeclKind::Enum, name, is_pub: *is_pub, doc: &doc[..], generics: generic_params, traits, fields: &[], variants: Some(variants), span: *span, attrs, methods })?
            }
            _ => false,
        };
        if expanded && let Some((kind, _, name, signature)) = crate::stable::describe(&item)
            && let Some(mark) = program.stable_marks.iter_mut().find(|m| m.name == name && m.kind == kind) {
                mark.signature = signature;
            }
        out.push(item);
    }
    for (module, span) in &pass.imports {
        let path = ModulePath::new(std::iter::once("std").chain(module.split('.')).map(String::from).collect(), false, 0, *span);
        out.push(Item::Import(ImportDecl { path, from_path: None, alias: Some(alias_of(module)), symbols: Vec::new(), glob: false, is_pub: false, span: *span }));
    }
    program.items = out;
    program.applied.extend(pass.applied);
    Ok(())
}
