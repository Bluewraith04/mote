//! Shared by the tests that run programs importing a package from `motec/packages`.

use std::path::Path;

/// Makes the package `name` importable from `dir`, as `mote install` would.
#[allow(dead_code)]
pub fn install_package(dir: &Path, name: &str) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages").join(name).join("src");
    let target = dir.join(".mote_packages").join(name).join("src");
    std::fs::create_dir_all(&target).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
    }
}

/// Makes the packages `names`, which carry native libraries, importable from `dir` and granted them, by running `mote install` on a manifest that lists them with `native = true`.
#[allow(dead_code)]
pub fn install_native_packages(dir: &Path, names: &[&str]) {
    let packages = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages");
    let host = pkg::native::host_triple();
    let mut manifest = String::from("[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n");
    for name in names {
        let path = packages.join(name).canonicalize().unwrap();
        assert!(path.join("native").join(&host).is_dir(), "package {name} has no library for {host}; build one into native/{host}/");
        manifest.push_str(&format!("{name} = {{ path = \"{}\", native = true }}\n", path.display().to_string().replace('\\', "/")));
    }
    std::fs::write(dir.join("mote.toml"), manifest).unwrap();
    pkg::PackageManager::install_dependencies(dir, false).unwrap();
}
