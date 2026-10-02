use std::path::Path;

/// Represents a normalized canonical module identifier.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CanonicalModuleId(pub String);

impl CanonicalModuleId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The module name used in diagnostics: the file stem, with `mod.mote` and `lib.mote` named after their directory. Not unique and not identifier-safe.
    pub fn display_name(&self) -> &str {
        if let Some(std_name) = self.0.strip_prefix("<std>/") {
            return std_name;
        }
        let path = Path::new(&self.0);
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("mod");
        if stem == "mod" || stem == "lib" {
            path.parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .unwrap_or(stem)
        } else {
            stem
        }
    }

    /// The module file path for diagnostics: relative to the current directory when under it, else the full path.
    pub(crate) fn display_path(&self) -> String {
        let full = Path::new(&self.0);
        let rel = std::env::current_dir()
            .ok()
            .and_then(|cwd| full.strip_prefix(&cwd).ok().map(Path::to_path_buf));
        match rel {
            Some(rel) if !rel.as_os_str().is_empty() => {
                let s = rel.to_string_lossy();
                if s.contains(std::path::MAIN_SEPARATOR) {
                    s.into_owned()
                } else {
                    format!(".{}{}", std::path::MAIN_SEPARATOR, s)
                }
            }
            _ => self.0.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_path_is_relative_to_cwd() {
        let cwd = std::env::current_dir().unwrap();
        let id = CanonicalModuleId::new(
            cwd.join("src").join("ext.mote").to_string_lossy().to_string(),
        );
        assert_eq!(
            id.display_path(),
            format!("src{}ext.mote", std::path::MAIN_SEPARATOR)
        );

        let bare = CanonicalModuleId::new(cwd.join("main.mote").to_string_lossy().to_string());
        assert_eq!(
            bare.display_path(),
            format!(".{}main.mote", std::path::MAIN_SEPARATOR)
        );
    }

    #[test]
    fn display_path_falls_back_to_full_path_when_not_under_cwd() {
        let id = CanonicalModuleId::new("/somewhere/else/lib.mote");
        assert_eq!(id.display_path(), "/somewhere/else/lib.mote");
    }
}
