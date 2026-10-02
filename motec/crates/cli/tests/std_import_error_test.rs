//! An import of a standard-library path that is not a module says what to write instead.

use std::process::Command;

fn check(source: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_std_import_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("check").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

#[test]
fn bare_std_points_at_a_module() {
    let text = check("import { env } from std\nfn main() { }\n", "bare");
    assert!(text.contains("unknown standard-library module 'std': name one, as in `import std.sys.env`"), "{text}");
}

#[test]
fn a_directory_of_modules_names_the_module_inside() {
    let text = check("import { types } from std.experimental\nfn main() { }\n", "dir");
    assert!(text.contains("did you mean `std.experimental.types`?"), "{text}");
}

#[test]
fn an_old_top_level_path_names_where_the_module_moved() {
    for (old, now) in [("json", "std.data.json"), ("fs", "std.sys.fs"), ("log", "std.dev.log"), ("yaml", "std.data.yaml")] {
        let text = check(&format!("import std.{old}\nfn main() {{ }}\n"), old);
        assert!(text.contains(&format!("unknown standard-library module 'std.{old}': did you mean `{now}`?")), "{text}");
    }
}

#[test]
fn a_member_a_group_lacks_is_an_error() {
    let text = check("import std.data\nfn main() { println(data.nope.x()) }\n", "nomember");
    assert!(text.contains("nope"), "{text}");
    let text = check("import { nope } from std.data\nfn main() { }\n", "nomember_from");
    assert!(text.contains("Cannot import private symbol 'nope'"), "{text}");
}

#[test]
fn a_plain_typo_keeps_the_short_message() {
    let text = check("import std.nope\nfn main() { }\n", "typo");
    assert!(text.contains("unknown standard-library module 'std.nope'") && !text.contains("did you mean"), "{text}");
}
