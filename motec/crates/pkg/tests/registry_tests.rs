//! Registry fetch through `mote install` against a `file://` registry.

use std::fs;
use std::path::{Path, PathBuf};

use pkg::registry::archive_checksum;
use pkg::{IndexEntry, IndexFile, Lockfile, MpkArchive, PackageManager, Version};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mote_registry_{name}_{}", std::process::id()));
    fs::remove_dir_all(&dir).ok();
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn publish_entries(reg: &Path, name: &str, version: &str, entries: Vec<(String, Vec<u8>)>, deps: &[(&str, &str)]) -> String {
    let bytes = MpkArchive::encode(&entries).unwrap();
    let dir = reg.join("packages").join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{version}.mpk")), &bytes).unwrap();
    let index_path = reg.join("index").join(format!("{name}.toml"));
    fs::create_dir_all(index_path.parent().unwrap()).unwrap();
    let mut index: IndexFile = fs::read_to_string(&index_path).map(|t| toml::from_str(&t).unwrap()).unwrap_or_default();
    let checksum = archive_checksum(&bytes);
    index.versions.push(IndexEntry {
        version: version.parse().unwrap(),
        checksum: checksum.clone(),
        dependencies: deps.iter().map(|(n, r)| (n.to_string(), r.to_string())).collect(),
        yanked: false,
    });
    fs::write(&index_path, toml::to_string(&index).unwrap()).unwrap();
    checksum
}

fn publish(reg: &Path, name: &str, version: &str, lib: &str, deps: &[(&str, &str)]) -> String {
    let manifest = format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\nentry = \"src/lib.mote\"\n");
    let entries = vec![
        ("mote.toml".to_string(), manifest.into_bytes()),
        ("src/lib.mote".to_string(), lib.as_bytes().to_vec()),
        (format!("{name}.mbc"), b"ignored".to_vec()),
    ];
    publish_entries(reg, name, version, entries, deps)
}

fn app(root: &Path, deps: &str) -> PathBuf {
    let app = root.join("app");
    fs::create_dir_all(app.join("src")).unwrap();
    let url = format!("file://{}", root.join("reg").display());
    fs::write(
        app.join("mote.toml"),
        format!("[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n{deps}\n[registry]\nurl = \"{url}\"\n"),
    )
    .unwrap();
    app
}

fn locked_version(app: &Path, name: &str) -> Version {
    let lock = Lockfile::from_file(&app.join("mote.lock")).unwrap();
    lock.packages.iter().find(|p| p.name == name).unwrap().version.clone()
}

#[test]
fn install_fetches_verifies_and_unpacks_transitive_packages() {
    let root = temp("fetch");
    let reg = root.join("reg");
    let shapes = publish(&reg, "shapes", "1.0.0", "pub fn sq(x: Int) -> Int { return x * x }\n", &[]);
    publish(&reg, "geo", "0.3.0", "import shapes\npub fn area(x: Int) -> Int { return shapes.sq(x) }\n", &[("shapes", "^1.0.0")]);
    let app = app(&root, "geo = \"^0.3.0\"\n");
    fs::write(app.join("src/main.mote"), "import geo\nfn main() -> Int { return geo.area(7) }\nreturn main()\n").unwrap();

    PackageManager::install_dependencies(&app, false).unwrap();
    let lock = Lockfile::from_file(&app.join("mote.lock")).unwrap();
    let names: Vec<&str> = lock.packages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["geo", "shapes"]);
    let s = &lock.packages[1];
    assert_eq!(s.source, format!("registry+file://{}", reg.display()));
    assert_eq!(s.checksum.as_deref(), Some(shapes.as_str()));
    assert!(app.join(".mote_packages/shapes/src/lib.mote").is_file());
    assert!(app.join(".mote_packages/shapes/mote.toml").is_file());
    assert!(!app.join(".mote_packages/shapes/shapes.mbc").exists(), "the .mbc is not unpacked");
    assert_eq!(fs::read_to_string(app.join(".mote_packages/shapes/.checksum")).unwrap(), shapes);

    PackageManager::install_dependencies(&app, true).unwrap();
    let (_, compiled) = PackageManager::compile_program(&app).unwrap();
    let mut types = isa::value::TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = types.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &types);
    assert_eq!(rt.run_entry().unwrap().registers[0].as_int(), Some(49));

    fs::remove_dir_all(root).ok();
}

#[test]
fn install_keeps_the_locked_version_and_skips_yanked_ones() {
    let root = temp("pin");
    let reg = root.join("reg");
    publish(&reg, "util", "1.0.0", "pub fn v() -> Int { return 1 }\n", &[]);
    let app = app(&root, "util = \"^1.0.0\"\n");
    PackageManager::install_dependencies(&app, false).unwrap();
    assert_eq!(locked_version(&app, "util"), Version::new(1, 0, 0));

    publish(&reg, "util", "1.1.0", "pub fn v() -> Int { return 2 }\n", &[]);
    PackageManager::install_dependencies(&app, false).unwrap();
    assert_eq!(locked_version(&app, "util"), Version::new(1, 0, 0));
    fs::remove_file(app.join("mote.lock")).unwrap();
    PackageManager::install_dependencies(&app, false).unwrap();
    assert_eq!(locked_version(&app, "util"), Version::new(1, 1, 0));
    assert!(fs::read_to_string(app.join(".mote_packages/util/src/lib.mote")).unwrap().contains("return 2"));

    let index_path = reg.join("index/util.toml");
    let mut index: IndexFile = toml::from_str(&fs::read_to_string(&index_path).unwrap()).unwrap();
    index.versions[1].yanked = true;
    fs::write(&index_path, toml::to_string(&index).unwrap()).unwrap();
    PackageManager::install_dependencies(&app, true).unwrap();
    fs::remove_file(app.join("mote.lock")).unwrap();
    PackageManager::install_dependencies(&app, false).unwrap();
    assert_eq!(locked_version(&app, "util"), Version::new(1, 0, 0));

    fs::remove_dir_all(root).ok();
}

