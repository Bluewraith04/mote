use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use crate::installer::{package_checksum, PackageInstaller};
use crate::lockfile::Lockfile;
use crate::manifest::{DependencySpec, PackageManifest, PackageMeta};
use crate::registry::Registry;
use crate::resolver::{AvailablePackage, DependencyResolver, ResolvedPackage};
use crate::semver::Version;

/// The package commands: new, build, run, package and install.
pub struct PackageManager;

impl PackageManager {
    /// Scaffolds a new package project.
    pub fn init_project(target_dir: &Path, name: Option<&str>, is_lib: bool) -> Result<(), String> {
        let pkg_name = name.unwrap_or_else(|| {
            target_dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("my-mote-package")
        });

        let entry = if is_lib { "src/lib.mote" } else { "src/main.mote" };
        let manifest = PackageManifest {
            package: PackageMeta {
                name: pkg_name.to_string(),
                version: Version::new(0, 1, 0),
                authors: vec!["Author <author@example.com>".into()],
                edition: "2026".into(),
                entry: entry.to_string(),
            },
            dependencies: BTreeMap::new(),
            registry: None,
            run: None,
        };

        fs::create_dir_all(target_dir.join("src"))
            .map_err(|e| format!("Failed to create src directory: {}", e))?;

        let manifest_path = target_dir.join("mote.toml");
        if !manifest_path.exists() {
            manifest.save_to_file(&manifest_path)?;
        }

        let entry_path = target_dir.join(entry);
        if !entry_path.exists() {
            if is_lib {
                fs::write(
                    &entry_path,
                    format!("// Library: {}\npub fn hello() -> String {{\n    return \"Hello from {}\"\n}}\n", pkg_name, pkg_name),
                )
                .map_err(|e| e.to_string())?;
            } else {
                fs::write(
                    &entry_path,
                    format!("fn main() {{\n    println(\"Hello from {}\")\n}}\n", pkg_name),
                )
                .map_err(|e| e.to_string())?;
            }
        }

        let gitignore = target_dir.join(".gitignore");
        if !gitignore.exists() {
            fs::write(&gitignore, "/dist\n/.build\n.mote_packages\n").ok();
        }

        Ok(())
    }

    /// Adds a dependency to the manifest, resolves, and installs it.
    pub fn add_dependency(project_root: &Path, package_spec_str: &str) -> Result<(), String> {
        let manifest_path = project_root.join("mote.toml");
        let mut manifest = PackageManifest::from_file(&manifest_path)?;

        let (pkg_name, version_req) = match package_spec_str.split_once('@') {
            Some((name, req)) => (name, req.to_string()),
            None => {
                let registry = Registry::required(&manifest, package_spec_str)?;
                let latest = registry
                    .index(package_spec_str)?
                    .into_iter()
                    .filter(|e| !e.yanked)
                    .map(|e| e.version)
                    .max_by_key(|v| (!v.is_prerelease(), v.clone()))
                    .ok_or_else(|| format!("'{package_spec_str}' has no published versions"))?;
                (package_spec_str, format!("^{latest}"))
            }
        };

        let original = fs::read(&manifest_path).map_err(|e| format!("Failed to read mote.toml: {e}"))?;
        manifest
            .dependencies
            .insert(pkg_name.to_string(), DependencySpec::Simple(version_req));
        manifest.save_to_file(&manifest_path)?;

        Self::install_dependencies(project_root, false).map(|_| ()).inspect_err(|_| {
            let _ = fs::write(&manifest_path, &original);
        })
    }

    /// Removes a dependency from manifest, lockfile, and local installation.
    pub fn remove_dependency(project_root: &Path, pkg_name: &str) -> Result<(), String> {
        let manifest_path = project_root.join("mote.toml");
        let mut manifest = PackageManifest::from_file(&manifest_path)?;

        manifest.dependencies.remove(pkg_name);
        manifest.save_to_file(&manifest_path)?;

        let pkg_dir = project_root.join(".mote_packages").join(pkg_name);
        if pkg_dir.exists() {
            fs::remove_dir_all(&pkg_dir).ok();
        }

        Self::install_dependencies(project_root, false).map(|_| ())
    }

