//! Fingerprints the compiler front end, so a parse-cache entry from another compiler is never reused.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn main() {
    let src = Path::new("../compiler/src");
    println!("cargo:rerun-if-changed=../compiler/src");
    println!("cargo:rerun-if-changed=../compiler/Cargo.toml");
    let mut files = Vec::new();
    collect(src, &mut files);
    files.sort();
    let mut hasher = Sha256::new();
    hasher.update(env!("CARGO_PKG_VERSION"));
    for file in files.iter().chain([Path::new("../compiler/Cargo.toml").to_path_buf()].iter()) {
        hasher.update(file.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(std::fs::read(file).unwrap());
    }
    let hex: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    println!("cargo:rustc-env=MOTE_FRONTEND_FINGERPRINT={hex}");
}
