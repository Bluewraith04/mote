use std::collections::HashMap;

use mote_base::{Diagnostic, Id, Span, Symbol};
use mote_syntax as ast;
use mote_syntax::{ItemKind, Member, PatternKind, StmtKind};

use crate::cx::Cx;
use crate::defs::{Alias, ConstDef, Def, DefId, DefKind, Enum, ExtendDef, ExtendId, Function, Global, Link, ModuleId, Origin, Receiver, Record, Resolved, Trait};
use crate::types::TyId;

/// A method as the syntax tree declares it.
struct MethodDecl {
    member: u32,
    name: ast::Ident,
    is_pub: bool,
    has_body: bool,
}

fn function(has_body: bool, link: Option<Link>) -> DefKind {
    DefKind::Function(Function { generics: Vec::new(), params: Vec::new(), receiver: Receiver::None, ret: TyId::UNIT, has_body, link })
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

    #[allow(clippy::too_many_arguments)]
    fn add_def(
        &mut self,
        module: ModuleId,
        name: ast::Ident,
        is_pub: bool,
        origin: Option<Origin>,
        owner: Option<DefId>,
        extend: Option<ExtendId>,
        kind: DefKind,
    ) -> DefId {
        self.program.defs.alloc(Def { name: name.name, module, span: name.span, is_pub, owner, extend, origin, doc: None, kind })
    }

    fn define(&mut self, module: ModuleId, name: ast::Ident, def: DefId, is_pub: bool) {
        self.bind(module, name.name, Resolved::Def(def), name.span, is_pub);
    }

    fn record_std_item(&mut self, module: ModuleId, name: Symbol, def: DefId) {
        if !self.program.modules[module].is_std {
            return;
        }
        let path: Vec<&str> = self.program.modules[module].path.iter().map(|&s| self.interner.resolve(s)).collect();
        let sym = self.interner.resolve(name);
        match (path.as_slice(), sym) {
            (["std", "collections"], "Map") => self.program.std.map = Some(def),
            (["std", "collections"], "Set") => self.program.std.set = Some(def),
            (["std", "collections"], "Range") => self.program.std.range = Some(def),
            (["std", "result"], "Result") | (["std", "option"], "Result") => self.program.std.result = Some(def),
            (["std", "error"], "Error") => self.program.std.error = Some(def),
            (["std", "error"], "ErrorKind") => self.program.std.error_kind = Some(def),
            (["std", "traits"], "Display") => self.program.std.display = Some(def),
            (["std", "traits"], "Debug") => self.program.std.debug = Some(def),
            (["std", "traits"], "Eq") => self.program.std.eq = Some(def),
            (["std", "traits"], "Ord") => self.program.std.ord = Some(def),
            (["std", "traits"], "Clone") => self.program.std.clone = Some(def),
            (["std", "traits"], "Closeable") => self.program.std.closeable = Some(def),
            (["std", "traits"], "Key") => self.program.std.key = Some(def),
            (["std", "reflect"], "TypeInfo") => self.program.std.type_info = Some(def),
            (["std", "reflect"], "FieldInfo") => self.program.std.field_info = Some(def),
            (["std", "reflect"], "VariantInfo") => self.program.std.variant_info = Some(def),
            (["std", "reflect"], "Kind") => self.program.std.reflect_kind = Some(def),
            (["std", "reflect"], "Shape") => self.program.std.reflect_shape = Some(def),
            (["std", "collections"], "Span") => self.program.std.span = Some(def),
            (["std", "collections"], "MutSpan") => self.program.std.mut_span = Some(def),
            (["std", "string"], "StrView") => self.program.std.str_view = Some(def),
            (["std", "mem"], "Buffer") => self.program.std.buffer = Some(def),
            (["std", "bytes"], "Bytes") => self.program.std.bytes = Some(def),
            _ => {}
        }
    }

    fn collect_item(&mut self, module: ModuleId, item_id: Id<ast::Item>, item: &ast::Item) {
        let raw_origin = Origin { item: item_id, member: None };
        let origin = Some(raw_origin);
        let ast = self.ast(module);
        match &item.kind {
            ItemKind::Function(f) => {
                let def = self.add_def(module, f.name, item.is_pub, origin, None, None, function(true, None));
                self.define(module, f.name, def, item.is_pub);
                self.attributes(module, item, def);
            }
            ItemKind::ExternBlock(eb) => {
                let library = eb.library;
                let decls = eb.decls.clone();
                let mut seen: HashMap<Symbol, Span> = HashMap::new();
                for (member, decl) in decls.iter().enumerate() {
                    if let Some(&first) = seen.get(&decl.name.name) {
                        let message = format!("`{}` is defined twice", self.text(decl.name.name));
                        self.sink.emit(Diagnostic::error(message, decl.name.span).label(first, "first defined here"));
                        continue;
                    }
                    seen.insert(decl.name.name, decl.name.span);
                    let blocking = self.extern_fn_attributes(decl);
                    let is_pub = item.is_pub || decl.is_pub;
                    let link = Some(Link { library, symbol: decl.name.name, blocking });
                    let member_origin = Some(Origin { item: item_id, member: Some(member as u32) });
                    let def = self.add_def(module, decl.name, is_pub, member_origin, None, None, function(false, link));
                    self.define(module, decl.name, def, is_pub);
                }
            }
            ItemKind::Extend(ext) => {
                let ext_id = self.program.extends.alloc(ExtendDef {
                    generics: Vec::new(),
                    target: TyId::ERROR,
                    traits: Vec::new(),
                    methods: Vec::new(),
                    module,
                    origin: raw_origin,
                    span: item.span,
                });
                let decls: Vec<_> = ext
                    .methods
                    .iter()
                    .enumerate()
                    .map(|(member, method)| MethodDecl { member: member as u32, name: method.function.name, is_pub: method.is_pub, has_body: true })
                    .collect();
                let methods = self.collect_extend_methods(module, item_id, ext_id, &decls);
                self.program.extends[ext_id].methods = methods;
            }
            ItemKind::Const(c) => {
                let kind = DefKind::Const(ConstDef { ty: TyId::ERROR, ast_ty: c.ty, expr: c.value });
                let def = self.add_def(module, c.name, item.is_pub, origin, None, None, kind);
                self.define(module, c.name, def, item.is_pub);
                self.attributes(module, item, def);
            }
            ItemKind::Test(test) => {
                let def = self.add_def(module, ast::Ident { name: test.title, span: item.span }, false, origin, None, None, DefKind::Test);
                if item.is_pub {
                    self.error(item.span, "a test cannot be `pub`");
                }
                self.attributes(module, item, def);
            }
            ItemKind::Record(record) => {
                let kind = DefKind::Record(Record { is_class: record.kind == ast::RecordKind::Class, ..Record::default() });
                let rec_name = record.name;
                let def = self.add_def(module, rec_name, item.is_pub, origin, None, None, kind);
                self.program.defs[def].doc = item.doc;
                self.define(module, rec_name, def, item.is_pub);
                self.record_std_item(module, rec_name.name, def);
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
                let en_name = en.name;
                let def = self.add_def(module, en_name, item.is_pub, origin, None, None, DefKind::Enum(Enum::default()));
                self.define(module, en_name, def, item.is_pub);
                self.record_std_item(module, en_name.name, def);
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
                let tr_name = tr.name;
                let def = self.add_def(module, tr_name, item.is_pub, origin, None, None, DefKind::Trait(Trait::default()));
                self.define(module, tr_name, def, item.is_pub);
                self.record_std_item(module, tr_name.name, def);
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
                let def = self.add_def(module, alias.name, item.is_pub, origin, None, None, kind);
                self.define(module, alias.name, def, item.is_pub);
                self.attributes(module, item, def);
            }
            ItemKind::Stmt(stmt) => {
                match &ast[*stmt].kind {
                    StmtKind::Binding(binding) => {
                        let mut names = Vec::new();
                        bound_names(ast, binding.pattern, &mut names);
                        for name in names {
                            let kind = DefKind::Global(Global { ty: TyId::ERROR, is_var: binding.kind == ast::BindingKind::Var });
                            let def = self.add_def(module, name, item.is_pub, origin, None, None, kind);
                            self.define(module, name, def, item.is_pub);
                        }
                    }
                    StmtKind::Const(c) => {
                        let kind = DefKind::Const(ConstDef { ty: TyId::ERROR, ast_ty: c.ty, expr: c.value });
                        let def = self.add_def(module, c.name, item.is_pub, origin, None, None, kind);
                        self.define(module, c.name, def, item.is_pub);
                    }
                    _ => {}
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
            let def = self.add_def(module, decl.name, decl.is_pub, origin, Some(owner), None, function(decl.has_body, None));
            methods.push(def);
        }
        methods
    }

    /// Defines the methods declared inside an `extend` block.
    fn collect_extend_methods(&mut self, module: ModuleId, item: Id<ast::Item>, extend: ExtendId, decls: &[MethodDecl]) -> Vec<DefId> {
        let mut seen: HashMap<Symbol, Span> = HashMap::new();
        let mut methods = Vec::new();
        for decl in decls {
            let name = decl.name.name;
            if name != self.from {
                if let Some(&first) = seen.get(&name) {
                    let message = format!("`{}` is defined twice", self.text(name));
                    self.sink.emit(Diagnostic::error(message, decl.name.span).label(first, "first defined here"));
                    continue;
                }
                seen.insert(name, decl.name.span);
            }
            let origin = Some(Origin { item, member: Some(decl.member) });
            let def = self.add_def(module, decl.name, decl.is_pub, origin, None, Some(extend), function(decl.has_body, None));
            methods.push(def);
        }
        methods
    }
}
