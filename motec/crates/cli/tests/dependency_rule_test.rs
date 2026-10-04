
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const CONTRACT_CRATES: &[&str] = &["isa", "contracts"];
const COMPOSITION_ROOT: &str = "cli";

const KNOWN_VIOLATIONS: &[(&str, &str, &str, &str)] = &[
    ("ffi", "runtime", "dependencies", "A1/A4: natives implement contracts, not the runtime"),
    ("ffi", "platform", "dependencies", "N2: natives are resolved by name and the composition root supplies the platform"),
    ("ffi", "gui", "dependencies", "G3: the window natives drive the gui crate"),
    ("ffi", "cli", "dev-dependencies", "A0 follow-up: the cli-level test moves into cli"),
    ("modules", "compiler", "dependencies", "A7: compiler stage contracts"),
    ("pkg", "compiler", "dependencies", "A7: compiler stage contracts"),
    ("pkg", "modules", "dependencies", "A7: compiler stage contracts"),
    ("modules", "ffi", "dev-dependencies", "A5/A7"),
    ("modules", "gc", "dev-dependencies", "A2/A7"),
    ("pkg", "runtime", "dev-dependencies", "A7"),
    ("pkg", "ffi", "dev-dependencies", "run-a-program tests move into cli"),
    ("modules", "runtime", "dev-dependencies", "run-a-program tests move into cli"),
    ("compiler", "runtime", "dev-dependencies", "run-a-program tests move into cli"),
    ("compiler", "gc", "dev-dependencies", "run-a-program tests move into cli"),
    ("compiler", "ffi", "dev-dependencies", "run-a-program tests move into cli"),
];

fn workspace_edges() -> BTreeSet<(String, String, String)> {
    let crates_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let names: BTreeSet<String> = fs::read_dir(&crates_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("Cargo.toml").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();

    let mut edges = BTreeSet::new();
    for from in &names {
        let manifest = fs::read_to_string(crates_dir.join(from).join("Cargo.toml")).unwrap();
        let mut section = String::new();
        for line in manifest.lines() {
            let line = line.trim();
            if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = header.to_string();
                continue;
            }
            if !matches!(
                section.as_str(),
                "dependencies" | "dev-dependencies" | "build-dependencies"
            ) {
                continue;
            }
            if let Some((dep, rest)) = line.split_once('=') {
                let dep = dep.trim();
                if rest.contains("path") && names.contains(dep) {
                    edges.insert((from.clone(), dep.to_string(), section.clone()));
                }
            }
        }
    }
    edges
}

fn is_violation(from: &str, to: &str) -> bool {
    from != COMPOSITION_ROOT && !CONTRACT_CRATES.contains(&to)
}

#[test]
fn test_dependency_rule_holds_except_for_the_known_violations() {
    let violations: BTreeSet<(String, String, String)> = workspace_edges()
        .into_iter()
        .filter(|(from, to, _)| is_violation(from, to))
        .collect();
    let known: BTreeSet<(String, String, String)> = KNOWN_VIOLATIONS
        .iter()
        .map(|(f, t, k, _)| (f.to_string(), t.to_string(), k.to_string()))
        .collect();

    let new: Vec<_> = violations.difference(&known).collect();
    assert!(
        new.is_empty(),
        "new dependency-rule violation(s) (from, to, kind): {new:?}\n\
         Crates may depend only on the contracts layer ({CONTRACT_CRATES:?}); \
         only `{COMPOSITION_ROOT}` may name other implementation crates."
    );

    let fixed: Vec<_> = known.difference(&violations).collect();
    assert!(
        fixed.is_empty(),
        "these known violations no longer exist — delete them from KNOWN_VIOLATIONS \
         so the ratchet tightens: {fixed:?}"
    );
}

#[test]
fn test_the_contracts_layer_depends_only_on_itself() {
    for c in CONTRACT_CRATES {
        let deps: Vec<_> = workspace_edges()
            .into_iter()
            .filter(|(f, t, _)| f == c && !CONTRACT_CRATES.contains(&t.as_str()))
            .collect();
        assert!(deps.is_empty(), "contracts-layer crate `{c}` must not depend on: {deps:?}");
    }
}
