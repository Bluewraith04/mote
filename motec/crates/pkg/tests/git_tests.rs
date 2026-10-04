//! Git packages through `mote install` and `mote add`, against repositories on disk.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use pkg::{Lockfile, PackageManager};

struct Hosts {
    root: PathBuf,
}

fn hosts(name: &str) -> Hosts {
    let root = std::env::temp_dir().join(format!("mote_git_{name}_{}", std::process::id()));
    fs::remove_dir_all(&root).ok();
    fs::create_dir_all(&root).unwrap();
    Hosts { root }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

impl Hosts {
    /// The URL of repository `repo`.
    fn url(&self, repo: &str) -> String {
        format!("file://{}", self.root.join(repo).display())
    }

    /// Commits `files` as the whole tree of `repo` on `main`, tags it `tag` when given, and returns the commit.
    fn commit(&self, repo: &str, tag: Option<&str>, files: &[(&str, &str)]) -> String {
        let dir = self.root.join(repo);
        if !dir.join(".git").exists() {
            fs::create_dir_all(&dir).unwrap();
            git(&dir, &["init", "-q", "-b", "main"]);
        }
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.file_name().is_some_and(|n| n == ".git") {
                continue;
            }
            if path.is_dir() { fs::remove_dir_all(&path).unwrap() } else { fs::remove_file(&path).unwrap() }
        }
        for (path, content) in files {
            let full = dir.join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, content).unwrap();
        }
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-q", "--allow-empty", "-m", "c"]);
        if let Some(tag) = tag {
            git(&dir, &["tag", "-a", "-m", tag, tag]);
        }
        git(&dir, &["rev-parse", "HEAD"])
    }

    /// Publishes a package named `repo` at `tag` with a `lib.mote` and dependencies written as `deps` lines.
    fn package(&self, repo: &str, tag: &str, lib: &str, deps: &str) -> String {
        let version = tag.trim_start_matches('v');
        let manifest = format!("[package]\nname = \"{repo}\"\nversion = \"{version}\"\nentry = \"src/lib.mote\"\n\n[dependencies]\n{deps}");
        self.commit(repo, Some(tag), &[("mote.toml", &manifest), ("src/lib.mote", lib), ("README.md", "ignored")])
    }

    fn app(&self, deps: &str) -> PathBuf {
        let app = self.root.join("app");
        fs::remove_dir_all(&app).ok();
        fs::create_dir_all(app.join("src")).unwrap();
        fs::write(app.join("mote.toml"), format!("[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n{deps}")).unwrap();
        app
    }

    fn dep(&self, repo: &str, tag: &str) -> String {
        format!("{{ git = \"{}\", tag = \"{tag}\" }}", self.url(repo))
    }
}

#[test]
fn install_fetches_locks_and_follows_dependencies() {
    let h = hosts("fetch");
    let shapes = h.package("shapes", "v1.0.0", "pub fn sq(x: Int) -> Int { return x * x }\n", "");
    let deps = format!("shapes = {}\n", h.dep("shapes", "v1.0.0"));
    h.package("geo", "v0.3.0", "import shapes\npub fn area(x: Int) -> Int { return shapes.sq(x) }\n", &deps);
    let app = h.app(&format!("geo = {}\n", h.dep("geo", "v0.3.0")));
    fs::write(app.join("src/main.mote"), "import geo\nfn main() -> Int { return geo.area(7) }\nreturn main()\n").unwrap();

    PackageManager::install_dependencies(&app, false).unwrap();
    let lock = Lockfile::from_file(&app.join("mote.lock")).unwrap();
    let names: Vec<&str> = lock.packages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["geo", "shapes"]);
    assert_eq!(lock.packages[1].source, format!("git+{}?tag=v1.0.0#{shapes}", h.url("shapes")));
    assert!(lock.packages[1].checksum.as_deref().is_some_and(|c| c.starts_with("sha256:")));
    assert!(app.join(".mote_packages/shapes/src/lib.mote").is_file());
    assert!(app.join(".mote_packages/shapes/mote.toml").is_file());
    assert!(!app.join(".mote_packages/shapes/README.md").exists(), "only mote.toml and src/ are unpacked");

    PackageManager::install_dependencies(&app, true).unwrap();
    let (_, compiled) = PackageManager::compile_program(&app).unwrap();
    let mut types = isa::value::TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = types.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &types);
    assert_eq!(rt.run_entry().unwrap().registers[0].as_int(), Some(49));
    fs::remove_dir_all(&h.root).ok();
}

