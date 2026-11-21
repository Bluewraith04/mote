//! The code generator refuses no program: a case it cannot lower is an `internal` error.

use std::path::Path;

/// The words that made a refusal; spelled in parts so this file does not hold them.
fn banned() -> [String; 2] {
    [["unsup", "ported("].concat(), ["not compiled", " yet"].concat()]
}

fn sources(dir: &Path, found: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("the source directory") {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
}

#[test]
fn refusals_no_source_of_the_code_generator_refuses_a_program() {
    let mut files = Vec::new();
    sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    assert!(files.len() > 20);
    let mut found = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("a source file");
        for (number, line) in text.lines().enumerate() {
            if banned().iter().any(|word| line.contains(word.as_str())) {
                found.push(format!("{}:{}", file.display(), number + 1));
            }
        }
    }
    assert_eq!(found, Vec::<String>::new());
}
