//! Per-module symbol tables and name mangling.

use std::collections::{HashMap, HashSet};

use compiler::ast::Program;

use crate::path::CanonicalModuleId;
use crate::visibility::ModuleExports;

/// The implicit prelude's module id.
const PRELUDE: &str = "<std>/prelude";

/// The std module a builtin type must have imported, if it needs one.
pub(crate) fn gated_module(type_name: &str) -> Option<&'static str> {
    match type_name {
        "Set" => Some("collections"),
        "Stream" => Some("stream"),
        "Task" => Some("task"),
        _ => None,
    }
}

fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// The mangle prefix of a module, a pure function of its canonical id.
pub(crate) fn mangle_prefix(module_id: &CanonicalModuleId, wide: bool) -> String {
    let sanitized: String = module_id
        .display_name()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    let hash = fnv1a(module_id.as_str());
    if wide {
        format!("_m{hash:016x}_{sanitized}_")
    } else {
        format!("_m{:08x}_{sanitized}_", hash >> 32)
    }
}

/// Every module's mangle prefix.
pub fn module_prefixes(ids: &[CanonicalModuleId], entry: &CanonicalModuleId) -> Result<HashMap<CanonicalModuleId, String>, String> {
    let short: Vec<String> = ids.iter().map(|id| mangle_prefix(id, false)).collect();
    let mut out = HashMap::new();
    for (id, prefix) in ids.iter().zip(&short) {
        let prefix = if id == entry {
            String::new()
        } else if short.iter().filter(|p| *p == prefix).count() > 1 {
            mangle_prefix(id, true)
        } else {
            prefix.clone()
        };
        out.insert(id.clone(), prefix);
    }
    let mut seen: HashMap<&str, &CanonicalModuleId> = HashMap::new();
    for (id, prefix) in out.iter().filter(|(_, p)| !p.is_empty()) {
        if let Some(other) = seen.insert(prefix.as_str(), id) {
            return Err(format!(
                "modules '{}' and '{}' mangle to the same prefix '{prefix}'",
                other.display_path(), id.display_path()
            ));
        }
    }
    Ok(out)
}

/// `message` with every module prefix removed.
pub(crate) fn demangle(message: &str, prefixes: &std::collections::HashMap<CanonicalModuleId, String>) -> String {
    prefixes.values().filter(|p| !p.is_empty()).fold(message.to_string(), |m, p| m.replace(p.as_str(), ""))
}

/// A module imported under an alias, for resolving qualified calls.
#[derive(Clone, Debug)]
pub struct AliasTarget {
    pub module: CanonicalModuleId,
    pub prefix: String,
    pub exports: ModuleExports,
    /// Member to mangled name for re-exports defined in another module.
    pub reexport_mangled: HashMap<String, String>,
}

impl AliasTarget {
    /// The mangled name of `member` if this module exports it with `pub`.
    pub fn resolve(&self, member: &str) -> Option<String> {
        if self.exports.functions.contains(member)
            || self.exports.globals.contains(member)
            || self.exports.structs.contains(member)
            || self.exports.classes.contains(member)
            || self.exports.traits.contains(member)
            || self.exports.aliases.contains(member)
            || self.exports.enums.contains(member)
        {
            return Some(format!("{}{}", self.prefix, member));
        }
        self.reexport_mangled.get(member).cloned()
    }

    /// Whether `member` is named by the module at all, public or not.
    pub fn names(&self, member: &str) -> bool {
        self.resolve(member).is_some()
    }
}

/// The name environment for one module: what its bare identifiers resolve to.
#[derive(Debug)]
pub struct ModuleScope {
    pub module: CanonicalModuleId,
    pub prefix: String,
    value_bindings: HashMap<String, String>,
    type_bindings: HashMap<String, String>,
    aliases: HashMap<String, AliasTarget>,
    glob_from: HashMap<String, CanonicalModuleId>,
    imported: HashSet<String>,
}

impl ModuleScope {
    pub fn new(module: CanonicalModuleId, prefix: String) -> Self {
        Self {
            module,
            prefix,
            value_bindings: HashMap::new(),
            type_bindings: HashMap::new(),
            aliases: HashMap::new(),
            glob_from: HashMap::new(),
            imported: HashSet::new(),
        }
    }

    /// The mangled name a bare definition in this module takes.
    pub fn mangled(&self, bare: &str) -> String {
        format!("{}{}", self.prefix, bare)
    }

    /// Register one of this module's own top-level function definitions.
    pub(crate) fn define_fn(&mut self, bare: &str) {
        self.value_bindings
            .insert(bare.to_string(), self.mangled(bare));
    }

    /// Registers one of this module's top-level globals.
    pub(crate) fn define_global(&mut self, bare: &str) {
        self.value_bindings
            .insert(bare.to_string(), self.mangled(bare));
    }

    /// Registers one of this module's top-level struct or class definitions.
    pub(crate) fn define_type(&mut self, bare: &str) {
        let m = self.mangled(bare);
        self.type_bindings.insert(bare.to_string(), m.clone());
        self.value_bindings.insert(bare.to_string(), m);
    }