#[test]
fn the_lock_keeps_a_branch_on_its_commit() {
    let h = hosts("branch");
    let manifest = "[package]\nname = \"util\"\nversion = \"0.1.0\"\n";
    let first = h.commit("util", None, &[("mote.toml", manifest), ("src/lib.mote", "pub fn v() -> Int { return 1 }\n")]);
    let url = h.url("util");
    let app = h.app(&format!("util = {{ git = \"{url}\", branch = \"main\" }}\n"));
    PackageManager::install_dependencies(&app, false).unwrap();
    let source = |app: &Path| Lockfile::from_file(&app.join("mote.lock")).unwrap().packages[0].source.clone();
    assert_eq!(source(&app), format!("git+{url}?branch=main#{first}"));

    let second = h.commit("util", None, &[("mote.toml", manifest), ("src/lib.mote", "pub fn v() -> Int { return 2 }\n")]);
    PackageManager::install_dependencies(&app, false).unwrap();
    assert_eq!(source(&app), format!("git+{url}?branch=main#{first}"));
    assert!(fs::read_to_string(app.join(".mote_packages/util/src/lib.mote")).unwrap().contains("return 1"));

    fs::remove_file(app.join("mote.lock")).unwrap();
    PackageManager::install_dependencies(&app, false).unwrap();
    assert_eq!(source(&app), format!("git+{url}?branch=main#{second}"));
    assert!(fs::read_to_string(app.join(".mote_packages/util/src/lib.mote")).unwrap().contains("return 2"));
    fs::remove_dir_all(&h.root).ok();
}

#[test]
fn a_rev_names_one_commit() {
    let h = hosts("rev");
    let manifest = "[package]\nname = \"util\"\nversion = \"0.1.0\"\n";
    let first = h.commit("util", None, &[("mote.toml", manifest), ("src/lib.mote", "pub fn v() -> Int { return 1 }\n")]);
    h.commit("util", None, &[("mote.toml", manifest), ("src/lib.mote", "pub fn v() -> Int { return 2 }\n")]);
    let app = h.app(&format!("util = {{ git = \"{}\", rev = \"{first}\" }}\n", h.url("util")));
    PackageManager::install_dependencies(&app, false).unwrap();
    assert!(fs::read_to_string(app.join(".mote_packages/util/src/lib.mote")).unwrap().contains("return 1"));
    fs::remove_dir_all(&h.root).ok();
}

#[test]
fn locked_never_resolves_and_a_changed_tree_always_fails() {
    let h = hosts("tamper");
    h.package("util", "v1.0.0", "pub fn v() -> Int { return 1 }\n", "");
    let app = h.app(&format!("util = {}\n", h.dep("util", "v1.0.0")));

    let err = PackageManager::install_dependencies(&app, true).unwrap_err();
    assert!(err.contains("--locked: mote.lock is out of date"), "{err}");
    assert!(!app.join(".mote_packages/util").exists());

    PackageManager::install_dependencies(&app, false).unwrap();
    let lock = fs::read_to_string(app.join("mote.lock")).unwrap();
    let at = lock.find("sha256:").unwrap() + "sha256:".len();
    let flipped = if lock[at..].starts_with('0') { '1' } else { '0' };
    fs::write(app.join("mote.lock"), format!("{}{flipped}{}", &lock[..at], &lock[at + 1..])).unwrap();
    fs::remove_dir_all(app.join(".mote_packages")).unwrap();
    for locked in [false, true] {
        let err = PackageManager::install_dependencies(&app, locked).unwrap_err();
        assert!(err.contains("the repository changed"), "{err}");
        assert!(!app.join(".mote_packages/util").exists());
    }

    h.package("other", "v1.0.0", "pub fn v() -> Int { return 1 }\n", "");
    let app = h.app(&format!("other = {}\n", h.dep("other", "v1.0.0")));
    PackageManager::install_dependencies(&app, false).unwrap();
    fs::write(app.join(".mote_packages/other/src/lib.mote"), b"edited").unwrap();
    PackageManager::install_dependencies(&app, true).unwrap();
    assert!(fs::read_to_string(app.join(".mote_packages/other/src/lib.mote")).unwrap().contains("return 1"), "a hand-edited package is fetched again");
    fs::remove_dir_all(&h.root).ok();
}

#[test]
fn a_mislabelled_or_manifestless_repository_is_rejected() {
    let h = hosts("mislabelled");
    let toml = |name: &str| format!("[package]\nname = \"{name}\"\nversion = \"1.0.0\"\n");
    h.commit("liar", Some("v1.0.0"), &[("mote.toml", &toml("other"))]);
    h.commit("bare", Some("v1.0.0"), &[("src/lib.mote", "x")]);

    for (name, expect) in [("liar", "names the package other"), ("bare", "has no mote.toml")] {
        let app = h.app(&format!("{name} = {}\n", h.dep(name, "v1.0.0")));
        let err = PackageManager::install_dependencies(&app, false).unwrap_err();
        assert!(err.contains(expect), "{name}: {err}");
        assert!(!app.join(".mote_packages").join(name).exists(), "{name}: nothing written");
        fs::remove_dir_all(&app).unwrap();
    }
    fs::remove_dir_all(&h.root).ok();
}

