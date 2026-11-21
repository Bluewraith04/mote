use std::collections::HashMap;

use mote_base::{Diagnostic, Id, Span, Symbol};
use mote_syntax as ast;
use mote_syntax::{ItemKind, Member, PatternKind, StmtKind};

use crate::cx::Cx;
use crate::defs::{Alias, Def, DefId, DefKind, Enum, Function, Global, ModuleId, Origin, Receiver, Record, Resolved, Trait};
use crate::types::TyId;

/// A method as the syntax tree declares it.
struct MethodDecl {
    member: u32,
    name: ast::Ident,
    is_pub: bool,
    has_body: bool,
}

fn function(has_body: bool, native: bool) -> DefKind {
    DefKind::Function(Function { generics: Vec::new(), params: Vec::new(), receiver: Receiver::None, ret: TyId::UNIT, has_body, native, link: None, intrinsic: None })
}

pub(crate) fn bound_names(ast: &ast::Ast, pattern: Id<ast::Pattern>, out: &mut Vec<ast::Ident>) {
    match &ast[pattern].kind {
        PatternKind::Binding(name) => out.push(*name),
        PatternKind::At { name, pattern } => {
            out.push(*name);
            bound_names(ast, *pattern, out);
        }
        PatternKind::Or(alternatives) => {
            if let Some(&first) = alternatives.first() {
                bound_names(ast, first, out);
            }
        }
        PatternKind::Tuple(elements) | PatternKind::TupleStruct { elements, .. } => {
            for &element in elements {
                bound_names(ast, element, out);
            }
        }
        PatternKind::Struct { fields, .. } => {
            for field in fields {
                match field.pattern {
                    Some(inner) => bound_names(ast, inner, out),
                    None => out.push(field.name),
                }
            }
        }
        PatternKind::Type { name: Some(name), .. } => out.push(*name),
        _ => {}
    }
}

