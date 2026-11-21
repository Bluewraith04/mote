use std::collections::HashSet;

use mote_base::{Diagnostic, Id, Span, Symbol};
use mote_syntax as ast;
use mote_syntax::{Import, ItemKind};

use crate::cx::Cx;
use crate::defs::{ModuleId, Resolved};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Visit {
    New,
    Open,
    Done,
}

/// A depth-first walk over the import graph, dependencies first.
struct Walk {
    state: Vec<Visit>,
    stack: Vec<usize>,
    order: Vec<usize>,
}

type Edges = Vec<Vec<(Id<ast::Item>, usize)>>;

impl Cx<'_> {
    /// Fills each module's scope and exports from its imports; a cycle is reported and its closing import skipped.
    pub(super) fn imports(&mut self) {
        let edges = self.import_edges();
        let mut walk = Walk { state: vec![Visit::New; edges.len()], stack: Vec::new(), order: Vec::new() };
        for index in 0..edges.len() {
            if walk.state[index] == Visit::New {
                self.visit(index, &edges, &mut walk);
            }
        }
        for index in walk.order {
            self.bind_imports(index);
        }
    }

    fn import_edges(&self) -> Edges {
        (0..self.ids.len())
            .map(|index| {
                let ast = self.ast(self.ids[index]);
                ast.root
                    .iter()
                    .filter(|&&item| matches!(ast[item].kind, ItemKind::Import(_)))
                    .filter_map(|&item| self.imports[index].get(&item).map(|&target| (item, target)))
                    .collect()
            })
            .collect()
    }

    fn visit(&mut self, index: usize, edges: &Edges, walk: &mut Walk) {
        walk.state[index] = Visit::Open;
        walk.stack.push(index);
        for &(item, target) in &edges[index] {
            match walk.state[target] {
                Visit::New => self.visit(target, edges, walk),
                Visit::Open => {
                    let start = walk.stack.iter().position(|&member| member == target).expect("an open module is on the stack");
                    let mut names: Vec<String> = walk.stack[start..].iter().map(|&member| self.module_name(self.ids[member])).collect();
                    names.push(self.module_name(self.ids[target]));
                    let span = self.ast(self.ids[index])[item].span;
                    self.error(span, format!("circular import {}", names.join(" -> ")));
                    self.cut.insert((index, item));
                }
                Visit::Done => {}
            }
        }
        walk.stack.pop();
        walk.state[index] = Visit::Done;
        walk.order.push(index);
    }

    fn bind_imports(&mut self, index: usize) {
        let module = self.ids[index];
        let ast = self.ast(module);
        let mut globs = Vec::new();
        for &item_id in &ast.root {
            let item = &ast[item_id];
            let ItemKind::Import(import) = &item.kind else { continue };
            if self.cut.contains(&(index, item_id)) {
                continue;
            }
            let Some(&target) = self.imports[index].get(&item_id) else { continue };
            let target = self.ids[target];
            match import {
                Import::Module { path, alias } => {
                    let Some(name) = alias.or(path.segments.last().copied()) else { continue };
                    self.bind(module, name.name, Resolved::Module(target), name.span, item.is_pub);
                }
                Import::Names { names, .. } => {
                    for entry in names {
                        let name = entry.alias.unwrap_or(entry.name);
                        match self.program.modules[target].exports.get(&entry.name.name).copied() {
                            Some(resolved) => self.bind(module, name.name, resolved, name.span, item.is_pub),
                            None => self.missing_name(target, entry.name),
                        }
                    }
                }
                Import::Glob { .. } => globs.push((target, item.is_pub, item.span)),
            }
        }
        self.bind_globs(module, &globs);
    }

    pub(crate) fn missing_name(&mut self, target: ModuleId, name: ast::Ident) {
        let message = if self.program.modules[target].scope.contains_key(&name.name) {
            format!("`{}` is private in {}", self.text(name.name), self.module_name(target))
        } else {
            format!("no `{}` in {}", self.text(name.name), self.module_name(target))
        };
        self.error(name.span, message);
    }

    /// Adds the names of `{ * }` imports; a module's own and explicitly imported names win.
    fn bind_globs(&mut self, module: ModuleId, globs: &[(ModuleId, bool, Span)]) {
        let index = module.index();
        let mut from_glob: HashSet<Symbol> = HashSet::new();
        for &(target, public, span) in globs {
            let mut names: Vec<(Symbol, Resolved)> = self.program.modules[target].exports.iter().map(|(&name, &resolved)| (name, resolved)).collect();
            names.sort_by(|a, b| self.interner.resolve(a.0).cmp(self.interner.resolve(b.0)));
            for (name, resolved) in names {
                match self.program.modules[module].scope.get(&name).copied() {
                    None => {
                        self.program.modules[module].scope.insert(name, resolved);
                        self.bound_at[index].insert(name, span);
                        from_glob.insert(name);
                    }
                    Some(_) if !from_glob.contains(&name) => continue,
                    Some(existing) if existing == resolved => {}
                    Some(_) => {
                        let first = self.bound_at[index][&name];
                        let message = format!("`{}` comes from two glob imports", self.text(name));
                        self.sink.emit(Diagnostic::error(message, span).label(first, "first imported here"));
                        continue;
                    }
                }
                if public {
                    self.program.modules[module].exports.insert(name, resolved);
                }
            }
        }
    }
}
