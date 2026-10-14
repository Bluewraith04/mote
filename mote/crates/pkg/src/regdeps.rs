//! Resolving and installing the dependencies that come from a registry.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::config::Registries;
use crate::installer::{tree_checksum, PackageInstaller};
use crate::lockfile::{LockedPackage, Lockfile};
use crate::manifest::PackageManifest;
use crate::mpk::MpkArchive;
use crate::native::{self, archive_checksums};
use crate::registry::{IndexEntry, Registry};
use crate::resolver::{AvailablePackage, DependencyResolver, ResolvedPackage};
use crate::semver::{Version, VersionReq};

/// A requirement on a registry package, and who made it.
struct Job {
    name: String,
    registry: Registry,
    req: String,
    required_by: String,
}

/// What an installed registry package adds to the lock.
pub(crate) struct Installed {
    pub checksum: String,
    pub native: BTreeMap<String, String>,
}

/// The registry side of `mote sync`.
pub(crate) struct RegistryDeps<'a> {
    registries: Registries,
    old: Option<&'a Lockfile>,
    installer: &'a PackageInstaller,
    use_lock: bool,
    locked: bool,
    registry_of: HashMap<String, Registry>,
    indexes: HashMap<String, Vec<IndexEntry>>,
    archives: HashMap<String, Vec<(String, Vec<u8>)>>,
    explored: HashSet<(String, Version)>,
    from_lock: bool,
}

impl<'a> RegistryDeps<'a> {
    /// A registry source for the given lock settings.
    pub fn new(manifest: &PackageManifest, old: Option<&'a Lockfile>, installer: &'a PackageInstaller, use_lock: bool, locked: bool) -> Result<Self, String> {
        Ok(RegistryDeps {
            registries: Registries::for_manifest(manifest)?,
            old,
            installer,
            use_lock,
            locked,
            registry_of: HashMap::new(),
            indexes: HashMap::new(),
            archives: HashMap::new(),
            explored: HashSet::new(),
            from_lock: false,
        })
    }

    /// Whether any version came from the lockfile instead of the registry.
    pub fn used_lock(&self) -> bool {
        self.from_lock
    }