impl Cx<'_> {
    /// Gives every item a definition and binds the module's own names.
    pub(super) fn collect(&mut self) {
        for index in 0..self.ids.len() {
            let module = self.ids[index];
            let ast = self.ast(module);
            for &item_id in &ast.root {
                self.collect_item(module, item_id, &ast[item_id]);
            }
        }
    }

    fn add_def(&mut self, module: ModuleId, name: ast::Ident, is_pub: bool, origin: Option<Origin>, owner: Option<DefId>, kind: DefKind) -> DefId {
        self.program.defs.alloc(Def { name: name.name, module, span: name.span, is_pub, lang: None, owner, origin, kind })
    }

    fn define(&mut self, module: ModuleId, name: ast::Ident, def: DefId, is_pub: bool) {
        self.bind(module, name.name, Resolved::Def(def), name.span, is_pub);
    }

    fn collect_item(&mut self, module: ModuleId, item_id: Id<ast::Item>, item: &ast::Item) {
        let origin = Some(Origin { item: item_id, member: None });
        let ast = self.ast(module);
        match &item.kind {
            ItemKind::Function(f) => {
                let def = self.add_def(module, f.name, item.is_pub, origin, None, function(true, false));
                self.define(module, f.name, def, item.is_pub);
                self.attributes(module, item, def);
            }
            ItemKind::NativeFunction(f) => {
                let def = self.add_def(module, f.name, item.is_pub, origin, None, function(false, true));
                self.define(module, f.name, def, item.is_pub);
                self.attributes(module, item, def);
            }
            ItemKind::Test(test) => {
                let def = self.add_def(module, ast::Ident { name: test.title, span: item.span }, false, origin, None, DefKind::Test);
                if item.is_pub {
                    self.error(item.span, "a test cannot be `pub`");
                }
                self.attributes(module, item, def);
            }
            ItemKind::Record(record) => {
                let kind = DefKind::Record(Record { is_class: record.kind == ast::RecordKind::Class, ..Record::default() });
                let def = self.add_def(module, record.name, item.is_pub, origin, None, kind);
                self.define(module, record.name, def, item.is_pub);
                self.attributes(module, item, def);
                let decls: Vec<_> = record
                    .members
                    .iter()
                    .enumerate()
                    .filter_map(|(member, entry)| match entry {
                        Member::Method(method) => Some(MethodDecl { member: member as u32, name: method.function.name, is_pub: method.is_pub, has_body: true }),
                        Member::Field(_) => None,
                    })
                    .collect();
                let methods = self.collect_methods(module, item_id, def, &decls, &[]);
                if let DefKind::Record(record) = &mut self.program.defs[def].kind {
                    record.methods = methods;
                }
            }
            ItemKind::Enum(en) => {
                let def = self.add_def(module, en.name, item.is_pub, origin, None, DefKind::Enum(Enum::default()));
                self.define(module, en.name, def, item.is_pub);
                self.attributes(module, item, def);
                let mut seen: HashMap<Symbol, Span> = HashMap::new();
                for variant in &en.variants {
                    if let Some(&first) = seen.get(&variant.name.name) {
                        let message = format!("`{}` is defined twice", self.text(variant.name.name));
                        self.sink.emit(Diagnostic::error(message, variant.name.span).label(first, "first defined here"));
                    } else {
                        seen.insert(variant.name.name, variant.name.span);
                    }
                }
                let reserved: Vec<Symbol> = en.variants.iter().map(|variant| variant.name.name).collect();
                let decls: Vec<_> = en
                    .methods
                    .iter()
                    .enumerate()
                    .map(|(member, method)| MethodDecl { member: member as u32, name: method.function.name, is_pub: method.is_pub, has_body: true })
                    .collect();
                let methods = self.collect_methods(module, item_id, def, &decls, &reserved);
                if let DefKind::Enum(en) = &mut self.program.defs[def].kind {
                    en.methods = methods;
                }
            }
            ItemKind::Trait(tr) => {
                let def = self.add_def(module, tr.name, item.is_pub, origin, None, DefKind::Trait(Trait::default()));
                self.define(module, tr.name, def, item.is_pub);
                self.attributes(module, item, def);
                let decls: Vec<_> = tr
                    .members
                    .iter()
                    .enumerate()
                    .map(|(member, entry)| MethodDecl { member: member as u32, name: entry.name, is_pub: item.is_pub, has_body: entry.body.is_some() })
                    .collect();
                let methods = self.collect_methods(module, item_id, def, &decls, &[]);
                if let DefKind::Trait(tr) = &mut self.program.defs[def].kind {
                    tr.methods = methods;
                }
            }
            ItemKind::TypeAlias(alias) => {
                let kind = DefKind::Alias(Alias { generics: Vec::new(), target: TyId::ERROR });
                let def = self.add_def(module, alias.name, item.is_pub, origin, None, kind);
                self.define(module, alias.name, def, item.is_pub);
                self.attributes(module, item, def);
            }
            ItemKind::Stmt(stmt) => {
                let StmtKind::Binding(binding) = &ast[*stmt].kind else { return };
                let mut names = Vec::new();
                bound_names(ast, binding.pattern, &mut names);
                for name in names {
                    let kind = DefKind::Global(Global { ty: TyId::ERROR, is_var: binding.kind == ast::BindingKind::Var });
                    let def = self.add_def(module, name, item.is_pub, origin, None, kind);
                    self.define(module, name, def, item.is_pub);
                }
            }
            ItemKind::Import(_) | ItemKind::Error => {}
        }
    }

    /// Defines the methods of one type; only `from` may repeat a name.
    fn collect_methods(&mut self, module: ModuleId, item: Id<ast::Item>, owner: DefId, decls: &[MethodDecl], variants: &[Symbol]) -> Vec<DefId> {
        let mut seen: HashMap<Symbol, Span> = HashMap::new();
        let mut methods = Vec::new();
        for decl in decls {
            let name = decl.name.name;
            if variants.contains(&name) {
                let message = format!("`{}` is a variant and a method", self.text(name));
                self.error(decl.name.span, message);
                continue;
            }
            if name != self.from {
                if let Some(&first) = seen.get(&name) {
                    let message = format!("`{}` is defined twice", self.text(name));
                    self.sink.emit(Diagnostic::error(message, decl.name.span).label(first, "first defined here"));
                    continue;
                }
                seen.insert(name, decl.name.span);
            }
            let origin = Some(Origin { item, member: Some(decl.member) });
            let def = self.add_def(module, decl.name, decl.is_pub, origin, Some(owner), function(decl.has_body, false));
            methods.push(def);
        }
        methods
    }
}
