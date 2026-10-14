//! The module system's side of the modifier pass: finding a trait's modifier and running it.

use std::collections::HashMap;
use std::path::PathBuf;

use compiler::ast::{Item, ModulePath, Program};
use compiler::modifiers::{Additions, Diagnostic, Generator, Host};
use compiler::span::Span;

use crate::path::CanonicalModuleId;
use crate::resolver::ModuleResolver;
use crate::stage0::Stage0;

/// A trait that names a modifier.
struct TraitInfo {
    name: String,
    /// What `@generate` wrote.
    generate: String,
    methods: Vec<String>,
    is_pub: bool,
}

/// Where each trait's modifier is, across every module of the program.
#[derive(Default)]
pub(crate) struct World {
    traits: HashMap<CanonicalModuleId, Vec<TraitInfo>>,
    paths: HashMap<CanonicalModuleId, PathBuf>,
    /// The modules each module can see traits of.
    visible: HashMap<CanonicalModuleId, Vec<CanonicalModuleId>>,
}

impl World {
    pub(crate) fn new<'a>(programs: impl Iterator<Item = (&'a CanonicalModuleId, &'a PathBuf, &'a Program)>) -> World {
        let mut world = World::default();
        for (id, path, program) in programs {
            world.paths.insert(id.clone(), path.clone());
            for item in &program.items {
                let Item::Trait(t) = item else { continue };
                let Some(generate) = t.attrs.iter().find(|a| a.name == "generate").and_then(|a| a.args.first()) else { continue };
                let methods = t.members.iter().filter(|m| m.default_body.is_none()).map(|m| m.name.clone()).collect();
                world.traits.entry(id.clone()).or_default().push(TraitInfo { name: t.name.clone(), generate: generate.clone(), methods, is_pub: t.is_pub });
            }
        }
        world
    }
}

struct Spec {
    trait_module: CanonicalModuleId,
    generate: String,
}

/// The stage-0 programs opened so far, shared by every module of a compile.
#[derive(Default)]
pub(crate) struct Sessions {
    specs: Vec<Spec>,
    runners: HashMap<(CanonicalModuleId, String), Result<Stage0, String>>,
}

/// The host for one module's expansion.
pub(crate) struct ModuleHost<'a> {
    pub world: &'a mut World,
    pub sessions: &'a mut Sessions,
    pub resolver: &'a mut ModuleResolver,
    pub module: CanonicalModuleId,
    /// The module's own imports.
    pub imports: Vec<ModulePath>,
    pub dir: PathBuf,
    /// Inside a stage-0 compile, where no module may ask for a modifier.
    pub stage0: bool,
}

impl ModuleHost<'_> {
    /// The modules whose traits this module can name.
    fn visible(&mut self) -> Result<Vec<CanonicalModuleId>, String> {
        if let Some(v) = self.world.visible.get(&self.module) {
            return Ok(v.clone());
        }
        let mut ids = vec![self.module.clone()];
        for path in &self.imports {
            let (id, _) = self.resolver.resolve_path(path, &self.dir)?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        self.world.visible.insert(self.module.clone(), ids.clone());
        Ok(ids)
    }
}

/// The `std` module name an embedded id stands for.
fn std_name(id: &CanonicalModuleId) -> String {
    id.as_str().strip_prefix("<std>/").unwrap_or("").to_string()
}

impl Host for ModuleHost<'_> {
    fn find(&mut self, name: &str, at: Span) -> Result<Option<Generator>, Diagnostic> {
        let wanted = name.rsplit('.').next().unwrap_or(name);
        let mut hits = Vec::new();
        if self.world.traits.values().any(|ts| ts.iter().any(|t| t.name == wanted)) {
            let visible = self.visible().map_err(|e| (e, at))?;
            for id in &visible {
                for t in self.world.traits.get(id).into_iter().flatten() {
                    if t.name == wanted && (*id == self.module || t.is_pub) {
                        hits.push((id.clone(), t.generate.clone(), t.methods.clone()));
                    }
                }
            }
        }
        match hits.len() {
            0 => Ok(None),
            1 if self.stage0 => Err((format!("a module a modifier uses cannot ask for `{wanted}`"), at)),
            1 => {
                let (module, generate, methods) = hits.pop().expect("one hit");
                let id = self.sessions.specs.len();
                let std = std_name(&module);
                self.sessions.specs.push(Spec { trait_module: module, generate });
                Ok(Some(Generator { trait_name: wanted.to_string(), module: std, methods, id }))
            }
            _ => Err((format!("`{wanted}` names more than one trait with a modifier here"), at)),
        }
    }

    fn run(&mut self, generator: &Generator, decl_json: &str) -> Result<Additions, String> {
        match self.runner(generator)? {
            Ok(runner) => runner.apply(decl_json),
            Err(e) => Err(e.clone()),
        }
    }

    fn reads(&mut self, generator: &Generator) -> Vec<String> {
        match self.runner(generator) {
            Ok(Ok(runner)) => runner.reads().to_vec(),
            _ => Vec::new(),
        }
    }
}

impl ModuleHost<'_> {
    /// The stage-0 runner for a trait's modifier, opened on first use.
    fn runner(&mut self, generator: &Generator) -> Result<&mut Result<Stage0, String>, String> {
        let (trait_module, generate) = {
            let spec = &self.sessions.specs[generator.id];
            (spec.trait_module.clone(), spec.generate.clone())
        };
        let segments: Vec<String> = generate.split('.').map(String::from).collect();
        let (function, module_segments) = segments.split_last().expect("split gives one piece");
        let (module_id, module_path) = if module_segments.is_empty() {
            (trait_module.clone(), self.world.paths.get(&trait_module).cloned().unwrap_or_default())
        } else {
            let path = ModulePath::new(module_segments.to_vec(), false, 0, Span::default());
            self.resolver.resolve_path(&path, &self.dir)?
        };
        let key = (module_id, function.clone());
        if !self.sessions.runners.contains_key(&key) {
            let root = self.resolver.root();
            let cache = self.resolver.cache_dir().map(|d| d.to_path_buf());
            let opened = Stage0::open(&module_path, &root, function, cache.as_deref());
            self.sessions.runners.insert(key.clone(), opened);
        }
        Ok(self.sessions.runners.get_mut(&key).expect("inserted above"))
    }
}
