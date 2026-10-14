use std::collections::HashMap;
use std::path::{Path, PathBuf};
use compiler::ast::*;
use compiler::codegen::{CodeGenerator, CompiledProgram};
use compiler::type_check::TypeChecker;
use compiler::DiagnosticFormatter;

use sha2::{Digest, Sha256};

use crate::graph::DependencyGraph;
use crate::modifier_host::{ModuleHost, Sessions, World};
use crate::path::CanonicalModuleId;
use crate::resolver::ModuleResolver;
use crate::symbols::{demangle, module_prefixes, register_own_definitions, AliasTarget, ModuleScope};
use crate::rewrite::rewrite_program;
use crate::visibility::VisibilityChecker;

const TEST_FILTER_FN: &str = "mote_test_filter";
const TEST_FILTER_SRC: &str = "
import std.regex as mote_test_filter_rx
import std.process as mote_test_filter_proc
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

/// The `std` modules a modifier may use: the ones that touch nothing outside the program.
const STAGE0_STD: &[&str] = &["prelude", "result", "error", "traits", "meta", "string", "iter", "collections", "math"];

/// The entry a stage-0 program runs.
fn stage0_main(modifier_fn: &str) -> String {
    format!(
        "
import std.meta as __stage0_meta
import std.env as __stage0_env
fn main() {{
    println(__stage0_meta.respond(__stage0_env.args().get(1), {modifier_fn}))
}}
"
    )
}

/// A code generation error with its source line.
fn render_codegen_error(err: &str, span: &compiler::span::Span) -> String {
    match compiler::span::source_info(span.source) {
        Some((name, text)) if !text.is_empty() => DiagnosticFormatter::format_error(&text, &name, err, span),
        _ => format!("error: {err}\n --> line {}:{}\n", span.line, span.col),
    }
}

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
    /// Compiling a stage-0 program, where no module may ask for a modifier.
    stage0: bool,
}

impl MultiFileCompiler {
    pub fn new(root_dir: PathBuf) -> Self {
        Self {
            resolver: ModuleResolver::new(root_dir),
            visibility: VisibilityChecker::new(),
            memory_report: false,
            stage0: false,
        }
    }

    /// Fills `CompiledProgram::memory_sites` (`mote check --memory`).
    pub fn with_memory_report(mut self) -> Self {
        self.memory_report = true;
        self
    }

    /// Keeps parsed modules under `dir/modules`.
    pub fn with_cache(mut self, dir: PathBuf) -> Self {
        self.resolver.set_cache_dir(dir);
        self
    }

    /// Compiles a multi-file program starting from an entry source file.
    pub fn compile_program(&mut self, entry_path: &Path) -> Result<CompiledProgram, String> {
        self.compile_program_impl(entry_path, false, None)
    }

    /// Expands every module that asked for a modifier into `(path, source)`.
    pub fn expanded_sources(&mut self, entry_path: &Path) -> Result<Vec<(String, String)>, String> {
        let graph = DependencyGraph::build(entry_path.to_path_buf(), &mut self.resolver)?;
        let order = graph.topological_sort()?;
        let mut programs = self.load_programs(&graph, &order)?;
        self.run_modifiers(&graph, &mut programs)?;
        let mut out = Vec::new();
        for (mod_id, program) in &programs {
            if crate::embedded::is_embedded_id(&mod_id.0) || program.applied.is_empty() {
                continue;
            }
            let source = self.resolver.get_source(mod_id).unwrap_or("");
            out.push((mod_id.display_path(), compiler::modifiers::render(source, &program.applied)));
        }
        Ok(out)
    }

    /// Every module of the graph, parsed, in compile order.
    fn load_programs(&mut self, graph: &DependencyGraph, order: &[CanonicalModuleId]) -> Result<Vec<(CanonicalModuleId, Program)>, String> {
        let mut programs = Vec::new();
        for mod_id in order {
            let Some(node) = graph.nodes.get(mod_id) else { continue };
            let prog = self.resolver.parse_module(mod_id, &node.target)?;
            if let Some(mark) = prog.stable_marks.first() {
                if !crate::embedded::is_embedded_id(&mod_id.0) {
                    return Err(format!(
                        "'{}' in module '{}' is marked `@stable`, but `@stable` is only legal inside `std`",
                        mark.name, mod_id.display_path()
                    ));
                }
            }
            programs.push((mod_id.clone(), prog));
        }
        Ok(programs)
    }

    /// The modifier pass over every module.
    fn run_modifiers(&mut self, graph: &DependencyGraph, programs: &mut [(CanonicalModuleId, Program)]) -> Result<(), String> {
        let mut world = World::new(programs.iter().filter_map(|(id, p)| graph.nodes.get(id).map(|n| (id, &n.target, p))));
        let mut sessions = Sessions::default();
        let stage0 = self.stage0;
        for (id, prog) in programs.iter_mut() {
            let Some(node) = graph.nodes.get(id) else { continue };
            let imports = prog.items.iter().filter_map(|i| if let Item::Import(imp) = i { Some(imp.path.clone()) } else { None }).collect();
            let dir = node.target.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
            let result = {
                let mut host = ModuleHost { world: &mut world, sessions: &mut sessions, resolver: &mut self.resolver, module: id.clone(), imports, dir, stage0 };
                compiler::modifiers::expand(prog, &mut host)
            };
            if let Err((message, span)) = result {
                let source = self.resolver.get_source(id).unwrap_or("");
                return Err(DiagnosticFormatter::format_error(source, &id.display_path(), &message, &span));
            }
        }
        Ok(())
    }

