use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use crate::git::{validate_name, Git, GitRef, Repo};
use crate::installer::{package_checksum, PackageInstaller};
use crate::lockfile::Lockfile;
use crate::manifest::{DependencySpec, PackageManifest, PackageMeta};
use crate::native;
use crate::resolver::{AvailablePackage, DependencyResolver, ResolvedPackage};
use crate::semver::Version;

/// The package commands: new, build, run, package and sync.
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
                mote: Some(crate::manifest::toolchain_version()),
            },
            dependencies: BTreeMap::new(),
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

    /// Adds `<git repository>[@ref]` to the manifest, resolves, and installs it.
    pub fn add_dependency(project_root: &Path, package_spec_str: &str) -> Result<(), String> {
        let manifest_path = project_root.join("mote.toml");
        let mut manifest = PackageManifest::from_file(&manifest_path)?;

        let (repo_spec, at) = split_ref(package_spec_str);
        let repo = Repo::parse(repo_spec).map_err(|e| format!("{e}; `mote add` takes <repository>[@ref], and a local package is a path under [dependencies]"))?;
        let git = Git;
        let git_ref = match at {
            Some(at) => git.classify(&repo, at)?,
            None => GitRef::Tag(git.latest_tag(&repo)?),
        };
        let commit = git.resolve(&repo, &git_ref)?;
        let entries = crate::store::fetch_files(&git, &repo, &commit)?;
        let toml = entries.iter().find(|(p, _)| p == "mote.toml").ok_or_else(|| format!("{} has no mote.toml", repo.spec()))?;
        let name = PackageManifest::from_toml_str(&String::from_utf8_lossy(&toml.1)).map_err(|e| format!("{}: {e}", repo.spec()))?.package.name;
        validate_name(&name)?;

        let original = fs::read(&manifest_path).map_err(|e| format!("Failed to read mote.toml: {e}"))?;
        manifest.dependencies.insert(name, DependencySpec::from_git(&repo, &git_ref));
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

    /// `mote sync`: resolves all declared dependencies, writes the lockfile, and puts the packages in `.mote_packages/`.
    pub fn install_dependencies(project_root: &Path, locked: bool) -> Result<Vec<ResolvedPackage>, String> {
        let manifest_path = project_root.join("mote.toml");
        let manifest = PackageManifest::from_file(&manifest_path)?;
        let lockfile_path = project_root.join("mote.lock");
        let old = if lockfile_path.exists() { Some(Lockfile::from_file(&lockfile_path)?) } else { None };
        let locked_entry = |name: &str, source_prefix: &str| {
            old.iter().flat_map(|o| &o.packages).find(|q| q.name == name && q.source.starts_with(source_prefix))
        };

        let mut solver = DependencyResolver::new();
        let mut git_jobs: Vec<GitJob> = Vec::new();
        for (name, spec) in &manifest.dependencies {
            let git = spec.git()?;
            let Some(local_path) = spec.path() else {
                let Some((repo, git_ref)) = git else {
                    return Err(no_source(name));
                };
                git_jobs.push(GitJob { name: name.clone(), repo, git_ref, required_by: manifest.package.name.clone() });
                continue;
            };
            if git.is_some() {
                return Err(format!("{name}: a dependency is a path or a git repository, not both"));
            }
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

        let installer = PackageInstaller::new(project_root);
        let git = Git;
        let mut pins: HashMap<String, (String, String)> = HashMap::new();
        let mut git_sums: HashMap<String, String> = HashMap::new();
        let mut git_natives: HashMap<String, BTreeMap<String, String>> = HashMap::new();
        while let Some(job) = git_jobs.pop() {
            let name = &job.name;
            let ident = format!("{}?{}", job.repo.spec(), job.git_ref.label());
            if let Some((pinned, by)) = pins.get(name) {
                if *pinned != ident {
                    return Err(format!("{name} is pinned to {pinned} by {by} and to {ident} by {}", job.required_by));
                }
                continue;
            }
            pins.insert(name.clone(), (ident.clone(), job.required_by.clone()));

            let prefix = format!("git+{ident}#");
            let entry = locked_entry(name, &prefix);
            let commit = match entry {
                Some(q) => q.source[prefix.len()..].to_string(),
                None if locked => return Err("--locked: mote.lock is out of date; run `mote sync` to update it".to_string()),
                None => git.resolve(&job.repo, &job.git_ref)?,
            };
            let source = format!("{prefix}{commit}");
            let locked_sum = entry.and_then(|q| q.checksum.as_deref());
            let locked_native = entry.map(|q| q.native.clone()).unwrap_or_default();
            let granted = manifest.dependencies.get(name).is_some_and(|s| s.native());
            let (package, checksum, natives) = match installer.installed_git(name, &source, locked_sum, &locked_native) {
                Some(package) => {
                    let sum = package_checksum(&project_root.join(".mote_packages").join(name))?;
                    (package, sum, locked_native)
                }
                None => {
                    let entries = crate::store::fetch_files(&git, &job.repo, &commit)?;
                    let (package, sum, natives) = installer.check_git(name, &entries)?;
                    if let Some(locked_sum) = locked_sum.filter(|s| *s != sum) {
                        return Err(format!("{name}: the files at {commit} are {sum}, but mote.lock has {locked_sum}; the repository changed"));
                    }
                    if entry.is_some() && natives != locked_native {
                        return Err(format!("{name}: the native files at {commit} differ from mote.lock; the repository changed"));
                    }
                    native::require(name, &natives, granted)?;
                    installer.install_git(name, &source, &entries)?;
                    (package, sum, natives)
                }
            };
            native::require(name, &natives, granted)?;
            git_sums.insert(name.clone(), checksum);
            git_natives.insert(name.clone(), natives);

            let mut dependencies = HashMap::new();
            let by = format!("{name} {}", package.package.version);
            for (dep, spec) in &package.dependencies {
                let Some((repo, git_ref)) = spec.git()? else {
                    return Err(if spec.path().is_some() {
                        format!("{by}: a package fetched from git cannot have the path dependency {dep}")
                    } else {
                        no_source(dep)
                    });
                };
                dependencies.insert(dep.clone(), spec.version_req_str().to_string());
                git_jobs.push(GitJob { name: dep.clone(), repo, git_ref, required_by: by.clone() });
            }
            solver.add_available_package(AvailablePackage { name: name.clone(), version: package.package.version.clone(), dependencies, source });
        }

        let resolved = solver.resolve(&manifest)?;

        let mut lockfile = Lockfile::from_resolved(&resolved);
        for p in &mut lockfile.packages {
            if let Some(dir) = installer.local_source_dir(&p.source) {
                if !dir.join("src").is_dir() {
                    return Err(format!("path dependency '{}': {} is not a directory", p.name, dir.join("src").display()));
                }
                p.checksum = Some(package_checksum(&dir)?);
                p.native = native::dir_checksums(&dir)?;
                let granted = manifest.dependencies.get(&p.name).is_some_and(|s| s.native());
                native::require(&p.name, &p.native, granted)?;
            } else {
                p.checksum = git_sums.get(&p.name).cloned();
                p.native = git_natives.get(&p.name).cloned().unwrap_or_default();
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
                None => return Err("--locked: mote.lock does not exist; run `mote sync` first".to_string()),
                Some(o) if *o != lockfile => {
                    let detail = if changed.is_empty() { String::new() } else { format!(" ({} changed)", changed.join(", ")) };
                    return Err(format!("--locked: mote.lock is out of date{detail}; run `mote sync` to update it"));
                }
                Some(_) => {}
            }
        } else {
            if !changed.is_empty() {
                eprintln!("note: {} changed since mote.lock was written; checksum updated", changed.join(", "));
            }
            lockfile.save_to_file(&lockfile_path)?;
        }

        let local: Vec<_> = resolved.iter().filter(|r| r.source.starts_with("local+")).cloned().collect();
        installer.install(&local)?;

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

}

/// A git dependency waiting to be fetched, and who asked for it.
struct GitJob {
    name: String,
    repo: Repo,
    git_ref: GitRef,
    required_by: String,
}

fn no_source(name: &str) -> String {
    format!("{name}: a dependency needs a source; write {{ git = \"https://host/user/repo\", tag = \"v1.0.0\" }} or {{ path = \"../{name}\" }}")
}

/// `repo@ref` split at the last `@` whose tail has no `/` or `:`, so `git@host:path` keeps its user.
pub(crate) fn split_ref(spec: &str) -> (&str, Option<&str>) {
    match spec.rfind('@') {
        Some(at) if !spec[at + 1..].is_empty() && !spec[at + 1..].contains(['/', ':']) => (&spec[..at], Some(&spec[at + 1..])),
        _ => (spec, None),
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
