//! Writes the files of a new package.

use std::fs;
use std::path::Path;

use mote_packages::{is_name, is_reserved, shown};

use crate::pipeline::toolchain;

/// The name the directory gives: its own with every character that is not a letter, digit or `_` made `_`; the directory may not exist yet.
fn name_from_directory(dir: &Path) -> Result<String, String> {
    let real = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let Some(directory) = real.file_name().and_then(|name| name.to_str()) else { return Err(format!("{} has no name", shown(dir))) };
    let name: String = directory.chars().map(|found| if found.is_ascii_alphanumeric() || found == '_' { found } else { '_' }).collect();
    if is_name(&name) && !is_reserved(&name) {
        Ok(name)
    } else {
        Err(format!("`{directory}` is not a package name"))
    }
}

fn manifest_text(name: &str, lib: bool) -> String {
    let entry = if lib { "entry = \"src/lib.mote\"\n" } else { "" };
    format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nmote = \"{}\"\n{entry}\n[dependencies]\n", toolchain())
}

const MAIN_TEXT: &str = "fn main() {\n    println(\"Hello, world!\")\n}\n";

fn lib_text(name: &str) -> String {
    format!("import {{ assert_eq }} from std.test\n\npub fn hello() -> String {{\n    return \"Hello from {name}\"\n}}\n\ntest \"hello\" {{\n    assert_eq(hello(), \"Hello from {name}\")\n}}\n")
}

/// Makes the directory `path` (the current one when absent or `.`) a package named after the directory, and says which files it wrote; an existing manifest is an error, and an existing source file is kept.
pub fn init(cwd: &Path, path: Option<&str>, lib: bool) -> Result<String, String> {
    let (dir, prefix) = match path.map(|path| path.trim_end_matches('/')) {
        None | Some("" | ".") => (cwd.to_path_buf(), String::new()),
        Some(path) => (cwd.join(path), format!("{path}/")),
    };
    let name = name_from_directory(&dir)?;
    let manifest = dir.join("mote.toml");
    if manifest.exists() {
        return Err(format!("{} already exists", shown(&manifest)));
    }
    let source = if lib { "src/lib.mote" } else { "src/main.mote" };
    let write = |path: &Path, text: String| fs::write(path, text).map_err(|error| format!("cannot write {}: {error}", shown(path)));
    fs::create_dir_all(dir.join("src")).map_err(|error| format!("cannot create {}: {error}", shown(&dir.join("src"))))?;
    write(&manifest, manifest_text(&name, lib))?;
    let mut said = format!("created {prefix}mote.toml\n");
    if dir.join(source).exists() {
        said.push_str(&format!("kept {prefix}{source}\n"));
    } else {
        write(&dir.join(source), if lib { lib_text(&name) } else { MAIN_TEXT.to_string() })?;
        said.push_str(&format!("created {prefix}{source}\n"));
    }
    Ok(said)
}

#[cfg(test)]
mod tests {
    use mote_packages::testing::Scratch;
    use mote_packages::Manifest;

    use super::*;
    use crate::cli::{execute, Outcome, FAILED, OK};

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    /// An empty directory named `name` in a scratch directory.
    fn empty(name: &str) -> (Scratch, std::path::PathBuf) {
        let scratch = Scratch::new();
        let dir = scratch.path(name);
        fs::create_dir_all(&dir).unwrap();
        (scratch, dir)
    }

    #[test]
    fn init_writes_a_main_that_prints_one_line_and_a_package_that_checks_and_runs() {
        let (_scratch, dir) = empty("greeter");
        let made = execute(&words("init"), &dir);
        assert_eq!(made, Outcome::ok("created mote.toml\ncreated src/main.mote\n"));
        assert_eq!(fs::read_to_string(dir.join("src/main.mote")).unwrap(), "fn main() {\n    println(\"Hello, world!\")\n}\n");
        assert_eq!(execute(&words("check"), &dir), Outcome::ok("checked greeter 0.1.0\n"));
        assert_eq!(execute(&words("run"), &dir), Outcome::ok("Hello, world!\n"));
        assert_eq!(execute(&words("test"), &dir).code, OK);
    }

    #[test]
    fn init_lib_writes_a_library_that_checks_and_passes_its_test() {
        let (_scratch, dir) = empty("tiles");
        assert_eq!(execute(&words("init --lib"), &dir), Outcome::ok("created mote.toml\ncreated src/lib.mote\n"));
        assert!(!dir.join("src/main.mote").exists());
        assert_eq!(execute(&words("check"), &dir), Outcome::ok("checked tiles 0.1.0\n"));
        let tested = execute(&words("test"), &dir);
        assert_eq!(tested.code, OK);
        assert_eq!(tested.stdout, "test hello (src/lib.mote:7) ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 skipped\n");
        let manifest = Manifest::read(&dir.join("mote.toml")).unwrap();
        assert_eq!(manifest.entry, "src/lib.mote");
    }