    /// Checks the `@modifier` function `modifier_fn` of `module_path` for stage 0.
    pub(crate) fn stage0_check(&mut self, module_path: &Path, modifier_fn: &str) -> Result<(String, Vec<String>), String> {
        let mut reads = Vec::new();
        let graph = DependencyGraph::build(module_path.to_path_buf(), &mut self.resolver)?;
        let mut sources: Vec<(String, String)> = Vec::new();
        for mod_id in graph.topological_sort()? {
            let Some(node) = graph.nodes.get(&mod_id) else { continue };
            let embedded = crate::embedded::is_embedded_id(&mod_id.0);
            let program = self.resolver.parse_module(&mod_id, &node.target)?;
            let source = self.resolver.get_source(&mod_id).unwrap_or("").to_string();
            let name = mod_id.display_path();
            if !embedded {
                let fail = |message: String, span: &compiler::span::Span| DiagnosticFormatter::format_error(&source, &name, &message, span);
                for item in &program.items {
                    let Item::Import(imp) = item else { continue };
                    if imp.path.is_relative || imp.path.segments.first().map(String::as_str) != Some("std") {
                        continue;
                    }
                    let used = imp.path.segments[1..].join(".");
                    if !STAGE0_STD.contains(&used.as_str()) {
                        return Err(fail(format!("a modifier cannot use `std.{used}`; only pure modules can run at compile time"), &imp.span));
                    }
                }
            }
            if mod_id == graph.entry_id {
                let Some(mark) = program.modifier_marks.iter().find(|m| m.fn_name == modifier_fn) else {
                    return Err(format!("`{modifier_fn}` is not a `@modifier` function of '{name}'"));
                };
                reads = mark.reads.clone();
                if program.items.iter().any(|i| matches!(i, Item::Function(f) if f.name == "main")) {
                    return Err(format!("'{name}' holds the modifier `{modifier_fn}` and defines `main`; move the modifier to a module without one"));
                }
            }
            sources.push((name, source));
        }
        sources.sort();
        let mut hasher = Sha256::new();
        hasher.update(env!("MOTE_FRONTEND_FINGERPRINT"));
        hasher.update([0]);
        hasher.update(modifier_fn);
        for (name, source) in sources {
            hasher.update([0]);
            hasher.update(name);
            hasher.update([0]);
            hasher.update(source);
        }
        Ok((hasher.finalize().iter().map(|b| format!("{b:02x}")).collect(), reads))
    }

    /// Compiles the stage-0 program for `modifier_fn` of `module_path`.
    pub(crate) fn stage0_build(&mut self, module_path: &Path, modifier_fn: &str) -> Result<CompiledProgram, String> {
        self.stage0 = true;
        let graph = DependencyGraph::build(module_path.to_path_buf(), &mut self.resolver)?;
        let entry_id = graph.entry_id.clone();
        self.resolver.append_source(&entry_id, module_path, &stage0_main(modifier_fn), "<stage 0>")?;
        self.compile_program_impl(module_path, false, None)
    }

    /// Checks then builds the stage-0 program.
    pub fn compile_stage0(&mut self, module_path: &Path, modifier_fn: &str) -> Result<CompiledProgram, String> {
        self.stage0_check(module_path, modifier_fn)?;
        self.stage0_build(module_path, modifier_fn)
    }

    /// Compiles `entry_path` with its tests wired in as the harness.
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

        let mut programs = self.load_programs(&graph, &compile_order)?;
        self.run_modifiers(&graph, &mut programs)?;
        for (mod_id, prog) in &programs {
            let Some(node) = graph.nodes.get(mod_id) else { continue };
            let module_dir = node.target.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
            self.visibility.register_module_exports(mod_id, prog, &mut self.resolver, &module_dir)?;
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
                if !imp.alias.as_deref().is_some_and(|a| a.starts_with("__")) {
                    scope.note_import(&target_id);
                    if !imp.path.is_relative {
                        for (_, path) in crate::embedded::group_members(&imp.path.segments) {
                            let (member_id, _) = self.resolver.resolve_path(&member_path(&path, imp), &from_dir)?;
                            scope.note_import(&member_id);
                        }
                    }
                }

                if !imp.symbols.is_empty() {
                    for sym in &imp.symbols {
                        if crate::symbols::gated_module(&sym.name).is_some_and(|m| crate::embedded::embedded_id(m) == target_id.as_str()) {
                            continue;
                        }
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
                    if let Some(ns) = namespace.filter(|_| !imp.glob) {
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
                    if imp.glob {
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
        compiler::field_defaults::fill_field_defaults(&mut refs);

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
            .map_err(|(err, span)| demangle(&render_codegen_error(&err, &span), &prefixes))?;
        for site in &mut compiled.memory_sites {
            site.type_name = demangle(&site.type_name, &prefixes);
        }

        Ok(compiled)
    }
}
