use std::collections::HashMap;
use std::path::{Path, PathBuf};
use compiler::ast::*;
use compiler::codegen::{CodeGenerator, CompiledProgram};
use compiler::type_check::TypeChecker;
use compiler::DiagnosticFormatter;

use crate::graph::DependencyGraph;
use crate::path::CanonicalModuleId;
use crate::resolver::ModuleResolver;
use crate::symbols::{demangle, module_prefixes, register_own_definitions, AliasTarget, ModuleScope};
use crate::rewrite::rewrite_program;
use crate::visibility::VisibilityChecker;

const TEST_FILTER_FN: &str = "mote_test_filter";
const TEST_FILTER_SRC: &str = "
import std.regex as mote_test_filter_rx
import std.sys.process as mote_test_filter_proc
fn mote_test_filter(pattern: String, name: String) -> Bool {
    match mote_test_filter_rx.Regex.compile(pattern) {
        Ok(re) => { return re.is_match(name) }
        Err(e) => {
            println(\"error: mote test --filter: ${e.message}\")
            mote_test_filter_proc.exit(2)
            return false
        }
    }
}
";

fn member_path(segments: &[String], imp: &ImportDecl) -> ModulePath {
    ModulePath::new(segments.to_vec(), false, 0, imp.path.span)
}

fn alias_target(
    visibility: &VisibilityChecker,
    prefixes: &HashMap<CanonicalModuleId, String>,
    id: &CanonicalModuleId,
) -> Result<AliasTarget, String> {
    let prefix = prefixes.get(id).cloned().ok_or_else(|| format!("module '{}' was not in the compile graph", id.display_path()))?;
    let exports = visibility.exports_of(id).cloned().unwrap_or_default();
    let reexport_mangled = exports
        .re_exports
        .iter()
        .filter_map(|(name, src)| prefixes.get(&src.defining_module).map(|p| (name.clone(), format!("{}{}", p, src.defining_name))))
        .collect();
    Ok(AliasTarget { module: id.clone(), prefix, exports, reexport_mangled })
}

/// Compiles an entry file and every module it imports.
pub struct MultiFileCompiler {
    resolver: ModuleResolver,
    visibility: VisibilityChecker,
    memory_report: bool,
}

impl MultiFileCompiler {
    pub fn new(root_dir: PathBuf) -> Self {
        Self {
            resolver: ModuleResolver::new(root_dir),
            visibility: VisibilityChecker::new(),
            memory_report: false,
        }
    }

    /// Fills `CompiledProgram::memory_sites` (`mote check --memory`).
    pub fn with_memory_report(mut self) -> Self {
        self.memory_report = true;
        self
    }

    /// Keeps parsed modules under `dir/modules`, reused while a file's text is unchanged.
    pub fn with_cache(mut self, dir: PathBuf) -> Self {
        self.resolver.set_cache_dir(dir);
        self
    }

    /// Compiles a multi-file program starting from an entry source file.
    pub fn compile_program(&mut self, entry_path: &Path) -> Result<CompiledProgram, String> {
        self.compile_program_impl(entry_path, false, None)
    }

    /// `mote test`: compiles `entry_path` with its `test { }` and `@test` declarations wired in as the harness.
    /// `filter`, when given, is a `std.regex` pattern each test name is matched against at run time.
    pub fn compile_program_for_test(
        &mut self,
        entry_path: &Path,
        filter: Option<&str>,
    ) -> Result<CompiledProgram, String> {
        self.compile_program_impl(entry_path, true, filter)
    }

    fn compile_program_impl(
        &mut self,
        entry_path: &Path,
        want_tests: bool,
        filter: Option<&str>,
    ) -> Result<CompiledProgram, String> {
        if want_tests && filter.is_some() {
            let entry_id = CanonicalModuleId::new(entry_path.canonicalize().unwrap_or(entry_path.to_path_buf()).to_string_lossy().to_string());
            self.resolver.append_source(&entry_id, entry_path, TEST_FILTER_SRC, "<mote test --filter>")?;
        }
        let graph = DependencyGraph::build(entry_path.to_path_buf(), &mut self.resolver)?;
        let compile_order = graph.topological_sort()?;

        let mut programs: Vec<(CanonicalModuleId, Program)> = Vec::new();
        for mod_id in &compile_order {
            if let Some(node) = graph.nodes.get(mod_id) {
                let path = &node.target;
                let prog = self.resolver.parse_module(mod_id, path)?;
                if let Some(mark) = prog.stable_marks.first() {
                    if !crate::embedded::is_embedded_id(&mod_id.0) {
                        return Err(format!(
                            "'{}' in module '{}' is marked `@stable`, but `@stable` is only legal inside `std`",
                            mark.name, mod_id.display_path()
                        ));
                    }
                }
                let module_dir = path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
                self.visibility
                    .register_module_exports(mod_id, &prog, &mut self.resolver, &module_dir)?;
                programs.push((mod_id.clone(), prog));
            }
        }

        let ids: Vec<CanonicalModuleId> = programs.iter().map(|(id, _)| id.clone()).collect();
        let prefixes = module_prefixes(&ids, &graph.entry_id)?;

        for (mod_id, prog) in &mut programs {
            let Some(node) = graph.nodes.get(mod_id) else { continue };
            let from_dir = node.target.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();

            let mut scope = ModuleScope::new(mod_id.clone(), prefixes[mod_id].clone());
            register_own_definitions(&mut scope, prog);

            for item in &prog.items {
                let Item::Import(imp) = item else { continue };
                let (target_id, _target) = self.resolver.resolve_path(&imp.path, &from_dir)?;
                let target_prefix = prefixes
                    .get(&target_id)
                    .cloned()
                    .ok_or_else(|| format!("module '{}' was not in the compile graph", target_id.display_path()))?;
                let exports = self
                    .visibility
                    .exports_of(&target_id)
                    .cloned()
                    .unwrap_or_default();

                if !imp.symbols.is_empty() {
                    for sym in &imp.symbols {
                        let is_fn = exports.functions.contains(&sym.name);
                        let is_enum = exports.enums.contains(&sym.name);
                        let is_type = is_enum
                            || exports.structs.contains(&sym.name)
                            || exports.classes.contains(&sym.name)
                            || exports.traits.contains(&sym.name)
                            || exports.aliases.contains(&sym.name);
                        let is_global = exports.globals.contains(&sym.name);
                        let local = sym.alias.as_ref().unwrap_or(&sym.name);
                        if is_fn || is_type || is_global {
                            let mangled = format!("{}{}", target_prefix, sym.name);
                            scope.bind_import(local, mangled, is_fn || is_global || is_enum, is_type)?;
                        } else if let Some(src) = exports.re_exports.get(&sym.name) {
                            let defining_prefix = prefixes.get(&src.defining_module).cloned().ok_or_else(|| {
                                format!(
                                    "module '{}' was not in the compile graph (re-exported by '{}')",
                                    src.defining_module.display_path(), target_id.display_path()
                                )
                            })?;
                            let mangled = format!("{}{}", defining_prefix, src.defining_name);
                            scope.bind_import(local, mangled, !src.is_type, src.is_type)?;
                        } else if let Some((_, path)) = crate::embedded::group_members(&imp.path.segments).into_iter().find(|(m, _)| *m == sym.name) {
                            let (member_id, _) = self.resolver.resolve_path(&member_path(&path, imp), &from_dir)?;
                            scope.bind_alias(local, alias_target(&self.visibility, &prefixes, &member_id)?);
                        } else {
                            return Err(format!(
                                "Cannot import private symbol '{}' from module '{}'. Mark it with 'pub' to export. (imported by '{}')",
                                sym.name, target_id.display_path(), mod_id.display_path()
                            ));
                        }
                    }
                } else {
                    let reexport_mangled: HashMap<String, String> = exports
                        .re_exports
                        .iter()
                        .filter_map(|(name, src)| {
                            prefixes
                                .get(&src.defining_module)
                                .map(|p| (name.clone(), format!("{}{}", p, src.defining_name)))
                        })
                        .collect();

                    let namespace = imp
                        .alias
                        .clone()
                        .or_else(|| imp.path.segments.last().cloned());
                    if let Some(ns) = namespace {
                        if !imp.path.is_relative {
                            for (member, path) in crate::embedded::group_members(&imp.path.segments) {
                                let (member_id, _) = self.resolver.resolve_path(&member_path(&path, imp), &from_dir)?;
                                scope.bind_alias(&format!("{ns}.{member}"), alias_target(&self.visibility, &prefixes, &member_id)?);
                            }
                        }
                        scope.bind_alias(
                            &ns,
                            AliasTarget {
                                module: target_id.clone(),
                                prefix: target_prefix.clone(),
                                exports: exports.clone(),
                                reexport_mangled: reexport_mangled.clone(),
                            },
                        );
                    }
                    if imp.alias.is_none() {
                        for name in exports.functions.iter().chain(exports.globals.iter()) {
                            let m = format!("{}{}", target_prefix, name);
                            scope.bind_glob(name, &target_id, m, true, false)?;
                        }
                        for name in exports
                            .structs
                            .iter()
                            .chain(exports.classes.iter())
                            .chain(exports.enums.iter())
                            .chain(exports.traits.iter())
                            .chain(exports.aliases.iter())
                        {
                            let m = format!("{}{}", target_prefix, name);
                            scope.bind_glob(name, &target_id, m, true, true)?;
                        }
                        for (name, src) in &exports.re_exports {
                            if let Some(m) = reexport_mangled.get(name) {
                                scope.bind_glob(name, &target_id, m.clone(), true, src.is_type)?;
                            }
                        }
                    }
                }
            }

            rewrite_program(prog, &scope)?;
        }

        let test_marks: Vec<TestMark> = if want_tests {
            programs
                .iter()
                .flat_map(|(mod_id, prog)| {
                    let prefix = prefixes[mod_id].clone();
                    prog.test_marks.iter().map(move |m| TestMark {
                        display_name: m.display_name.clone(),
                        fn_name: format!("{}{}", prefix, m.fn_name),
                        ignored: m.ignored,
                        span: m.span,
                    })
                })
                .collect()
        } else {
            Vec::new()
        };

        let mut refs: Vec<&mut Program> = programs.iter_mut().map(|(_, p)| p).collect();
        compiler::impls::copy_trait_defaults(&mut refs);

        let mut type_checker = TypeChecker::new();
        for (mod_id, prog) in &mut programs {
            type_checker.set_module(&prefixes[mod_id]);
            if let Err(errs) = type_checker.check_program(prog) {
                let source = self.resolver.get_source(mod_id).unwrap_or("");
                let file_name = mod_id.display_path();
                let mut rendered = String::new();
                for (err, span) in errs {
                    rendered.push_str(&DiagnosticFormatter::format_error(source, &file_name, &err, &span));
                }
                return Err(demangle(&rendered, &prefixes));
            }
        }

        let checked = type_checker.finish(programs.into_iter().map(|(_id, prog)| prog).collect());
        let codegen = CodeGenerator::new()
            .with_module_prefixes(ids.iter().map(|id| prefixes[id].clone()).collect());
        let codegen = if self.memory_report { codegen.with_memory_report() } else { codegen };
        let codegen = if want_tests { codegen.with_test_marks(test_marks) } else { codegen };
        let codegen = match filter.filter(|_| want_tests) {
            Some(pattern) => codegen.with_test_filter(format!("{}{TEST_FILTER_FN}", prefixes[&graph.entry_id]), pattern.to_string()),
            None => codegen,
        };
        let mut compiled = codegen
            .compile_linked(checked)
            .map_err(|(err, span)| demangle(&format!("Codegen error at line {}:{}: {}", span.line, span.col, err), &prefixes))?;
        for site in &mut compiled.memory_sites {
            site.type_name = demangle(&site.type_name, &prefixes);
        }

        Ok(compiled)
    }
}
