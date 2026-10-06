use std::collections::{HashMap, HashSet};
use std::path::Path;
use compiler::ast::{Item, Program, Stmt};
use crate::path::CanonicalModuleId;
use crate::resolver::ModuleResolver;

/// Where a re-exported name is really defined; re-exports of re-exports are followed to the source when registered.
#[derive(Clone, Debug)]
pub struct ReExportSource {
    pub defining_module: CanonicalModuleId,
    pub defining_name: String,
    pub is_type: bool,
}

#[derive(Clone, Debug, Default)]
/// The names a module exports.
pub struct ModuleExports {
    pub functions: HashSet<String>,
    pub structs: HashSet<String>,
    pub classes: HashSet<String>,
    pub enums: HashSet<String>,
    pub traits: HashSet<String>,
    /// `pub type` aliases.
    pub aliases: HashSet<String>,
    /// `pub` top-level `let` / `var` globals another module may import by name.
    pub globals: HashSet<String>,
    /// Exported name -> where it is defined; only selective `pub import { a, b as c } from <path>` populates it.
    pub re_exports: HashMap<String, ReExportSource>,
}

/// Checks that a reference reaches only exported names.
pub struct VisibilityChecker {
    module_exports: HashMap<CanonicalModuleId, ModuleExports>,
}

impl Default for VisibilityChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl VisibilityChecker {
    pub fn new() -> Self {
        Self {
            module_exports: HashMap::new(),
        }
    }

    /// Collects all `pub` exports from a module AST, following `pub import` re-exports to their source. Call in topological order, so a re-export's target is already registered.
    pub fn register_module_exports(
        &mut self,
        id: &CanonicalModuleId,
        program: &Program,
        resolver: &mut ModuleResolver,
        module_dir: &Path,
    ) -> Result<(), String> {
        let mut exports = ModuleExports::default();

        for item in &program.items {
            match item {
                Item::Function(f) => {
                    if f.is_pub {
                        exports.functions.insert(f.name.clone());
                    }
                }
                Item::Struct(s) => {
                    if s.is_pub {
                        exports.structs.insert(s.name.clone());
                    }
                }
                Item::Class(c) => {
                    if c.is_pub {
                        exports.classes.insert(c.name.clone());
                    }
                }
                Item::Trait(t) => {
                    if t.is_pub {
                        exports.traits.insert(t.name.clone());
                    }
                }
                Item::Enum(e) => {
                    if e.is_pub {
                        exports.enums.insert(e.name.clone());
                    }
                }
                Item::TypeAlias(a) => {
                    if a.is_pub {
                        exports.aliases.insert(a.name.clone());
                    }
                }
                Item::TopLevelStmt(
                    Stmt::Let { name, is_pub: true, .. } | Stmt::Var { name, is_pub: true, .. },
                ) => {
                    exports.globals.insert(name.clone());
                }
                Item::Import(imp) => {
                    if !imp.is_pub || (imp.symbols.is_empty() && !imp.glob) {
                        continue;
                    }
                    let (target_id, _target) = resolver.resolve_path(&imp.path, module_dir)?;
                    let target_exports = self.module_exports.get(&target_id).cloned().unwrap_or_default();
                    if imp.glob {
                        let defined = |names: &HashSet<String>, is_type: bool| {
                            names.iter().map(|n| (n.clone(), ReExportSource { defining_module: target_id.clone(), defining_name: n.clone(), is_type })).collect::<Vec<_>>()
                        };
                        let values = defined(&target_exports.functions, false).into_iter().chain(defined(&target_exports.globals, false));
                        let types = [&target_exports.structs, &target_exports.classes, &target_exports.enums, &target_exports.traits, &target_exports.aliases]
                            .into_iter()
                            .flat_map(|names| defined(names, true));
                        exports.re_exports.extend(values.chain(types));
                        exports.re_exports.extend(target_exports.re_exports.clone());
                        continue;
                    }
                    for sym in &imp.symbols {
                        let exported_name = sym.alias.as_ref().unwrap_or(&sym.name).clone();
                        let source = if target_exports.functions.contains(&sym.name)
                            || target_exports.globals.contains(&sym.name)
                        {
                            Some(ReExportSource {
                                defining_module: target_id.clone(),
                                defining_name: sym.name.clone(),
                                is_type: false,
                            })
                        } else if target_exports.structs.contains(&sym.name)
                            || target_exports.classes.contains(&sym.name)
                            || target_exports.enums.contains(&sym.name)
                            || target_exports.traits.contains(&sym.name)
                            || target_exports.aliases.contains(&sym.name)
                        {
                            Some(ReExportSource {
                                defining_module: target_id.clone(),
                                defining_name: sym.name.clone(),
                                is_type: true,
                            })
                        } else {
                            target_exports.re_exports.get(&sym.name).cloned()
                        };
                        match source {
                            Some(src) => {
                                exports.re_exports.insert(exported_name, src);
                            }
                            None => {
                                return Err(format!(
                                    "'{}' cannot be re-exported from module '{}': it is not `pub` there (via `pub import` in '{}')",
                                    sym.name, target_id.display_path(), id.display_path()
                                ));
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        self.module_exports.insert(id.clone(), exports);
        Ok(())
    }

    /// The collected `pub` exports of a module, if it has been registered.
    pub(crate) fn exports_of(&self, module_id: &CanonicalModuleId) -> Option<&ModuleExports> {
        self.module_exports.get(module_id)
    }

    /// Validates if a symbol from a target module is publicly accessible.
    pub fn is_symbol_exported(&self, module_id: &CanonicalModuleId, symbol: &str) -> bool {
        if let Some(exports) = self.module_exports.get(module_id) {
            exports.functions.contains(symbol)
                || exports.structs.contains(symbol)
                || exports.classes.contains(symbol)
                || exports.enums.contains(symbol)
                || exports.traits.contains(symbol)
                || exports.aliases.contains(symbol)
                || exports.globals.contains(symbol)
                || exports.re_exports.contains_key(symbol)
        } else {
            false
        }
    }

    /// Checks that all selective and qualified imports in the program only access `pub` symbols.
    pub fn check_import_visibility(
        &self,
        target_id: &CanonicalModuleId,
        symbol_name: &str,
    ) -> Result<(), String> {
        if !self.is_symbol_exported(target_id, symbol_name) {
            return Err(format!(
                "Cannot import private symbol '{}' from module '{}'. Mark it with 'pub' to export.",
                symbol_name, target_id.display_path()
            ));
        }
        Ok(())
    }
}