    /// Resolves all declared dependencies, writes lockfile, and installs packages into.mote_packages/.
    pub fn install_dependencies(project_root: &Path, locked: bool) -> Result<Vec<ResolvedPackage>, String> {
        let manifest_path = project_root.join("mote.toml");
        let manifest = PackageManifest::from_file(&manifest_path)?;
        let lockfile_path = project_root.join("mote.lock");
        let old = if lockfile_path.exists() { Some(Lockfile::from_file(&lockfile_path)?) } else { None };
        let locked_entry = |name: &str, source: &str| {
            old.iter().flat_map(|o| &o.packages).find(|q| q.name == name && q.source == source)
        };

        let mut solver = DependencyResolver::new();
        let mut pending = Vec::new();
        for (name, spec) in &manifest.dependencies {
            let Some(local_path) = spec.path() else {
                pending.push(name.clone());
                continue;
            };
            let local_manifest_path = project_root.join(local_path).join("mote.toml");
            let version = PackageManifest::from_file(&local_manifest_path)
                .map(|lm| lm.package.version)
                .unwrap_or_else(|_| Version::new(1, 0, 0));
            solver.add_available_package(AvailablePackage {
                name: name.clone(),
                version,
                dependencies: HashMap::new(),
                source: format!("local+{}", local_path),
            });
        }

        let registry = match pending.first() {
            Some(first) => Some(Registry::required(&manifest, first)?),
            None => None,
        };
        let mut checksums: HashMap<(String, Version), String> = HashMap::new();
        if let Some(registry) = &registry {
            let source = registry.source();
            let mut seen = HashSet::new();
            while let Some(name) = pending.pop() {
                if manifest.dependencies.get(&name).is_some_and(|s| s.path().is_some()) || !seen.insert(name.clone()) {
                    continue;
                }
                let pinned = locked_entry(&name, &source).map(|q| q.version.clone());
                for entry in registry.index(&name)? {
                    if entry.yanked && pinned.as_ref() != Some(&entry.version) {
                        continue;
                    }
                    pending.extend(entry.dependencies.keys().cloned());
                    checksums.insert((name.clone(), entry.version.clone()), entry.checksum);
                    solver.add_available_package(AvailablePackage {
                        name: name.clone(),
                        version: entry.version,
                        dependencies: entry.dependencies.into_iter().collect(),
                        source: source.clone(),
                    });
                }
                if let Some(v) = pinned {
                    solver.prefer(&name, v);
                }
            }
        }

        let resolved = solver.resolve(&manifest)?;
        let installer = PackageInstaller::new(project_root);

        let mut lockfile = Lockfile::from_resolved(&resolved);
        for p in &mut lockfile.packages {
            if let Some(dir) = installer.local_source_dir(&p.source) {
                if !dir.join("src").is_dir() {
                    return Err(format!("path dependency '{}': {} is not a directory", p.name, dir.join("src").display()));
                }
                p.checksum = Some(package_checksum(&dir)?);
            } else if let Some(sum) = checksums.get(&(p.name.clone(), p.version.clone())) {
                if let Some(q) = locked_entry(&p.name, &p.source).filter(|q| q.version == p.version && q.checksum.as_ref() != Some(sum)) {
                    return Err(format!(
                        "{} {}: the registry lists checksum {sum}, but mote.lock has {}; the published archive changed",
                        p.name,
                        p.version,
                        q.checksum.as_deref().unwrap_or("none")
                    ));
                }
                p.checksum = Some(sum.clone());
            }
        }
        let changed: Vec<&str> = lockfile
            .packages
            .iter()
            .filter(|p| p.source.starts_with("local+"))
            .filter(|p| {
                old.iter().flat_map(|o| &o.packages).any(|q| q.name == p.name && q.checksum.is_some() && q.checksum != p.checksum)
            })
            .map(|p| p.name.as_str())
            .collect();
        if locked {
            match &old {
                None => return Err("--locked: mote.lock does not exist; run `mote install` first".to_string()),
                Some(o) if *o != lockfile => {
                    let detail = if changed.is_empty() { String::new() } else { format!(" ({} changed)", changed.join(", ")) };
                    return Err(format!("--locked: mote.lock is out of date{detail}; run `mote install` to update it"));
                }
                Some(_) => {}
            }
        } else {
            if !changed.is_empty() {
                eprintln!("note: {} changed since mote.lock was written; checksum updated", changed.join(", "));
            }
            lockfile.save_to_file(&lockfile_path)?;
        }

        let (local, fetched): (Vec<_>, Vec<_>) = resolved.iter().cloned().partition(|r| r.source.starts_with("local+"));
        installer.install(&local)?;
        for p in &fetched {
            let (Some(registry), Some(sum)) = (&registry, checksums.get(&(p.name.clone(), p.version.clone()))) else {
                continue;
            };
            if !installer.has_archive(&p.name, sum) {
                let bytes = registry.archive(&p.name, &p.version)?;
                installer.install_archive(&p.name, &p.version, sum, &bytes)?;
            }
        }

        Ok(resolved)
    }

