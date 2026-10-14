//! The global config `~/.mote/mote_config.toml` and the registries a project can use.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::git::validate_name;
use crate::manifest::{PackageManifest, RegistryEntry};
use crate::registry::Registry;
use crate::store;

const FILE: &str = "mote_config.toml";
const TOKEN_ENV: &str = "MOTE_REGISTRY_TOKEN";

/// `[registries.NAME]` in the global config.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalRegistry {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// The contents of `~/.mote/mote_config.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default)]
    pub registries: BTreeMap<String, GlobalRegistry>,
}

impl Config {
    fn path() -> Result<PathBuf, String> {
        Ok(store::home()?.join(FILE))
    }

    /// The global config; empty when the file does not exist.
    pub fn load() -> Result<Config, String> {
        let path = Self::path()?;
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => return Err(format!("Failed to read '{}': {e}", path.display())),
        };
        let config: Config = toml::from_str(&text).map_err(|e| format!("Failed to parse '{}': {e}", path.display()))?;
        if config.registries.values().any(|r| r.token.is_some()) && readable_by_others(&path) {
            return Err(format!("{} holds a token and others can read it; run `chmod 600 {}`", path.display(), path.display()));
        }
        Ok(config)
    }

    /// Writes the config readable by its owner only.
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path()?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("Failed to create '{}': {e}", dir.display()))?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| format!("Failed to serialize {FILE}: {e}"))?;
        write_private(&path, text.as_bytes()).map_err(|e| format!("Failed to write '{}': {e}", path.display()))
    }
}

#[cfg(unix)]
fn readable_by_others(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o077 != 0)
}

#[cfg(not(unix))]
fn readable_by_others(_path: &Path) -> bool {
    false
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut file = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    fs::write(path, bytes)
}

/// A checked registry URL without a trailing slash.
pub fn check_url(url: &str) -> Result<String, String> {
    let bad = || format!("'{url}' is not a registry URL; write https://host, or http://localhost for local use");
    if url.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '?' | '#' | '\\')) {
        return Err(bad());
    }
    let (plain, rest) = match (url.strip_prefix("https://"), url.strip_prefix("http://")) {
        (Some(rest), _) => (false, rest),
        (_, Some(rest)) => (true, rest),
        _ => return Err(bad()),
    };
    let host = rest.split('/').next().unwrap_or("");
    let name = host.rsplit_once(':').filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit())).map_or(host, |(name, _)| name);
    if name.is_empty() {
        return Err(bad());
    }
    if plain && !matches!(name, "localhost" | "127.0.0.1") {
        return Err(format!("'{url}' is plain http; a registry must use https (http is only for localhost)"));
    }
    Ok(url.trim_end_matches('/').to_string())
}

/// `mote package --add-registry`: names `url` in the global config or `mote.toml`.
pub fn add_registry(name: &str, url: &str, project: Option<&Path>, make_default: bool) -> Result<(), String> {
    validate_name(name)?;
    let url = check_url(url)?;
    if let Some(root) = project {
        let path = root.join("mote.toml");
        let mut manifest = PackageManifest::from_file(&path)?;
        manifest.registries.insert(name.to_string(), RegistryEntry::Url(url));
        return manifest.save_to_file(&path);
    }
    let mut config = Config::load()?;
    let token = config.registries.get(name).filter(|r| r.url == url).and_then(|r| r.token.clone());
    config.registries.insert(name.to_string(), GlobalRegistry { url, token });
    if make_default || config.default.is_none() {
        config.default = Some(name.to_string());
    }
    config.save()
}

/// `mote package --remove-registry`: drops `name` from the global config or `mote.toml`.
pub fn remove_registry(name: &str, project: Option<&Path>) -> Result<(), String> {
    if let Some(root) = project {
        let path = root.join("mote.toml");
        let mut manifest = PackageManifest::from_file(&path)?;
        if manifest.registries.remove(name).is_none() {
            return Err(format!("mote.toml names no registry `{name}`"));
        }
        return manifest.save_to_file(&path);
    }
    let mut config = Config::load()?;
    if config.registries.remove(name).is_none() {
        return Err(format!("no registry named `{name}` in the global config"));
    }
    if config.default.as_deref() == Some(name) {
        config.default = None;
    }
    config.save()
}

/// The registries a project can use: its own `[registries]` over the global config.
pub struct Registries {
    named: BTreeMap<String, GlobalRegistry>,
    default: Option<String>,
}

impl Registries {
    /// The registries of `manifest` and the global config.
    pub fn for_manifest(manifest: &PackageManifest) -> Result<Self, String> {
        let config = Config::load()?;
        let mut named = config.registries;
        for (name, entry) in &manifest.registries {
            let url = check_url(entry.url()).map_err(|e| format!("registry `{name}`: {e}"))?;
            let token = named.get(name).filter(|r| r.url == url).and_then(|r| r.token.clone());
            named.insert(name.clone(), GlobalRegistry { url, token });
        }
        for (name, r) in &mut named {
            r.url = check_url(&r.url).map_err(|e| format!("registry `{name}`: {e}"))?;
        }
        Ok(Registries { named, default: config.default })
    }

    /// The registry called `name`, or the default one.
    pub fn get(&self, name: Option<&str>) -> Result<Registry, String> {
        let name = match name {
            Some(name) => name.to_string(),
            None => self.default_name()?,
        };
        let entry = self.named.get(&name).ok_or_else(|| format!("no registry named `{name}`; add it with `mote package --add-registry {name} URL`"))?;
        let token = std::env::var(TOKEN_ENV).ok().filter(|t| !t.is_empty()).or_else(|| entry.token.clone());
        Ok(Registry::new(&name, &entry.url, token))
    }

    fn default_name(&self) -> Result<String, String> {
        if let Some(name) = self.default.as_ref().filter(|n| self.named.contains_key(*n)) {
            return Ok(name.clone());
        }
        let mut names = self.named.keys();
        match (names.next(), names.next()) {
            (Some(only), None) => Ok(only.clone()),
            (None, _) => Err("no registry is configured; add one with `mote package --add-registry NAME URL`".to_string()),
            _ => Err("several registries are configured and none is the default; add one with `mote package --add-registry NAME URL --default`".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_https_or_local_http() {
        assert_eq!(check_url("https://reg.example.com/").unwrap(), "https://reg.example.com");
        assert_eq!(check_url("http://localhost:8080").unwrap(), "http://localhost:8080");
        assert!(check_url("http://127.0.0.1").is_ok());
        assert!(check_url("http://reg.example.com").unwrap_err().contains("https"));
        assert!(check_url("http://localhost@evil.example.com").is_err());
        assert!(check_url("ftp://x").is_err());
        assert!(check_url("https://").is_err());
        assert!(check_url("https://a b").is_err());
    }
}
