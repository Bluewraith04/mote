//! `mote.toml` as a typed [`Manifest`].

use std::fmt;
use std::path::Path;
use std::str::FromStr;

use crate::toml::{self, Entry, Table, Value};

/// The entry file when `entry` is not set.
pub const DEFAULT_ENTRY: &str = "src/main.mote";

/// `major.minor.patch`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl FromStr for Version {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, ()> {
        let mut parts = text.split('.').map(|part| if !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()) { part.parse::<u32>().map_err(|_| ()) } else { Err(()) });
        let version = Version { major: parts.next().ok_or(())??, minor: parts.next().ok_or(())??, patch: parts.next().ok_or(())?? };
        if parts.next().is_some() {
            return Err(());
        }
        Ok(version)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, out: &mut fmt::Formatter) -> fmt::Result {
        write!(out, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Where a dependency is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// A directory.
    Path,
    /// A zip file.
    Zip,
}

/// A dependency on the package in a directory or a zip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dependency {
    pub name: String,
    pub source: Source,
    /// As written, relative to the directory of the manifest.
    pub path: String,
    /// The line of the entry in the manifest.
    pub line: u32,
}

/// The contents of `mote.toml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub version: Version,
    /// The oldest `mote` that builds the package.
    pub mote: Option<Version>,
    /// The file the program starts in or the library is imported from, relative to the package.
    pub entry: String,
    pub description: Option<String>,
    pub license: Option<String>,
    pub authors: Vec<String>,
    /// In the order written.
    pub dependencies: Vec<Dependency>,
    /// The line of `name` in the manifest.
    pub name_line: u32,
    mote_line: u32,
}

/// Whether the text is a name the language can write: letters, digits and `_`, not starting with a digit.
pub fn is_name(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|first| first.is_ascii_alphabetic() || first == '_') && chars.all(|rest| rest.is_ascii_alphanumeric() || rest == '_')
}

/// A package that cannot be read or built: the file, the line (0 when there is none), the key and what is wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageError {
    pub file: String,
    pub line: u32,
    pub key: String,
    pub message: String,
}

impl fmt::Display for PackageError {
    fn fmt(&self, out: &mut fmt::Formatter) -> fmt::Result {
        write!(out, "{}", self.file)?;
        if self.line > 0 {
            write!(out, ":{}", self.line)?;
        }
        if self.key.is_empty() {
            write!(out, ": {}", self.message)
        } else {
            write!(out, ": `{}`: {}", self.key, self.message)
        }
    }
}

impl std::error::Error for PackageError {}

struct Reader<'a> {
    file: &'a str,
}