    #[test]
    fn init_writes_a_manifest_that_names_the_package_the_version_and_the_toolchain() {
        let (_scratch, dir) = empty("app");
        execute(&words("init"), &dir);
        let text = fs::read_to_string(dir.join("mote.toml")).unwrap();
        assert_eq!(text, format!("[package]\nname = \"app\"\nversion = \"0.1.0\"\nmote = \"{}\"\n\n[dependencies]\n", toolchain()));
        let manifest = Manifest::parse("mote.toml", &text).unwrap();
        assert_eq!((manifest.name.as_str(), manifest.entry.as_str()), ("app", "src/main.mote"));
        assert_eq!(manifest.mote, Some(toolchain()));
        assert_eq!(manifest.check_toolchain("mote.toml", &toolchain()), Ok(()));
    }

    #[test]
    fn init_with_a_path_makes_that_directory_a_package_named_after_it() {
        let (_scratch, dir) = empty("somewhere");
        assert_eq!(execute(&words("init hello"), &dir), Outcome::ok("created hello/mote.toml\ncreated hello/src/main.mote\n"));
        assert!(!dir.join("mote.toml").exists());
        let made = dir.join("hello");
        assert_eq!(Manifest::read(&made.join("mote.toml")).unwrap().name, "hello");
        assert_eq!(execute(&words("run"), &made).stdout, "Hello, world!\n");
    }

    #[test]
    fn init_with_a_dot_makes_the_current_directory_a_package() {
        let (_scratch, dir) = empty("here");
        assert_eq!(execute(&words("init ."), &dir), Outcome::ok("created mote.toml\ncreated src/main.mote\n"));
        assert_eq!(Manifest::read(&dir.join("mote.toml")).unwrap().name, "here");
    }

    #[test]
    fn init_with_a_path_goes_into_a_directory_that_is_already_there() {
        let (_scratch, dir) = empty("somewhere");
        fs::create_dir_all(dir.join("old")).unwrap();
        assert_eq!(execute(&words("init old/"), &dir), Outcome::ok("created old/mote.toml\ncreated old/src/main.mote\n"));
    }

    #[test]
    fn init_makes_a_name_of_a_directory_name_with_dashes_and_dots() {
        let (_scratch, dir) = empty("my-app.v2");
        execute(&words("init"), &dir);
        assert_eq!(Manifest::read(&dir.join("mote.toml")).unwrap().name, "my_app_v2");
    }

    #[test]
    fn init_refuses_a_directory_name_that_is_not_one() {
        for bad in ["9lives", "std"] {
            let (_scratch, dir) = empty(bad);
            let outcome = execute(&words("init"), &dir);
            assert_eq!((outcome.code, outcome.stderr.as_str()), (FAILED, format!("error: `{bad}` is not a package name\n").as_str()), "{bad}");
            assert!(!dir.join("mote.toml").exists());
        }
    }

    #[test]
    fn init_refuses_a_path_that_is_not_a_name_and_makes_no_directory() {
        let (_scratch, dir) = empty("here");
        let outcome = execute(&words("init 9x"), &dir);
        assert_eq!((outcome.code, outcome.stderr.as_str()), (FAILED, "error: `9x` is not a package name\n"));
        assert_eq!(execute(&words("init prelude"), &dir).stderr, "error: `prelude` is not a package name\n");
        assert!(!dir.join("9x").exists() && !dir.join("prelude").exists());
        assert_eq!(execute(&words("init a-b"), &dir), Outcome::ok("created a-b/mote.toml\ncreated a-b/src/main.mote\n"));
        assert_eq!(Manifest::read(&dir.join("a-b/mote.toml")).unwrap().name, "a_b");
    }

    #[test]
    fn init_does_not_touch_a_directory_that_has_a_manifest() {
        let (_scratch, dir) = empty("old");
        fs::write(dir.join("mote.toml"), "mine").unwrap();
        let outcome = execute(&words("init"), &dir);
        assert_eq!((outcome.code, outcome.stderr.clone()), (FAILED, format!("error: {}/mote.toml already exists\n", dir.display())));
        assert_eq!(fs::read_to_string(dir.join("mote.toml")).unwrap(), "mine");
        assert!(!dir.join("src").exists());
    }

    #[test]
    fn init_keeps_a_source_file_that_is_already_there() {
        let (_scratch, dir) = empty("keep");
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/main.mote"), "fn main() {\n    println(\"mine\")\n}\n").unwrap();
        assert_eq!(execute(&words("init"), &dir), Outcome::ok("created mote.toml\nkept src/main.mote\n"));
        assert_eq!(execute(&words("run"), &dir).stdout, "mine\n");
    }

    #[test]
    fn init_leaves_other_files_of_the_directory_alone() {
        let (_scratch, dir) = empty("busy");
        fs::write(dir.join("notes.txt"), "notes").unwrap();
        execute(&words("init"), &dir);
        assert_eq!(fs::read_to_string(dir.join("notes.txt")).unwrap(), "notes");
    }

    #[test]
    fn init_takes_at_most_one_path() {
        let (_scratch, dir) = empty("here");
        assert_eq!(execute(&words("init a b"), &dir).code, crate::cli::USAGE);
    }
}