    /// `mote compile`: the project source into `dist/<name>.mbc`.
    pub fn compile(project_root: &Path) -> Result<PathBuf, String> {
        Self::compile_program(project_root).map(|(path, _)| path)
    }

    /// Like [`compile`](Self::compile), but also returns the compiled program.
    pub fn compile_program(
        project_root: &Path,
    ) -> Result<(PathBuf, compiler::CompiledProgram), String> {
        let manifest_path = project_root.join("mote.toml");
        let manifest = PackageManifest::from_file(&manifest_path)?;

        let entry_path = project_root.join(&manifest.package.entry);
        if !entry_path.is_file() {
            return Err(format!(
                "entry '{}' from {}/mote.toml does not exist",
                manifest.package.entry,
                project_root.display()
            ));
        }
        let dist_dir = project_root.join("dist");
        fs::create_dir_all(&dist_dir).map_err(|e| format!("Failed to create dist directory: {}", e))?;

        let mut compiler = modules::MultiFileCompiler::new(project_root.to_path_buf()).with_cache(project_root.join(".build"));
        let compiled = compiler.compile_program(&entry_path)?;

        let output_file = dist_dir.join(format!("{}.mbc", manifest.package.name));
        crate::mbc::MbcFile::write(&compiled, &output_file)?;

        Ok((output_file, compiled))
    }

    /// `mote package`: `dist/<name>-<version>.mpk` holding `mote.toml`, `<name>.mbc`, the `src/` tree,
    /// and `interface.txt`.
    pub fn package(project_root: &Path) -> Result<PathBuf, String> {
        let manifest = PackageManifest::from_file(&project_root.join("mote.toml"))?;
        let (_mbc, compiled) = Self::compile_program(project_root)?;
        let name = &manifest.package.name;

        let read = |p: &Path| fs::read(p).map_err(|e| format!("Failed to read '{}': {}", p.display(), e));
        let mut entries = vec![
            ("mote.toml".to_string(), read(&project_root.join("mote.toml"))?),
            (format!("{name}.mbc"), crate::mbc::MbcFile::encode(&compiled)),
        ];
        let mut sources = Vec::new();
        collect_sources(&project_root.join("src"), "src", &mut sources)?;
        sources.sort();
        let mut interface = String::new();
        for rel in &sources {
            let bytes = read(&project_root.join(rel))?;
            if rel.ends_with(".mote") {
                let text = String::from_utf8_lossy(&bytes);
                let tokens = compiler::lexer::Lexer::new(&text)
                    .tokenize()
                    .map_err(|(e, s)| format!("{rel}:{}:{}: {e}", s.line, s.col))?;
                let program = compiler::parser::Parser::new(tokens)
                    .parse()
                    .map_err(|(e, s)| format!("{rel}:{}:{}: {e}", s.line, s.col))?;
                interface.push_str(&format!("// {rel}\n{}", compiler::stable::interface(&program)));
            }
            entries.push((rel.clone(), bytes));
        }
        entries.push(("interface.txt".to_string(), interface.into_bytes()));

        let out = project_root.join("dist").join(format!("{name}-{}.mpk", manifest.package.version));
        crate::mpk::MpkArchive::write(&entries, &out)?;
        Ok(out)
    }

    /// Not implemented: there is no package registry to publish to.
    pub fn publish(_project_root: &Path) -> Result<String, String> {
        Err("mote publish: no package registry is implemented".to_string())
    }
}

fn collect_sources(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| format!("Failed to read '{}': {}", dir.display(), e))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let rel = format!("{prefix}/{}", entry.file_name().to_string_lossy());
        if entry.path().is_dir() {
            collect_sources(&entry.path(), &rel, out)?;
        } else {
            out.push(rel);
        }
    }
    Ok(())
}