impl Reader<'_> {
    fn fail<T>(&self, line: u32, key: &str, message: impl Into<String>) -> Result<T, PackageError> {
        Err(PackageError { file: self.file.to_string(), line, key: key.to_string(), message: message.into() })
    }

    fn string(&self, entry: &Entry, key: &str) -> Result<String, PackageError> {
        match &entry.value {
            Value::Str(text) => Ok(text.clone()),
            other => self.fail(entry.line, key, format!("expected a string, found {}", other.kind())),
        }
    }

    fn version(&self, entry: &Entry, key: &str) -> Result<Version, PackageError> {
        let text = self.string(entry, key)?;
        text.parse().or_else(|()| self.fail(entry.line, key, format!("`{text}` is not a version: expected `major.minor.patch`")))
    }

    fn name(&self, entry: &Entry, key: &str, text: &str) -> Result<(), PackageError> {
        if !is_name(text) {
            return self.fail(entry.line, key, format!("`{text}` is not a name: letters, digits and `_`, not starting with a digit"));
        }
        Ok(())
    }

    fn entry_path(&self, entry: &Entry, key: &str, text: &str) -> Result<(), PackageError> {
        let path = Path::new(text);
        let inside = path.is_relative() && path.components().all(|part| matches!(part, std::path::Component::Normal(_)));
        if !inside || path.extension().is_none_or(|extension| extension != "mote") {
            return self.fail(entry.line, key, format!("`{text}` is not a path to a `.mote` file inside the package"));
        }
        Ok(())
    }

    fn package(&self, table: &Table, line: u32) -> Result<Manifest, PackageError> {
        let mut name = None;
        let mut name_line = 0;
        let mut version = None;
        let mut mote = None;
        let mut mote_line = 0;
        let mut entry_text = None;
        let mut description = None;
        let mut license = None;
        let mut authors = Vec::new();
        for entry in table {
            let key = format!("package.{}", entry.key);
            match entry.key.as_str() {
                "name" => {
                    let text = self.string(entry, &key)?;
                    self.name(entry, &key, &text)?;
                    name = Some(text);
                    name_line = entry.line;
                }
                "version" => version = Some(self.version(entry, &key)?),
                "mote" => {
                    mote = Some(self.version(entry, &key)?);
                    mote_line = entry.line;
                }
                "entry" => {
                    let text = self.string(entry, &key)?;
                    self.entry_path(entry, &key, &text)?;
                    entry_text = Some(text);
                }
                "description" => description = Some(self.string(entry, &key)?),
                "license" => license = Some(self.string(entry, &key)?),
                "authors" => authors = self.strings(entry, &key)?,
                _ => return self.fail(entry.line, &key, "unknown key"),
            }
        }
        let Some(name) = name else { return self.fail(line, "package.name", "missing") };
        let Some(version) = version else { return self.fail(line, "package.version", "missing") };
        Ok(Manifest { name, version, mote, entry: entry_text.unwrap_or_else(|| DEFAULT_ENTRY.to_string()), description, license, authors, dependencies: Vec::new(), name_line, mote_line })
    }

    fn strings(&self, entry: &Entry, key: &str) -> Result<Vec<String>, PackageError> {
        let Value::Array(items) = &entry.value else { return self.fail(entry.line, key, format!("expected an array, found {}", entry.value.kind())) };
        items
            .iter()
            .map(|item| match item {
                Value::Str(text) => Ok(text.clone()),
                other => self.fail(entry.line, key, format!("expected strings, found {}", other.kind())),
            })
            .collect()
    }

    fn dependencies(&self, table: &Table) -> Result<Vec<Dependency>, PackageError> {
        let mut found = Vec::new();
        for entry in table {
            let key = format!("dependencies.{}", entry.key);
            self.name(entry, &key, &entry.key)?;
            let Value::Table(fields) = &entry.value else { return self.fail(entry.line, &key, format!("expected a table with `path` or `zip`, found {}", entry.value.kind())) };
            let mut found_source = None;
            for field in fields {
                let source = match field.key.as_str() {
                    "path" => Source::Path,
                    "zip" => Source::Zip,
                    other => return self.fail(field.line, &format!("{key}.{other}"), "unknown key: only `path` and `zip` are read"),
                };
                let at = format!("{key}.{}", field.key);
                let text = self.string(field, &at)?;
                if text.is_empty() {
                    return self.fail(field.line, &at, "the path is empty");
                }
                if source == Source::Zip && !text.ends_with(".zip") {
                    return self.fail(field.line, &at, format!("`{text}` is not a `.zip` file"));
                }
                if found_source.replace((source, text)).is_some() {
                    return self.fail(field.line, &key, "give `path` or `zip`, not both");
                }
            }
            let Some((source, path)) = found_source else { return self.fail(entry.line, &key, "needs `path` or `zip`") };
            found.push(Dependency { name: entry.key.clone(), source, path, line: entry.line });
        }
        Ok(found)
    }
}

impl Manifest {
    /// Reads the manifest in `text`; `file` names it in an error.
    pub fn parse(file: &str, text: &str) -> Result<Manifest, PackageError> {
        let reader = Reader { file };
        let root = toml::parse(text).or_else(|syntax| reader.fail(syntax.line, "", syntax.message))?;
        let mut manifest = None;
        let mut dependencies = Vec::new();
        for entry in &root {
            let Value::Table(table) = &entry.value else { return reader.fail(entry.line, &entry.key, format!("expected a table, found {}", entry.value.kind())) };
            match entry.key.as_str() {
                "package" => manifest = Some(reader.package(table, entry.line)?),
                "dependencies" => dependencies = reader.dependencies(table)?,
                other => return reader.fail(entry.line, other, "unknown table"),
            }
        }
        let Some(mut manifest) = manifest else { return reader.fail(1, "package", "missing") };
        manifest.dependencies = dependencies;
        Ok(manifest)
    }

    /// Reads the file at `path`.
    pub fn read(path: &Path) -> Result<Manifest, PackageError> {
        let file = crate::package::shown(path);
        match std::fs::read_to_string(path) {
            Ok(text) => Manifest::parse(&file, &text),
            Err(error) => Err(PackageError { file, line: 0, key: String::new(), message: format!("cannot read: {error}") }),
        }
    }

