//! What the integration tests share: where the repository is and which files it holds.

#![allow(dead_code)]

pub mod blocks;
pub mod expect;

use std::path::{Path, PathBuf};

/// The line that opens a generated table, followed by its name and ` -->`.
pub const TABLE_OPEN: &str = "<!-- table: ";

/// The line that closes a generated table.
pub const TABLE_CLOSE: &str = "<!-- /table -->";

/// The root of the repository.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// The programs of a corpus group, as `(name, source)` in name order.
pub fn corpus(group: &str) -> Vec<(String, String)> {
    let dir = root().join("integration/corpus").join(group);
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap_or_else(|error| panic!("{}: {error}", dir.display())).map(|entry| entry.expect("a directory entry").path()).collect();
    files.sort();
    files.iter().filter(|path| path.extension().is_some_and(|ext| ext == "mote")).map(|path| (path.file_stem().expect("a file name").to_string_lossy().into_owned(), std::fs::read_to_string(path).expect("a readable program"))).collect()
}

/// The pushes a program asks for in its `// feed PORT MS BYTES` and `// close PORT MS` lines.
pub fn feeds_of(source: &str) -> Vec<mote::Feed> {
    source
        .lines()
        .filter_map(|line| {
            let words: Vec<&str> = line.strip_prefix("// ")?.split(' ').collect();
            let (port, at_ms) = (words.get(1)?.parse().ok()?, words.get(2)?.parse().ok()?);
            match words[0] {
                "feed" => Some(mote::Feed { port, at_ms, bytes: Some(words.get(3)?.split(',').map(|byte| byte.parse().unwrap()).collect()) }),
                "close" => Some(mote::Feed { port, at_ms, bytes: None }),
                _ => None,
            }
        })
        .collect()
}

/// Every file below the root with the extension, in path order, without build output and version control.
pub fn files_with(extension: &str) -> Vec<PathBuf> {
    fn walk(dir: &Path, extension: &str, found: &mut Vec<PathBuf>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir).expect("a readable directory").map(|entry| entry.expect("a directory entry").path()).collect();
        entries.sort();
        for path in entries {
            let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
            if path.is_dir() {
                if name != "target" && !name.starts_with('.') {
                    walk(&path, extension, found);
                }
            } else if path.extension().is_some_and(|found| found == extension) {
                found.push(path);
            }
        }
    }
    let mut found = Vec::new();
    walk(&root(), extension, &mut found);
    found
}

/// The 64-bit FNV-1a hash of the text as 16 hex digits, which is the same on every machine and release.
pub fn fnv(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// The path below the root, as written in messages.
pub fn relative(path: &Path) -> String {
    path.strip_prefix(root()).unwrap_or(path).display().to_string()
}