    /// The names of the packages that come from a registry.
    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.registry_of.keys()
    }

    /// Offers the resolver every version the requirements `roots` can reach.
    pub fn explore(&mut self, solver: &mut DependencyResolver, roots: &[(String, Option<String>, String)], root_name: &str) -> Result<(), String> {
        let mut jobs = Vec::new();
        for (name, registry, req) in roots {
            jobs.push(Job { name: name.clone(), registry: self.registries.get(registry.as_deref())?, req: req.clone(), required_by: root_name.to_string() });
        }
        while let Some(job) = jobs.pop() {
            self.visit(solver, job, &mut jobs)?;
        }
        Ok(())
    }

    fn visit(&mut self, solver: &mut DependencyResolver, job: Job, jobs: &mut Vec<Job>) -> Result<(), String> {
        let name = job.name.clone();
        match self.registry_of.get(&name) {
            Some(other) if other.name() != job.registry.name() => {
                return Err(format!("{name} comes from registry `{}` and from `{}`", other.name(), job.registry.name()));
            }
            Some(_) => {}
            None => {
                self.registry_of.insert(name.clone(), job.registry.clone());
            }
        }
        let req = VersionReq::parse(&job.req).map_err(|e| format!("{}: the requirement on {name} is not valid: {e}", job.required_by))?;
        let prefix = format!("registry+{}#", job.registry.url());

        let pinned = self.old.into_iter().flat_map(|o| &o.packages).find(|q| q.name == name && q.source.starts_with(&prefix) && req.matches(&q.version));
        if let Some(pinned) = pinned.filter(|_| self.use_lock) {
            return self.offer_pinned(solver, &job, pinned, jobs);
        }
        if self.locked {
            return Err("--locked: mote.lock is out of date; run `mote sync` to update it".to_string());
        }

        if !self.indexes.contains_key(&name) {
            let index = job.registry.index(&name)?;
            self.indexes.insert(name.clone(), index);
        }
        for entry in self.indexes[&name].iter().filter(|e| !e.yanked && req.matches(&e.version)) {
            if !self.explored.insert((name.clone(), entry.version.clone())) {
                continue;
            }
            let by = format!("{name} {}", entry.version);
            for (dep, dep_req) in &entry.dependencies {
                jobs.push(Job { name: dep.clone(), registry: job.registry.clone(), req: dep_req.clone(), required_by: by.clone() });
            }
            solver.add_available_package(AvailablePackage {
                name: name.clone(),
                version: entry.version.clone(),
                dependencies: entry.dependencies.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                source: format!("{prefix}{}", entry.checksum),
            });
        }
        Ok(())
    }

    /// Offers the version `pinned` by the lockfile.
    fn offer_pinned(&mut self, solver: &mut DependencyResolver, job: &Job, pinned: &LockedPackage, jobs: &mut Vec<Job>) -> Result<(), String> {
        self.from_lock = true;
        if !self.explored.insert((pinned.name.clone(), pinned.version.clone())) {
            return Ok(());
        }
        let manifest = match self.installer.installed_git(&pinned.name, &pinned.source, pinned.checksum.as_deref(), &pinned.native) {
            Some(manifest) => manifest,
            None => {
                let sum = pinned.source.rsplit_once('#').map_or("", |(_, s)| s);
                let bytes = job.registry.archive(&pinned.name, &pinned.version, sum)?;
                let entries = unpack(&bytes, &pinned.name, &pinned.version)?;
                let (manifest, _, _) = check(&pinned.name, &pinned.version, &entries)?;
                self.archives.insert(pinned.name.clone(), entries);
                manifest
            }
        };
        let by = format!("{} {}", pinned.name, pinned.version);
        let mut dependencies = HashMap::new();
        for (dep, spec) in &manifest.dependencies {
            if !spec.is_registry() || spec.registry().is_some_and(|r| r != job.registry.name()) {
                return Err(format!("{by}: a registry package can only depend on packages from its own registry; {dep} is not"));
            }
            dependencies.insert(dep.clone(), spec.version_req_str().to_string());
            jobs.push(Job { name: dep.clone(), registry: job.registry.clone(), req: spec.version_req_str().to_string(), required_by: by.clone() });
        }
        solver.add_available_package(AvailablePackage {
            name: pinned.name.clone(),
            version: pinned.version.clone(),
            dependencies,
            source: pinned.source.clone(),
        });
        Ok(())
    }

    /// Puts each resolved registry package in `.mote_packages/`.
    pub fn install(&mut self, resolved: &[ResolvedPackage]) -> Result<HashMap<String, Installed>, String> {
        let mut done = HashMap::new();
        for p in resolved.iter().filter(|p| p.source.starts_with("registry+")) {
            let name = &p.name;
            let registry = self.registry_of.get(name).ok_or_else(|| format!("{name}: no registry for this package"))?.clone();
            let pinned = self.old.into_iter().flat_map(|o| &o.packages).find(|q| q.name == *name && q.source == p.source && q.version == p.version);
            let locked_sum = pinned.and_then(|q| q.checksum.as_deref());
            let locked_native = pinned.map(|q| q.native.clone()).unwrap_or_default();

            if self.installer.installed_git(name, &p.source, locked_sum, &locked_native).is_some() {
                let dir = self.installer.package_dir(name);
                let checksum = crate::installer::package_checksum(&dir)?;
                native::require(name, &locked_native)?;
                done.insert(name.clone(), Installed { checksum, native: locked_native });
                continue;
            }
            let entries = match self.archives.remove(name) {
                Some(entries) => entries,
                None => {
                    let sum = p.source.rsplit_once('#').map_or("", |(_, s)| s);
                    unpack(&registry.archive(name, &p.version, sum)?, name, &p.version)?
                }
            };
            let (package, checksum, natives) = check(name, &p.version, &entries)?;
            let declared: BTreeMap<String, String> = package.dependencies.iter().map(|(k, v)| (k.clone(), v.version_req_str().to_string())).collect();
            if package.dependencies.values().any(|s| !s.is_registry()) {
                return Err(format!("{name} {}: a registry package can only depend on registry packages", p.version));
            }
            if let Some(entry) = self.indexes.get(name).and_then(|i| i.iter().find(|e| e.version == p.version)) {
                if declared != entry.dependencies {
                    return Err(format!("{name} {}: the archive's dependencies differ from the registry's index", p.version));
                }
            }
            if let Some(q) = pinned {
                if q.checksum.as_deref().is_some_and(|c| c != checksum) {
                    return Err(format!("{name} {}: the archive's files are {checksum}, but mote.lock has {}", p.version, q.checksum.as_deref().unwrap_or("")));
                }
                if q.checksum.is_some() && q.native != natives {
                    return Err(format!("{name} {}: the archive's native files differ from mote.lock", p.version));
                }
            }
            native::require(name, &natives)?;
            let files: Vec<_> = entries.into_iter().filter(|(path, _)| path != &format!("{name}.mbc") && path != "interface.txt").collect();
            self.installer.install_git(name, &p.source, &files)?;
            done.insert(name.clone(), Installed { checksum, native: natives });
        }
        Ok(done)
    }
}

/// The entries of an archive's bytes, with every path checked.
fn unpack(bytes: &[u8], name: &str, version: &Version) -> Result<Vec<(String, Vec<u8>)>, String> {
    let entries = MpkArchive::decode(bytes).map_err(|e| format!("{name} {version}: {e}"))?;
    check_paths(&entries).map_err(|e| format!("{name} {version}: {e}"))?;
    Ok(entries)
}

/// Fails on a path that is absolute or climbs out of the package.
pub(crate) fn check_paths(entries: &[(String, Vec<u8>)]) -> Result<(), String> {
    for (path, _) in entries {
        let bad = path.is_empty() || path.starts_with('/') || path.contains('\\') || path.contains('\0') || path.split('/').any(|part| part.is_empty() || part == "..");
        if bad {
            return Err(format!("the archive has the path '{path}'"));
        }
    }
    Ok(())
}

/// The package in an archive's `entries`, checked to be `name` at `version`.
fn check(name: &str, version: &Version, entries: &[(String, Vec<u8>)]) -> Result<(PackageManifest, String, BTreeMap<String, String>), String> {
    let toml = entries.iter().find(|(p, _)| p == "mote.toml").ok_or_else(|| format!("{name} {version}: the archive has no mote.toml"))?;
    let manifest = PackageManifest::from_toml_str(&String::from_utf8_lossy(&toml.1)).map_err(|e| format!("{name} {version}: {e}"))?;
    if manifest.package.name != name || manifest.package.version != *version {
        return Err(format!("{name} {version}: the archive holds {} {}", manifest.package.name, manifest.package.version));
    }
    let files = entries.iter().filter_map(|(p, d)| Some((p.strip_prefix("src/")?.to_string(), d.as_slice()))).collect();
    Ok((manifest, tree_checksum(files), archive_checksums(entries)))
}
