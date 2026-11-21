//! Helpers the tests of this crate share.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::manifest::Manifest;
use crate::package::{Package, SourceFile};

/// A package named `name` that depends on `deps` (each at `../<dep>`) and holds the files, as (path below `src`, text).
pub fn pkg(name: &str, deps: &[&str], files: &[(&str, &str)]) -> Package {
    let mut text = format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n[dependencies]\n");
    for dep in deps {
        text.push_str(&format!("{dep} = {{ path = \"../{dep}\" }}\n"));
    }
    let manifest = Manifest::parse("mote.toml", &text).unwrap();
    let files = files.iter().map(|(rel, text)| SourceFile { rel: rel.to_string(), name: format!("{name}/src/{rel}"), text: text.to_string() }).collect();
    Package::new(manifest, format!("/work/{name}"), files)
}

/// A directory under the system temp directory, removed when dropped.
pub struct Scratch {
    pub dir: PathBuf,
}

impl Default for Scratch {
    fn default() -> Self {
        Scratch::new()
    }
}

impl Scratch {
    pub fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("mote-packages-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    /// Writes `text` at `rel` below the directory, making the directories.
    pub fn write(&self, rel: &str, text: &str) -> PathBuf {
        let path = self.dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        path
    }

    /// Writes a package at `rel` with a manifest naming `deps` and the files below its `src/`.
    pub fn package(&self, rel: &str, name: &str, deps: &[(&str, &str)], files: &[(&str, &str)]) -> PathBuf {
        let mut text = format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n[dependencies]\n");
        for (dep, path) in deps {
            text.push_str(&format!("{dep} = {{ path = \"{path}\" }}\n"));
        }
        self.write(&format!("{rel}/mote.toml"), &text);
        for (file, body) in files {
            self.write(&format!("{rel}/src/{file}"), body);
        }
        self.dir.join(rel)
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.dir.join(rel)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A set of packages taken through resolve, check and instantiate from the `main` of the root's entry file.
pub struct Built {
    pub program: mote_semantics::Program,
    pub interner: mote_base::Interner,
    pub ranges: Vec<crate::graph::PackageModules>,
    /// The messages of the errors on the way.
    pub errors: Vec<String>,
}

pub fn analyzed(packages: &crate::package::Packages) -> Built {
    use mote_base::Severity;
    use mote_semantics::{Instance, Resolved};

    let graph = crate::graph::build(packages, false);
    let crate::graph::Graph { input, mut interner, mut sink, packages: ranges, .. } = graph;
    let mut program = mote_semantics::resolve(input, &mut interner, &mut sink);
    if !sink.has_errors() {
        mote_semantics::check(&mut program, &mut interner, &mut sink);
    }
    if !sink.has_errors() {
        let entry = ranges.last().and_then(|range| range.entry).expect("the root has an entry file");
        let module = program.modules.iter().nth(entry).map(|(id, _)| id).expect("the entry module");
        let main = interner.get("main").and_then(|name| program.modules[module].scope.get(&name).copied());
        if let Some(Resolved::Def(def)) = main {
            mote_semantics::instantiate(&mut program, &interner, &mut sink, &[Instance { def, args: Vec::new() }]);
        }
    }
    let errors = sink.iter().filter(|diagnostic| diagnostic.severity == Severity::Error).map(|diagnostic| diagnostic.message.clone()).collect();
    Built { program, interner, ranges, errors }
}

pub fn std_package() -> Package {
    Package::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../std")).unwrap()
}
