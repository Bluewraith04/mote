//! Embeds the manifest and the `.mote` files of `../std`, and of each package of `../core`, as constants; a release build also builds the natives of `../core` and embeds them compressed.

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use flate2::write::GzEncoder;
use flate2::Compression;

fn collect(dir: &Path, rel: &str, out: &mut Vec<(String, PathBuf)>) {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir).expect("a package source directory").map(|entry| entry.expect("a directory entry").path()).collect();
    paths.sort();
    for path in paths {
        let name = path.file_name().and_then(|name| name.to_str()).expect("a UTF-8 file name").to_string();
        if path.is_dir() {
            collect(&path, &format!("{rel}{name}/"), out);
        } else if name.ends_with(".mote") {
            out.push((format!("{rel}{name}"), path));
        }
    }
}

/// The source of a package as the text of a list of `(path, text)` pairs.
fn files_of(package: &Path) -> String {
    println!("cargo:rerun-if-changed={}", package.join("mote.toml").display());
    println!("cargo:rerun-if-changed={}", package.join("src").display());
    let mut files = Vec::new();
    if package.join("src").is_dir() {
        collect(&package.join("src"), "", &mut files);
    }
    let mut code = String::from("&[\n");
    for (rel, path) in files {
        code.push_str(&format!("    ({rel:?}, include_str!({:?})),\n", path.display().to_string()));
    }
    code.push(']');
    code
}

/// FNV-1a of the bytes, which names the version of an embedded file.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3))
}

/// Whether this build carries the natives: a release build does, `MOTE_EMBED_NATIVES` (`0` or `1`) decides otherwise.
fn embeds_natives() -> bool {
    println!("cargo:rerun-if-env-changed=MOTE_EMBED_NATIVES");
    match env::var("MOTE_EMBED_NATIVES") {
        Ok(value) => value != "0",
        Err(_) => env::var("PROFILE").is_ok_and(|profile| profile == "release"),
    }
}

/// Builds the natives into `out` and gives the `NATIVES` constant: `(library, file, version, gzip bytes)` for each file.
fn natives_of(root: &Path, out: &Path) -> String {
    let mut code = String::from("/// A native file: its library, its file name, its version and its gzip bytes.\npub type EmbeddedNative = (&'static str, &'static str, u64, &'static [u8]);\npub const NATIVES: &[EmbeddedNative] = &[\n");
    if embeds_natives() {
        let built = out.join("natives");
        let status = Command::new("bash").arg(root.join("tools/build-core-natives.sh")).arg("--core").arg(root.join("core")).arg("--out").arg(&built).env_remove("CARGO_ENCODED_RUSTFLAGS").env_remove("CARGO_MANIFEST_DIR").status().expect("bash runs");
        assert!(status.success(), "tools/build-core-natives.sh failed; build with MOTE_EMBED_NATIVES=0 to leave the natives out");
        let mut libraries: Vec<PathBuf> = fs::read_dir(&built).expect("the built natives").map(|entry| entry.expect("a directory entry").path()).filter(|path| path.is_dir()).collect();
        libraries.sort();
        for library in libraries {
            let name = library.file_name().and_then(|name| name.to_str()).expect("a UTF-8 library name").to_string();
            let mut files: Vec<PathBuf> = fs::read_dir(&library).expect("a library directory").map(|entry| entry.expect("a directory entry").path()).filter(|path| path.extension().is_some_and(|extension| extension == "so" || extension == "a")).collect();
            files.sort();
            for file in files {
                let file_name = file.file_name().and_then(|name| name.to_str()).expect("a UTF-8 file name").to_string();
                let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
                encoder.write_all(&fs::read(&file).expect("a built native")).expect("compression");
                let packed = encoder.finish().expect("compression");
                let target = library.join(format!("{file_name}.gz"));
                fs::write(&target, &packed).expect("a compressed native");
                code.push_str(&format!("    ({name:?}, {file_name:?}, {}, include_bytes!({:?})),\n", fnv(&packed), target.display().to_string()));
            }
        }
    }
    code.push_str("];\n");
    code
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("a manifest directory"));
    let std = manifest.join("../std").canonicalize().expect("the std directory");
    let mut code = format!("pub const STD_MANIFEST: &str = include_str!({:?});\n", std.join("mote.toml").display().to_string());
    code.push_str(&format!("pub const STD_FILES: &[(&str, &str)] = {};\n", files_of(&std)));

    let core = manifest.join("../core").canonicalize().expect("the core directory");
    println!("cargo:rerun-if-changed={}", core.display());
    let mut packages: Vec<PathBuf> = fs::read_dir(&core).expect("the core directory").map(|entry| entry.expect("a directory entry").path()).filter(|path| path.join("mote.toml").is_file()).collect();
    packages.sort();
    code.push_str("/// A package: its name, its manifest and its `(path, text)` source files.\npub type Embedded = (&'static str, &'static str, &'static [(&'static str, &'static str)]);\npub const CORE: &[Embedded] = &[\n");
    for package in packages {
        let name = package.file_name().and_then(|name| name.to_str()).expect("a UTF-8 folder name");
        code.push_str(&format!("    ({name:?}, include_str!({:?}), {}),\n", package.join("mote.toml").display().to_string(), files_of(&package)));
    }
    code.push_str("];\n");
    let out = PathBuf::from(env::var("OUT_DIR").expect("an output directory"));
    code.push_str(&natives_of(&manifest.join("..").canonicalize().expect("the workspace directory"), &out));
    fs::write(out.join("std_files.rs"), code).expect("the generated file");
}
