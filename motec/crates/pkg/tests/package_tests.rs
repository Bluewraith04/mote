use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use pkg::{
    AvailablePackage, DependencyResolver, Lockfile, PackageInstaller,
    PackageManager, PackageManifest, Version,
};

fn setup_temp_dir(test_name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mote_pkg_test_{}_{}", test_name, std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn test_manifest_run_section_sets_the_heap_limit() {
    let with_run = "[package]\nname = \"p\"\nversion = \"0.1.0\"\nauthors = []\nedition = \"2026\"\nentry = \"src/main.mote\"\n\n[run]\nmax-heap = \"2GiB\"\n";
    let manifest = PackageManifest::from_toml_str(with_run).unwrap();
    assert_eq!(manifest.run.and_then(|r| r.max_heap).as_deref(), Some("2GiB"));

    let without_run = "[package]\nname = \"p\"\nversion = \"0.1.0\"\nauthors = []\nedition = \"2026\"\nentry = \"src/main.mote\"\n";
    assert!(PackageManifest::from_toml_str(without_run).unwrap().run.is_none());
}

#[test]
fn test_manifest_parsing() {
    let toml_str = r#"
[package]
name = "http-client"
version = "0.2.1"
authors = ["Mote Team <team@mote-lang.org>"]
edition = "2026"
entry = "src/lib.mote"

[dependencies]
json = "^1.4.0"
socket = { version = ">=0.8.0, <2.0.0", path = "../socket" }
"#;

    let manifest = PackageManifest::from_toml_str(toml_str).unwrap();
    assert_eq!(manifest.package.name, "http-client");
    assert_eq!(manifest.package.version, Version::new(0, 2, 1));
    assert_eq!(manifest.dependencies.len(), 2);
}

#[test]
fn test_dependency_solver_and_conflicts() {
    let mut solver = DependencyResolver::new();

    solver.add_available_package(AvailablePackage {
        name: "json".into(),
        version: Version::new(1, 4, 0),
        dependencies: HashMap::new(),
        source: "registry".into(),
    });
    solver.add_available_package(AvailablePackage {
        name: "json".into(),
        version: Version::new(1, 4, 2),
        dependencies: HashMap::new(),
        source: "registry".into(),
    });
    solver.add_available_package(AvailablePackage {
        name: "json".into(),
        version: Version::new(2, 0, 0),
        dependencies: HashMap::new(),
        source: "registry".into(),
    });

    let manifest_toml = r#"
[package]
name = "app"
version = "0.1.0"

[dependencies]
json = "^1.4.0"
"#;
    let manifest = PackageManifest::from_toml_str(manifest_toml).unwrap();
    let resolved = solver.resolve(&manifest).unwrap();
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].name, "json");
    assert_eq!(resolved[0].version, Version::new(1, 4, 2));

    let conflict_toml = r#"
[package]
name = "app"
version = "0.1.0"

[dependencies]
json = ">=3.0.0"
"#;
    let conflict_manifest = PackageManifest::from_toml_str(conflict_toml).unwrap();
    let conflict_res = solver.resolve(&conflict_manifest);
    assert!(conflict_res.is_err());
    let err = conflict_res.unwrap_err();
    assert!(err.contains("Could not satisfy version constraint"));
}

#[test]
fn test_lockfile_reproducibility() {
    let toml_str = r#"
[package]
name = "app"
version = "1.0.0"

[dependencies]
math = "^1.0.0"
"#;
    let manifest = PackageManifest::from_toml_str(toml_str).unwrap();

    let mut solver = DependencyResolver::new();
    solver.add_available_package(AvailablePackage {
        name: "math".into(),
        version: Version::new(1, 2, 0),
        dependencies: HashMap::new(),
        source: "registry".into(),
    });

    let resolved1 = solver.resolve(&manifest).unwrap();
    let lock1 = Lockfile::from_resolved(&resolved1);

    let resolved2 = solver.resolve(&manifest).unwrap();
    let lock2 = Lockfile::from_resolved(&resolved2);

    assert_eq!(lock1, lock2);
}

