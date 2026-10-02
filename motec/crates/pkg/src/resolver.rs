use std::collections::HashMap;
use crate::manifest::PackageManifest;
use crate::semver::{Version, VersionReq};

#[derive(Clone, Debug, PartialEq, Eq)]
/// A version a registry offers.
pub struct AvailablePackage {
    pub name: String,
    pub version: Version,
    pub dependencies: HashMap<String, String>,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A package chosen by the resolver.
pub struct ResolvedPackage {
    pub name: String,
    pub version: Version,
    pub source: String,
}

/// Picks one version of every dependency that satisfies all constraints.
pub struct DependencyResolver {
    universe: HashMap<String, Vec<AvailablePackage>>,
    preferred: HashMap<String, Version>,
}

impl Default for DependencyResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl DependencyResolver {
    pub fn new() -> Self {
        Self {
            universe: HashMap::new(),
            preferred: HashMap::new(),
        }
    }

    pub fn add_available_package(&mut self, pkg: AvailablePackage) {
        self.universe.entry(pkg.name.clone()).or_default().push(pkg);
    }

    /// Tries `version` of `name` before any other matching version.
    pub(crate) fn prefer(&mut self, name: &str, version: Version) {
        self.preferred.insert(name.to_string(), version);
    }

    /// Resolves root manifest dependencies into a unified concrete version set.
    pub fn resolve(&self, root_manifest: &PackageManifest) -> Result<Vec<ResolvedPackage>, String> {
        let mut requirements: Vec<(String, VersionReq, String)> = Vec::new();

        for (name, spec) in &root_manifest.dependencies {
            let req_str = spec.version_req_str();
            let req = VersionReq::parse(req_str)
                .map_err(|e| format!("Invalid version constraint for '{}': {}", name, e))?;
            requirements.push((name.clone(), req, root_manifest.package.name.clone()));
        }

        let mut solution: HashMap<String, (Version, String)> = HashMap::new();
        self.backtrack_solve(&requirements, &mut solution)?;

        let mut result: Vec<ResolvedPackage> = solution
            .into_iter()
            .map(|(name, (version, source))| ResolvedPackage { name, version, source })
            .collect();

        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }

    fn backtrack_solve(
        &self,
        pending: &[(String, VersionReq, String)],
        current_solution: &mut HashMap<String, (Version, String)>,
    ) -> Result<(), String> {
        if pending.is_empty() {
            return Ok(());
        }

        let (pkg_name, req, required_by) = &pending[0];
        let rest = &pending[1..];

        if let Some((existing_ver, _)) = current_solution.get(pkg_name) {
            if !req.matches(existing_ver) {
                return Err(format!(
                    "Version conflict for package '{}': required constraint '{}' (from '{}') contradicts already resolved version '{}'",
                    pkg_name, req, required_by, existing_ver
                ));
            }
            return self.backtrack_solve(rest, current_solution);
        }

        let mut candidates = self.universe.get(pkg_name).cloned().unwrap_or_default();
        candidates.sort_by(|a, b| b.version.cmp(&a.version));
        candidates.sort_by_key(|c| self.preferred.get(pkg_name) != Some(&c.version));

        let matching: Vec<_> = candidates.into_iter().filter(|c| req.matches(&c.version)).collect();
        if matching.is_empty() {
            return Err(format!(
                "Could not satisfy version constraint '{}' for package '{}' (required by '{}'). No compatible versions found.",
                req, pkg_name, required_by
            ));
        }

        for cand in matching {
            current_solution.insert(pkg_name.clone(), (cand.version.clone(), cand.source.clone()));

            let mut next_pending = rest.to_vec();
            for (dep_name, dep_req_str) in &cand.dependencies {
                let dep_req = VersionReq::parse(dep_req_str)
                    .map_err(|e| format!("Invalid constraint in package '{}-{}': {}", cand.name, cand.version, e))?;
                next_pending.push((dep_name.clone(), dep_req, format!("{}-{}", cand.name, cand.version)));
            }

            if self.backtrack_solve(&next_pending, current_solution).is_ok() {
                return Ok(());
            }

            current_solution.remove(pkg_name);
        }

        Err(format!("Failed to find mutually compatible version set for '{}'", pkg_name))
    }
}