#[test]
fn a_name_pinned_twice_and_a_missing_source_are_errors() {
    let h = hosts("pins");
    h.package("c", "v1.0.0", "pub fn v() -> Int { return 1 }\n", "");
    h.package("c", "v2.0.0", "pub fn v() -> Int { return 2 }\n", "");
    h.package("a", "v1.0.0", "pub fn v() -> Int { return 1 }\n", &format!("c = {}\n", h.dep("c", "v1.0.0")));
    h.package("b", "v1.0.0", "pub fn v() -> Int { return 1 }\n", &format!("c = {}\n", h.dep("c", "v2.0.0")));
    let app = h.app(&format!("a = {}\nb = {}\n", h.dep("a", "v1.0.0"), h.dep("b", "v1.0.0")));
    let err = PackageManager::install_dependencies(&app, false).unwrap_err();
    assert!(err.contains(&format!("c is pinned to {}?tag=v", h.url("c"))), "{err}");
    fs::remove_dir_all(&app).unwrap();

    let app = h.app("foo = \"^1.0\"\n");
    let err = PackageManager::install_dependencies(&app, false).unwrap_err();
    assert!(err.contains("foo: a dependency needs a source") && err.contains("https://host/user/repo"), "{err}");
    fs::remove_dir_all(&app).unwrap();

    let url = h.url("c");
    let app = h.app(&format!("foo = {{ git = \"{url}\", tag = \"v1.0.0\", path = \"../foo\" }}\n"));
    let err = PackageManager::install_dependencies(&app, false).unwrap_err();
    assert!(err.contains("a path or a git repository, not both"), "{err}");
    fs::remove_dir_all(&app).unwrap();

    let app = h.app(&format!("foo = {{ git = \"{url}\" }}\n"));
    assert!(PackageManager::install_dependencies(&app, false).unwrap_err().contains("one of tag, branch or rev"));
    fs::remove_dir_all(&app).unwrap();
    fs::remove_dir_all(&h.root).ok();
}

#[test]
fn add_picks_the_latest_release_tag_and_names_the_ref() {
    let h = hosts("add");
    h.package("util", "v1.2.0", "pub fn v() -> Int { return 1 }\n", "");
    h.package("util", "v1.10.0", "pub fn v() -> Int { return 2 }\n", "");
    h.package("util", "v2.0.0-beta.1", "pub fn v() -> Int { return 3 }\n", "");
    let url = h.url("util");
    let app = h.app("");
    PackageManager::add_dependency(&app, &url).unwrap();
    let text = fs::read_to_string(app.join("mote.toml")).unwrap();
    assert!(text.contains(&format!("git = \"{url}\"")) && text.contains("tag = \"v1.10.0\""), "{text}");
    assert!(fs::read_to_string(app.join(".mote_packages/util/src/lib.mote")).unwrap().contains("return 2"));
    let lock = Lockfile::from_file(&app.join("mote.lock")).unwrap();
    assert!(lock.packages[0].source.starts_with(&format!("git+{url}?tag=v1.10.0#")));

    let app = h.app("");
    PackageManager::add_dependency(&app, &format!("{url}@v1.2.0")).unwrap();
    assert!(fs::read_to_string(app.join("mote.toml")).unwrap().contains("tag = \"v1.2.0\""));

    let manifest = "[package]\nname = \"edge\"\nversion = \"0.1.0\"\n";
    let sha = h.commit("edge", None, &[("mote.toml", manifest), ("src/lib.mote", "pub fn v() -> Int { return 1 }\n")]);
    let edge = h.url("edge");
    let app = h.app("");
    PackageManager::add_dependency(&app, &format!("{edge}@main")).unwrap();
    assert!(fs::read_to_string(app.join("mote.toml")).unwrap().contains("branch = \"main\""));
    let app = h.app("");
    PackageManager::add_dependency(&app, &format!("{edge}@{sha}")).unwrap();
    assert!(fs::read_to_string(app.join("mote.toml")).unwrap().contains(&format!("rev = \"{sha}\"")));

    let app = h.app("");
    let missing = h.url("missing");
    for (spec, expect) in [
        ("util".to_string(), "not a git repository"),
        ("logger@^1.0.0".to_string(), "not a git repository"),
        (format!("{url}@nope"), "has no tag or branch 'nope'"),
        (missing, "cannot be read"),
    ] {
        let err = PackageManager::add_dependency(&app, &spec).unwrap_err();
        assert!(err.contains(expect), "{spec}: {err}");
    }
    let text = fs::read_to_string(app.join("mote.toml")).unwrap();
    assert!(!text.contains("git ="), "mote.toml is unchanged: {text}");
    fs::remove_dir_all(&h.root).ok();
}

#[test]
fn a_git_package_cannot_depend_on_a_path() {
    let h = hosts("path_in_git");
    h.package("inner", "v1.0.0", "pub fn v() -> Int { return 1 }\n", "local = { path = \"../local\" }\n");
    let app = h.app(&format!("inner = {}\n", h.dep("inner", "v1.0.0")));
    let err = PackageManager::install_dependencies(&app, false).unwrap_err();
    assert!(err.contains("cannot have the path dependency local"), "{err}");
    fs::remove_dir_all(&h.root).ok();
}