#[test]
fn test_installer_rejects_non_local_packages() {
    let temp = setup_temp_dir("inst_disc");
    let installer = PackageInstaller::new(&temp);

    let packages = vec![pkg::ResolvedPackage {
        name: "geometry".into(),
        version: Version::new(1, 0, 0),
        source: "registry+file:///nowhere".into(),
    }];

    let err = installer.install(&packages).unwrap_err();
    assert!(err.contains("is not a path dependency"), "got: {err}");

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_package_is_one_archive_per_version() {
    let temp = setup_temp_dir("one_archive");
    PackageManager::init_project(&temp, Some("geo"), true).unwrap();
    let mpk = PackageManager::package(&temp).unwrap();
    assert!(mpk.ends_with("dist/geo-0.1.0.mpk"), "{}", mpk.display());
    let names: Vec<String> = pkg::MpkArchive::read(&mpk).unwrap().into_iter().map(|(p, _)| p).collect();
    assert!(names.contains(&"mote.toml".to_string()) && names.iter().any(|n| n.starts_with("src/")), "{names:?}");
    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_package_manager_cli_commands() {
    let temp = setup_temp_dir("cli_cmds");

    PackageManager::init_project(&temp, Some("test-project"), false).unwrap();
    assert!(temp.join("mote.toml").exists());
    assert!(temp.join("src").join("main.mote").exists());

    let err = PackageManager::add_dependency(&temp, "logger@^1.0.0").unwrap_err();
    assert!(err.contains("no registry is configured") && err.contains("MOTE_REGISTRY"), "{err}");
    let manifest = PackageManifest::from_file(&temp.join("mote.toml")).unwrap();
    assert!(!manifest.dependencies.contains_key("logger"));

    PackageManager::remove_dependency(&temp, "logger").unwrap();
    let updated_manifest = PackageManifest::from_file(&temp.join("mote.toml")).unwrap();
    assert!(!updated_manifest.dependencies.contains_key("logger"));

    let bytecode_artifact = PackageManager::compile(&temp).unwrap();
    assert!(bytecode_artifact.exists());

    assert!(PackageManager::package(&temp).unwrap().ends_with("dist/test-project-0.1.0.mpk"));
    assert!(PackageManager::publish(&temp).is_err());

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_compile_emits_loadable_mbc() {
    let temp = setup_temp_dir("build_mbc");
    fs::create_dir_all(temp.join("src")).unwrap();
    fs::write(
        temp.join("mote.toml"),
        "[package]\nname = \"calc\"\nversion = \"0.1.0\"\nentry = \"src/main.mote\"\n",
    )
    .unwrap();
    fs::write(
        temp.join("src").join("main.mote"),
        "fn main() -> Int { return 6 * 7 }\n",
    )
    .unwrap();

    let mbc_path = PackageManager::compile(&temp).unwrap();
    assert_eq!(mbc_path.extension().unwrap(), "mbc");

    let raw = fs::read(&mbc_path).unwrap();
    assert_eq!(&raw[0..5], pkg::MbcFile::MAGIC);

    let compiled = pkg::MbcFile::read(&mbc_path).unwrap();
    let mut registry = isa::value::TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(42));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_compile_program_returns_the_written_program() {
    let temp = setup_temp_dir("build_program");
    fs::create_dir_all(temp.join("src")).unwrap();
    fs::write(
        temp.join("mote.toml"),
        "[package]\nname = \"twelve\"\nversion = \"0.1.0\"\nentry = \"src/main.mote\"\n",
    )
    .unwrap();
    fs::write(temp.join("src").join("main.mote"), "fn main() -> Int { return 12 }\nreturn main()\n").unwrap();

    let (mbc_path, compiled) = PackageManager::compile_program(&temp).unwrap();
    assert!(mbc_path.ends_with("dist/twelve.mbc"));
    assert_eq!(pkg::MbcFile::read(&mbc_path).unwrap().code_objects.len(), compiled.code_objects.len());

    let mut rt = runtime::Runtime::with_types(compiled.code_objects, compiled.type_descriptors);
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(12));

    fs::remove_file(temp.join("src").join("main.mote")).unwrap();
    assert!(PackageManager::compile_program(&temp).unwrap_err().contains("does not exist"));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_bundle_program_embeds_a_prebuilt_program() {
    let temp = setup_temp_dir("bundle_program");
    fs::create_dir_all(temp.join("src")).unwrap();
    fs::write(
        temp.join("mote.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nentry = \"src/main.mote\"\n",
    )
    .unwrap();
    fs::write(temp.join("src").join("main.mote"), "fn main() -> Int { return 99 }\nreturn main()\n").unwrap();

    let (_mbc, compiled) = PackageManager::compile_program(&temp).unwrap();
    let out = temp.join("app.bin");
    pkg::StandaloneBundler::bundle_program(&compiled, &out).unwrap();
    assert!(out.exists());

    let raw = fs::read(&out).unwrap();
    assert_eq!(&raw[raw.len() - 15..], pkg::StandaloneBundler::MAGIC_TRAILER);

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_full_regression_suite() {
    let code = "fn fact(n: Int) -> Int { if n <= 1 { return 1 } return n * fact(n - 1) }\nreturn fact(5)";
    let compiled = compiler::Compiler::compile(code, "<input>").unwrap();
    let global_count = compiled.global_count as usize;
    let mut rt = runtime::Runtime::with_types(compiled.code_objects, compiled.type_descriptors);
    rt.set_global_count(global_count);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(120));
}

#[test]
fn test_dependency_graph_fuzz_stress() {
    let mut solver = DependencyResolver::new();

    for i in 0..20 {
        let name = format!("pkg_{}", i);
        let mut deps = HashMap::new();
        if i > 0 {
            deps.insert(format!("pkg_{}", i - 1), "^1.0.0".to_string());
        }
        solver.add_available_package(AvailablePackage {
            name: name.clone(),
            version: Version::new(1, 0, 0),
            dependencies: deps,
            source: "registry".into(),
        });
    }

    let manifest_toml = r#"
[package]
name = "root"
version = "1.0.0"

[dependencies]
pkg_19 = "^1.0.0"
"#;
    let manifest = PackageManifest::from_toml_str(manifest_toml).unwrap();
    let resolved = solver.resolve(&manifest).unwrap();
    assert_eq!(resolved.len(), 20);
}

fn app_with_util(test_name: &str) -> PathBuf {
    let root = setup_temp_dir(test_name);
    let util = root.join("util");
    fs::create_dir_all(util.join("src").join("geo")).unwrap();
    fs::write(util.join("mote.toml"), "[package]\nname = \"util\"\nversion = \"0.2.0\"\n").unwrap();
    fs::write(util.join("src").join("lib.mote"), "pub fn one() -> Int { return 1 }\n").unwrap();
    fs::write(util.join("src").join("geo").join("area.mote"), "pub fn sq(x: Int) -> Int { return x * x }\n").unwrap();
    let app = root.join("app");
    PackageManager::init_project(&app, Some("app"), false).unwrap();
    fs::write(app.join("mote.toml"), "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nutil = { version = \"^0.2.0\", path = \"../util\" }\n").unwrap();
    app
}

#[test]
fn test_lockfile_checksums_path_dependencies() {
    let app = app_with_util("lock_sum");
    PackageManager::install_dependencies(&app, false).unwrap();
    let lock = Lockfile::from_file(&app.join("mote.lock")).unwrap();
    let sum = lock.packages[0].checksum.clone().expect("path dependency is checksummed");
    assert!(sum.starts_with("sha256:") && sum.len() == 7 + 64, "{sum}");
    assert!(app.join(".mote_packages/util/src/geo/area.mote").exists(), "nested files are installed");
    assert!(!fs::read_to_string(app.join(".gitignore")).unwrap().contains("mote.lock"));

    PackageManager::install_dependencies(&app, true).unwrap();
    PackageManager::install_dependencies(&app, false).unwrap();
    assert_eq!(Lockfile::from_file(&app.join("mote.lock")).unwrap().packages[0].checksum, Some(sum.clone()));

    fs::write(app.join("../util/src/geo/area.mote"), "pub fn sq(x: Int) -> Int { return x * x * 1 }\n").unwrap();
    let err = PackageManager::install_dependencies(&app, true).unwrap_err();
    assert!(err.contains("out of date (util changed)"), "{err}");
    assert_eq!(Lockfile::from_file(&app.join("mote.lock")).unwrap().packages[0].checksum, Some(sum.clone()));
    PackageManager::install_dependencies(&app, false).unwrap();
    let relocked = Lockfile::from_file(&app.join("mote.lock")).unwrap().packages[0].checksum.clone().unwrap();
    assert_ne!(relocked, sum);
    PackageManager::install_dependencies(&app, true).unwrap();

    fs::remove_dir_all(app.parent().unwrap()).ok();
}

#[test]
fn test_locked_install_needs_an_up_to_date_lock() {
    let app = app_with_util("lock_need");
    let err = PackageManager::install_dependencies(&app, true).unwrap_err();
    assert!(err.contains("mote.lock does not exist"), "{err}");
    PackageManager::install_dependencies(&app, false).unwrap();
    fs::write(app.join("../util/mote.toml"), "[package]\nname = \"util\"\nversion = \"0.2.1\"\n").unwrap();
    let err = PackageManager::install_dependencies(&app, true).unwrap_err();
    assert!(err.contains("mote.lock is out of date;"), "{err}");

    fs::remove_dir_all(app.parent().unwrap()).ok();
}

#[test]
fn test_missing_path_dependency_is_an_error() {
    let app = app_with_util("lock_missing");
    fs::remove_dir_all(app.join("../util/src")).unwrap();
    let err = PackageManager::install_dependencies(&app, false).unwrap_err();
    assert!(err.contains("path dependency 'util'") && err.contains("is not a directory"), "{err}");

    fs::remove_dir_all(app.parent().unwrap()).ok();
}

#[test]
fn test_package_writes_a_reproducible_mpk() {
    let temp = setup_temp_dir("mpk");
    fs::create_dir_all(temp.join("src").join("shapes")).unwrap();
    fs::write(temp.join("mote.toml"), "[package]\nname = \"geo\"\nversion = \"1.2.3\"\nentry = \"src/lib.mote\"\n").unwrap();
    fs::write(
        temp.join("src").join("lib.mote"),
        "pub struct P { x: Int }\npub fn area(w: Int, h: Int) -> Int { return w * h }\nfn hidden() -> Int { return 1 }\n",
    )
    .unwrap();
    fs::write(temp.join("src").join("shapes").join("sq.mote"), "pub fn sq(x: Int) -> Int { return x * x }\n").unwrap();

    let path = PackageManager::package(&temp).unwrap();
    assert!(path.ends_with("dist/geo-1.2.3.mpk"), "{}", path.display());
    let first = fs::read(&path).unwrap();
    assert_eq!(first, fs::read(PackageManager::package(&temp).unwrap()).unwrap(), "same package, same bytes");

    let entries = pkg::MpkArchive::read(&path).unwrap();
    let names: Vec<&str> = entries.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(names, ["geo.mbc", "interface.txt", "mote.toml", "src/lib.mote", "src/shapes/sq.mote"]);
    let mbc = &entries[0].1;
    assert!(pkg::MbcFile::decode(mbc).is_ok());
    let interface = String::from_utf8(entries[1].1.clone()).unwrap();
    assert_eq!(
        interface,
        "// src/lib.mote\nstruct P { x: Int }\nfn area(w: Int, h: Int) -> Int\n// src/shapes/sq.mote\nfn sq(x: Int) -> Int\n"
    );

    fs::remove_dir_all(temp).ok();
}