    /// An error when the package needs a newer `mote` than `toolchain`.
    pub fn check_toolchain(&self, file: &str, toolchain: &Version) -> Result<(), PackageError> {
        match self.mote {
            Some(needed) if needed > *toolchain => Err(PackageError { file: file.to_string(), line: self.mote_line, key: "package.mote".into(), message: format!("needs mote {needed}, this is {toolchain}") }),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = "[package]\nname = \"pane\"\nversion = \"0.1.0\"\n";

    fn error(text: &str) -> String {
        Manifest::parse("mote.toml", text).expect_err("an invalid manifest").to_string()
    }

    #[test]
    fn manifest_reads_every_field() {
        let text = "[package]\nname = \"pane\"\nversion = \"1.2.3\"\nmote = \"0.2.0\"\nentry = \"src/lib.mote\"\ndescription = \"Windows\"\nlicense = \"MIT\"\nauthors = [\"Ada\", \"Bo\"]\n\n[dependencies]\nwire = { path = \"../wire\" }\ntile = { path = \"tile\" }\nnav = { zip = \"deps/nav-1.0.0.zip\" }\n";
        let manifest = Manifest::parse("mote.toml", text).unwrap();
        assert_eq!(manifest.name, "pane");
        assert_eq!(manifest.version, Version { major: 1, minor: 2, patch: 3 });
        assert_eq!(manifest.mote, Some(Version { major: 0, minor: 2, patch: 0 }));
        assert_eq!(manifest.entry, "src/lib.mote");
        assert_eq!(manifest.description.as_deref(), Some("Windows"));
        assert_eq!(manifest.license.as_deref(), Some("MIT"));
        assert_eq!(manifest.authors, ["Ada", "Bo"]);
        assert_eq!(manifest.dependencies, [Dependency { name: "wire".into(), source: Source::Path, path: "../wire".into(), line: 11 }, Dependency { name: "tile".into(), source: Source::Path, path: "tile".into(), line: 12 }, Dependency { name: "nav".into(), source: Source::Zip, path: "deps/nav-1.0.0.zip".into(), line: 13 }]);
        assert_eq!(manifest.name_line, 2);
    }

    #[test]
    fn manifest_with_only_a_name_and_a_version_takes_the_defaults() {
        let manifest = Manifest::parse("mote.toml", MINIMAL).unwrap();
        assert_eq!(manifest.entry, DEFAULT_ENTRY);
        assert_eq!(manifest.mote, None);
        assert_eq!(manifest.description, None);
        assert_eq!(manifest.license, None);
        assert_eq!(manifest.authors, Vec::<String>::new());
        assert_eq!(manifest.dependencies, Vec::new());
    }

    #[test]
    fn manifest_reads_the_std_manifest() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../std/mote.toml");
        let manifest = Manifest::read(&path).unwrap();
        assert_eq!((manifest.name.as_str(), manifest.entry.as_str()), ("std", "src/lib.mote"));
    }

    #[test]
    fn manifest_error_names_the_file_the_line_and_the_key() {
        let wrong = "[package]\nname = \"pane\"\nversion = 3\n";
        let found = Manifest::parse("pane/mote.toml", wrong).unwrap_err();
        assert_eq!((found.file.as_str(), found.line, found.key.as_str()), ("pane/mote.toml", 3, "package.version"));
        assert_eq!(found.to_string(), "pane/mote.toml:3: `package.version`: expected a string, found an integer");
    }

    #[test]
    fn manifest_syntax_error_names_the_file_and_the_line() {
        let found = Manifest::parse("mote.toml", "[package\n").unwrap_err();
        assert_eq!(found.to_string(), "mote.toml:1: expected `]`");
    }

    #[test]
    fn manifest_missing_parts_are_errors() {
        assert_eq!(error(""), "mote.toml:1: `package`: missing");
        assert_eq!(error("[package]\nversion = \"1.0.0\"\n"), "mote.toml:1: `package.name`: missing");
        assert_eq!(error("[package]\nname = \"a\"\n"), "mote.toml:1: `package.version`: missing");
        assert_eq!(error(&format!("{MINIMAL}[dependencies]\nwire = {{}}\n")), "mote.toml:5: `dependencies.wire`: needs `path` or `zip`");
    }

    #[test]
    fn manifest_unknown_keys_and_tables_are_errors() {
        assert_eq!(error(&format!("{MINIMAL}edition = \"2026\"\n")), "mote.toml:4: `package.edition`: unknown key");
        assert_eq!(error(&format!("{MINIMAL}[run]\n")), "mote.toml:4: `run`: unknown table");
        assert_eq!(error(&format!("{MINIMAL}[dependencies]\nwire = {{ path = \"w\", git = \"x\" }}\n")), "mote.toml:5: `dependencies.wire.git`: unknown key: only `path` and `zip` are read");
    }

    #[test]
    fn manifest_values_of_the_wrong_type_are_errors() {
        assert_eq!(error(&format!("{MINIMAL}authors = \"Ada\"\n")), "mote.toml:4: `package.authors`: expected an array, found a string");
        assert_eq!(error(&format!("{MINIMAL}authors = [1]\n")), "mote.toml:4: `package.authors`: expected strings, found an integer");
        assert_eq!(error("package = 1\n"), "mote.toml:1: `package`: expected a table, found an integer");
        assert_eq!(error(&format!("{MINIMAL}[dependencies]\nwire = \"1.0.0\"\n")), "mote.toml:5: `dependencies.wire`: expected a table with `path` or `zip`, found a string");
    }

    #[test]
    fn manifest_names_versions_and_paths_are_checked() {
        assert_eq!(error("[package]\nname = \"1pane\"\nversion = \"1.0.0\"\n"), "mote.toml:2: `package.name`: `1pane` is not a name: letters, digits and `_`, not starting with a digit");
        assert_eq!(error("[package]\nname = \"a\"\nversion = \"1.0\"\n"), "mote.toml:3: `package.version`: `1.0` is not a version: expected `major.minor.patch`");
        assert_eq!(error("[package]\nname = \"a\"\nversion = \"1.0.0-rc1\"\n"), "mote.toml:3: `package.version`: `1.0.0-rc1` is not a version: expected `major.minor.patch`");
        assert_eq!(error(&format!("{MINIMAL}entry = \"../x.mote\"\n")), "mote.toml:4: `package.entry`: `../x.mote` is not a path to a `.mote` file inside the package");
        assert_eq!(error(&format!("{MINIMAL}entry = \"/x.mote\"\n")), "mote.toml:4: `package.entry`: `/x.mote` is not a path to a `.mote` file inside the package");
        assert_eq!(error(&format!("{MINIMAL}entry = \"src/main\"\n")), "mote.toml:4: `package.entry`: `src/main` is not a path to a `.mote` file inside the package");
        assert_eq!(error(&format!("{MINIMAL}[dependencies]\nwire = {{ path = \"\" }}\n")), "mote.toml:5: `dependencies.wire.path`: the path is empty");
    }

    #[test]
    fn manifest_a_zip_dependency_is_a_zip_file_and_not_also_a_path() {
        assert_eq!(error(&format!("{MINIMAL}[dependencies]\nwire = {{ zip = \"deps/wire.tar\" }}\n")), "mote.toml:5: `dependencies.wire.zip`: `deps/wire.tar` is not a `.zip` file");
        assert_eq!(error(&format!("{MINIMAL}[dependencies]\nwire = {{ path = \"w\", zip = \"w.zip\" }}\n")), "mote.toml:5: `dependencies.wire`: give `path` or `zip`, not both");
        assert_eq!(error(&format!("{MINIMAL}[dependencies]\nwire = {{ zip = \"\" }}\n")), "mote.toml:5: `dependencies.wire.zip`: the path is empty");
    }

    #[test]
    fn manifest_a_missing_file_is_an_error_naming_it() {
        let found = Manifest::read(Path::new("/nonexistent/mote.toml")).unwrap_err();
        assert_eq!((found.file.as_str(), found.line), ("/nonexistent/mote.toml", 0));
        assert!(found.to_string().starts_with("/nonexistent/mote.toml: cannot read: "), "{found}");
    }

    #[test]
    fn manifest_needing_a_newer_mote_is_an_error_at_the_key() {
        let text = format!("{MINIMAL}mote = \"0.3.0\"\n");
        let manifest = Manifest::parse("mote.toml", &text).unwrap();
        let toolchain = Version { major: 0, minor: 2, patch: 0 };
        assert_eq!(manifest.check_toolchain("mote.toml", &toolchain).unwrap_err().to_string(), "mote.toml:4: `package.mote`: needs mote 0.3.0, this is 0.2.0");
        assert_eq!(manifest.check_toolchain("mote.toml", &Version { major: 0, minor: 3, patch: 0 }), Ok(()));
        assert_eq!(Manifest::parse("mote.toml", MINIMAL).unwrap().check_toolchain("mote.toml", &toolchain), Ok(()));
    }

    #[test]
    fn manifest_versions_compare_by_number_not_by_text() {
        let low: Version = "0.9.0".parse().unwrap();
        let high: Version = "0.10.0".parse().unwrap();
        assert!(low < high);
        assert_eq!(high.to_string(), "0.10.0");
    }
}