#[test]
fn checksum_differences_always_fail() {
    let root = temp("tamper");
    let reg = root.join("reg");
    publish(&reg, "util", "1.0.0", "pub fn v() -> Int { return 1 }\n", &[]);
    let app = app(&root, "util = \"^1.0.0\"\n");
    PackageManager::install_dependencies(&app, false).unwrap();

    let good = fs::read(reg.join("packages/util/1.0.0.mpk")).unwrap();
    let other = MpkArchive::encode(&[("mote.toml".to_string(), b"[package]\nname = \"util\"\nversion = \"1.0.0\"\n".to_vec())]).unwrap();
    fs::write(reg.join("packages/util/1.0.0.mpk"), &other).unwrap();
    fs::remove_dir_all(app.join(".mote_packages")).unwrap();
    let err = PackageManager::install_dependencies(&app, false).unwrap_err();
    assert!(err.contains("downloaded archive has checksum"), "{err}");
    assert!(!app.join(".mote_packages/util").exists());

    fs::write(reg.join("packages/util/1.0.0.mpk"), &other).unwrap();
    let index_path = reg.join("index/util.toml");
    let text = fs::read_to_string(&index_path).unwrap();
    fs::write(&index_path, text.replace(&archive_checksum(&good), &archive_checksum(&other))).unwrap();
    for locked in [false, true] {
        let err = PackageManager::install_dependencies(&app, locked).unwrap_err();
        assert!(err.contains("the published archive changed"), "{err}");
    }

    fs::remove_dir_all(root).ok();
}

#[test]
fn unsafe_or_mislabelled_archives_are_rejected() {
    let root = temp("unsafe");
    let reg = root.join("reg");
    let toml = |name: &str| format!("[package]\nname = \"{name}\"\nversion = \"1.0.0\"\n").into_bytes();
    publish_entries(&reg, "evil", "1.0.0", vec![("mote.toml".into(), toml("evil")), ("src/../../x.mote".into(), b"x".to_vec())], &[]);
    publish_entries(&reg, "liar", "1.0.0", vec![("mote.toml".into(), toml("other"))], &[]);
    publish_entries(&reg, "bare", "1.0.0", vec![("src/lib.mote".into(), b"x".to_vec())], &[]);

    for (name, expect) in [("evil", "not a safe relative path"), ("liar", "archive's mote.toml is other 1.0.0"), ("bare", "has no mote.toml")] {
        let app = app(&root, &format!("{name} = \"^1.0.0\"\n"));
        let err = PackageManager::install_dependencies(&app, false).unwrap_err();
        assert!(err.contains(expect), "{name}: {err}");
        assert!(!app.join(".mote_packages").join(name).exists(), "{name}: nothing written");
        assert!(!root.join("x.mote").exists());
        fs::remove_dir_all(&app).unwrap();
    }

    fs::remove_dir_all(root).ok();
}

#[test]
fn add_pins_the_latest_version_and_names_are_checked() {
    let root = temp("add");
    let reg = root.join("reg");
    publish(&reg, "util", "1.2.0", "pub fn v() -> Int { return 1 }\n", &[]);
    publish(&reg, "util", "1.10.0", "pub fn v() -> Int { return 2 }\n", &[]);
    let app = app(&root, "");
    PackageManager::add_dependency(&app, "util").unwrap();
    assert!(fs::read_to_string(app.join("mote.toml")).unwrap().contains("util = \"^1.10.0\""));
    assert_eq!(locked_version(&app, "util"), Version::new(1, 10, 0));

    let err = PackageManager::add_dependency(&app, "Bad-Name@^1.0.0").unwrap_err();
    assert!(err.contains("not a valid registry package name"), "{err}");
    assert!(!fs::read_to_string(app.join("mote.toml")).unwrap().contains("Bad-Name"), "mote.toml is restored");
    let err = PackageManager::add_dependency(&app, "missing").unwrap_err();
    assert!(err.contains("package 'missing' is not in the registry"), "{err}");
    assert!(pkg::Registry::new("http://example.org").unwrap_err().contains("https://"));

    fs::remove_dir_all(root).ok();
}

#[test]
fn prereleases_resolve_only_when_asked_for() {
    let root = temp("pre");
    let reg = root.join("reg");
    publish(&reg, "util", "1.0.0", "pub fn v() -> Int { return 1 }\n", &[]);
    publish(&reg, "util", "1.1.0-beta.1", "pub fn v() -> Int { return 2 }\n", &[]);
    let app_dir = app(&root, "util = \"^1.0.0\"\n");
    PackageManager::install_dependencies(&app_dir, false).unwrap();
    assert_eq!(locked_version(&app_dir, "util"), Version::new(1, 0, 0));
    fs::remove_dir_all(&app_dir).unwrap();

    let app_dir = app(&root, "util = \"^1.1.0-beta.1\"\n");
    PackageManager::install_dependencies(&app_dir, false).unwrap();
    assert_eq!(locked_version(&app_dir, "util").to_string(), "1.1.0-beta.1");
    assert!(fs::read_to_string(app_dir.join("mote.lock")).unwrap().contains("version = \"1.1.0-beta.1\""));
    fs::remove_dir_all(&app_dir).unwrap();

    let app_dir = app(&root, "");
    PackageManager::add_dependency(&app_dir, "util").unwrap();
    assert!(fs::read_to_string(app_dir.join("mote.toml")).unwrap().contains("util = \"^1.0.0\""));

    fs::remove_dir_all(root).ok();
}