    /// Binds a name from a selective import, optionally aliased.
    pub(crate) fn bind_import(
        &mut self,
        local: &str,
        mangled: String,
        as_value: bool,
        as_type: bool,
    ) -> Result<(), String> {
        if (self.value_bindings.contains_key(local) || self.type_bindings.contains_key(local))
            && !self.glob_from.contains_key(local) {
                return Err(format!(
                    "`{}` is imported into module '{}' but is already defined there",
                    local, self.module.display_path()
                ));
            }
        if as_value {
            self.value_bindings.insert(local.to_string(), mangled.clone());
        }
        if as_type {
            self.type_bindings.insert(local.to_string(), mangled);
        }
        self.glob_from.remove(local);
        Ok(())
    }

    /// Bind a name brought in by a bare `import.m` glob.
    pub(crate) fn bind_glob(
        &mut self,
        name: &str,
        from: &CanonicalModuleId,
        mangled: String,
        as_value: bool,
        as_type: bool,
    ) -> Result<(), String> {
        if !self.glob_from.contains_key(name)
            && (self.value_bindings.contains_key(name) || self.type_bindings.contains_key(name))
        {
            return Ok(());
        }
        if let Some(other) = self.glob_from.get(name) {
            if from.as_str() == PRELUDE {
                return Ok(());
            }
            let same_value = !as_value || self.value_bindings.get(name) == Some(&mangled);
            let same_type = !as_type || self.type_bindings.get(name) == Some(&mangled);
            if same_value && same_type {
                return Ok(());
            }
            if other != from && other.as_str() != PRELUDE {
                return Err(format!(
                    "`{}` is glob-imported into module '{}' from both '{}' and '{}' — import it explicitly to disambiguate",
                    name, self.module.display_path(), other.display_path(), from.display_path()
                ));
            }
        }
        if as_value {
            self.value_bindings.insert(name.to_string(), mangled.clone());
        }
        if as_type {
            self.type_bindings.insert(name.to_string(), mangled);
        }
        self.glob_from.insert(name.to_string(), from.clone());
        Ok(())
    }

    /// Records that this module imports `id` in some form.
    pub(crate) fn note_import(&mut self, id: &CanonicalModuleId) {
        self.imported.insert(id.as_str().to_string());
    }

    /// Whether this module is, or imports, the embedded `std` module `name`.
    pub(crate) fn has_std_module(&self, name: &str) -> bool {
        let id = crate::embedded::embedded_id(name);
        self.module.as_str() == id || self.imported.contains(&id)
    }

    /// Registers a module alias import.
    pub(crate) fn bind_alias(&mut self, alias: &str, target: AliasTarget) {
        self.aliases.insert(alias.to_string(), target);
    }

    pub(crate) fn resolve_value(&self, name: &str) -> Option<&str> {
        self.value_bindings.get(name).map(String::as_str)
    }

    pub(crate) fn resolve_type(&self, name: &str) -> Option<&str> {
        self.type_bindings.get(name).map(String::as_str)
    }

    pub fn alias(&self, name: &str) -> Option<&AliasTarget> {
        self.aliases.get(name)
    }
}

/// Collects a module's own top-level definitions and globals into `scope`.
pub(crate) fn register_own_definitions(scope: &mut ModuleScope, program: &Program) {
    use compiler::ast::{Item, Stmt};
    for item in &program.items {
        match item {
            Item::Function(f) => scope.define_fn(&f.name),
        Item::NativeFunction(f) => scope.define_fn(&f.name),
            Item::Struct(s) => scope.define_type(&s.name),
            Item::Class(c) => scope.define_type(&c.name),
            Item::Enum(e) => scope.define_type(&e.name),
            Item::Trait(t) => scope.define_type(&t.name),
            Item::TypeAlias(a) => scope.define_type(&a.name),
            Item::TopLevelStmt(Stmt::Let { name, .. } | Stmt::Var { name, .. }) => {
                scope.define_global(name)
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> CanonicalModuleId {
        CanonicalModuleId::new(s.to_string())
    }

    #[test]
    fn prefixes_depend_only_on_the_module_id() {
        let (entry, a, b) = (id("/p/main.mote"), id("/p/util.mote"), id("/p/sub/util.mote"));
        let one = module_prefixes(&[a.clone(), b.clone(), entry.clone()], &entry).unwrap();
        let two = module_prefixes(&[b.clone(), id("/p/extra.mote"), a.clone(), entry.clone()], &entry).unwrap();
        assert_eq!(one[&a], two[&a]);
        assert_eq!(one[&b], two[&b]);
        assert_ne!(one[&a], one[&b], "same stem, different paths");
        assert_eq!(one[&entry], "");
        assert!(one[&a].starts_with("_m") && one[&a].ends_with("_util_") && one[&a].len() == "_m12345678_util_".len());
    }

    #[test]
    fn colliding_short_prefixes_are_widened() {
        let mut seen: HashMap<String, String> = HashMap::new();
        let pair = (0..400_000)
            .map(|i| format!("/c/{i}/m.mote"))
            .find_map(|p| seen.insert(mangle_prefix(&id(&p), false), p.clone()).map(|q| (q, p)))
            .expect("a 32-bit collision among 400k ids");
        let (a, b) = (id(&pair.0), id(&pair.1));
        let entry = id("/c/main.mote");
        let prefixes = module_prefixes(&[a.clone(), b.clone(), entry.clone()], &entry).unwrap();
        assert_ne!(prefixes[&a], prefixes[&b]);
        assert_eq!(prefixes[&a], mangle_prefix(&a, true));
    }
}
