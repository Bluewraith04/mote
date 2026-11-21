//! A package as a zip file: packing a directory and unpacking a zip into one.

use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use crate::manifest::{Manifest, PackageError};
use crate::package::shown;

/// The most files a zip may hold.
pub const MAX_ENTRIES: usize = 20_000;

/// The most bytes a zip may unpack to.
pub const MAX_BYTES: u64 = 512 << 20;

/// A packed package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packed {
    /// `<name>-<version>.zip`.
    pub file: String,
    pub bytes: Vec<u8>,
    /// The number of files in it.
    pub files: usize,
}

fn fail(file: &str, message: impl Into<String>) -> PackageError {
    PackageError { file: file.to_string(), line: 0, key: String::new(), message: message.into() }
}

/// The files below `dir` as (path with `/`, file), in path order; the top-level `target` and `deps` and every name that starts with `.` are left out.
fn files_of(dir: &Path, rel: &str, found: &mut Vec<(String, PathBuf)>, depth: usize) -> Result<(), PackageError> {
    if depth > 64 {
        return Err(fail(&shown(dir), "nested deeper than 64 directories"));
    }
    let mut paths: Vec<PathBuf> = fs::read_dir(dir).map_err(|error| fail(&shown(dir), format!("cannot read: {error}")))?.map(|entry| entry.map(|entry| entry.path()).map_err(|error| fail(&shown(dir), format!("cannot read: {error}")))).collect::<Result<_, _>>()?;
    paths.sort();
    for path in paths {
        let name = path.file_name().and_then(|name| name.to_str()).ok_or_else(|| fail(&shown(&path), "not a UTF-8 name"))?;
        if name.starts_with('.') || (rel.is_empty() && (name == "target" || name == "deps")) {
            continue;
        }
        let kind = fs::symlink_metadata(&path).map_err(|error| fail(&shown(&path), format!("cannot read: {error}")))?.file_type();
        if kind.is_symlink() {
            return Err(fail(&shown(&path), "a symbolic link cannot be packed"));
        }
        if kind.is_dir() {
            files_of(&path, &format!("{rel}{name}/"), found, depth + 1)?;
        } else {
            found.push((format!("{rel}{name}"), path));
        }
    }
    Ok(())
}

/// Packs the package in `dir`: its files in path order with a fixed time, so the same files make the same bytes.
pub fn pack(dir: &Path) -> Result<Packed, PackageError> {
    let manifest = Manifest::read(&dir.join("mote.toml"))?;
    let mut files = Vec::new();
    files_of(dir, "", &mut files, 0)?;
    if files.len() > MAX_ENTRIES {
        return Err(fail(&shown(dir), format!("more than {MAX_ENTRIES} files")));
    }
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).last_modified_time(DateTime::default()).unix_permissions(0o644);
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let mut total = 0u64;
    for (rel, path) in &files {
        let bytes = fs::read(path).map_err(|error| fail(&shown(path), format!("cannot read: {error}")))?;
        total += bytes.len() as u64;
        if total > MAX_BYTES {
            return Err(fail(&shown(dir), format!("more than {} MiB of files", MAX_BYTES >> 20)));
        }
        writer.start_file(rel.as_str(), options).and_then(|()| writer.write_all(&bytes).map_err(Into::into)).map_err(|error| fail(&shown(path), format!("cannot pack: {error}")))?;
    }
    let bytes = writer.finish().map_err(|error| fail(&shown(dir), format!("cannot pack: {error}")))?.into_inner();
    Ok(Packed { file: format!("{}-{}.zip", manifest.name, manifest.version), bytes, files: files.len() })
}

/// The parts of a path inside a zip, or why it is not one.
fn parts_of(name: &str) -> Result<Vec<&str>, String> {
    let outside = || format!("`{name}` is outside the package");
    if name.is_empty() || name.contains('\\') || name.contains('\0') {
        return Err(outside());
    }
    let mut parts = Vec::new();
    for component in Path::new(name.trim_end_matches('/')).components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str().ok_or_else(outside)?),
            _ => return Err(outside()),
        }
    }
    Ok(parts)
}

/// Unpacks the zip into `dest`, which must not exist; a zip with a single top folder, such as an archive GitHub makes, is unpacked without it. The zip must hold a `mote.toml`.
pub fn unpack(bytes: &[u8], dest: &Path) -> Result<(), String> {
    unpack_within(bytes, dest, MAX_ENTRIES, MAX_BYTES)
}

fn unpack_within(bytes: &[u8], dest: &Path, max_entries: usize, max_bytes: u64) -> Result<(), String> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(|error| format!("not a zip file: {error}"))?;
    if zip.len() > max_entries {
        return Err(format!("more than {max_entries} files"));
    }
    let mut entries = Vec::new();
    let mut declared = 0u64;
    for at in 0..zip.len() {
        let file = zip.by_index(at).map_err(|error| format!("not a zip file: {error}"))?;
        if file.is_symlink() {
            return Err(format!("`{}` is a symbolic link", file.name()));
        }
        let parts: Vec<String> = parts_of(file.name())?.into_iter().map(str::to_string).collect();
        declared = declared.saturating_add(file.size());
        entries.push((parts, file.is_dir()));
    }
    if declared > max_bytes {
        return Err(format!("more than {max_bytes} bytes unpacked"));
    }
    let strip = usize::from(!entries.is_empty() && entries.iter().all(|(parts, dir)| parts.len() > 1 || *dir) && entries.iter().all(|(parts, _)| parts.first() == entries[0].0.first()));
    fs::create_dir_all(dest).map_err(|error| format!("cannot make {}: {error}", shown(dest)))?;
    let mut left = max_bytes;
    for (at, (parts, dir)) in entries.iter().enumerate() {
        let Some(below) = parts.get(strip..).filter(|below| !below.is_empty()) else { continue };
        let path = below.iter().fold(dest.to_path_buf(), |path, part| path.join(part));
        if *dir {
            fs::create_dir_all(&path).map_err(|error| format!("cannot make {}: {error}", shown(&path)))?;
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("cannot make {}: {error}", shown(parent)))?;
        }
        let mut file = zip.by_index(at).map_err(|error| format!("not a zip file: {error}"))?;
        let mut out = fs::File::create_new(&path).map_err(|error| format!("cannot write {}: {error}", below.join("/")))?;
        let copied = std::io::copy(&mut (&mut file).take(left + 1), &mut out).map_err(|error| format!("cannot unpack `{}`: {error}", below.join("/")))?;
        if copied > left {
            return Err(format!("more than {max_bytes} bytes unpacked"));
        }
        left -= copied;
    }
    if !dest.join("mote.toml").is_file() {
        return Err("no mote.toml at the top of the zip".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Scratch;

    fn zip_of(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, text) in entries {
            writer.start_file(*name, SimpleFileOptions::default()).unwrap();
            writer.write_all(text.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    const MANIFEST: &str = "[package]\nname = \"pane\"\nversion = \"1.2.3\"\n";

    fn names_in(dir: &Path) -> Vec<String> {
        let mut found = Vec::new();
        let mut files = Vec::new();
        files_of(dir, "", &mut files, 0).unwrap();
        for (rel, _) in files {
            found.push(rel);
        }
        found
    }

    #[test]
    fn archive_packs_the_files_in_path_order_and_names_the_zip_after_the_package() {
        let scratch = Scratch::new();
        let dir = scratch.package("pane", "pane", &[], &[("lib.mote", "a"), ("nav/go.mote", "b")]);
        let lib_rel = format!("lib/{}/{}", crate::host_triple(), crate::library_file("pane"));
        scratch.write(&format!("pane/{lib_rel}"), "so");
        scratch.write("pane/README.md", "read me");
        let packed = pack(&dir).unwrap();
        assert_eq!((packed.file.as_str(), packed.files), ("pane-0.1.0.zip", 5));
        let out = scratch.path("out");
        unpack(&packed.bytes, &out).unwrap();
        assert_eq!(names_in(&out), ["README.md", lib_rel.as_str(), "mote.toml", "src/lib.mote", "src/nav/go.mote"]);
        assert_eq!(fs::read_to_string(out.join("src/nav/go.mote")).unwrap(), "b");
    }

    #[test]
    fn archive_leaves_out_target_deps_and_dot_names() {
        let scratch = Scratch::new();
        let dir = scratch.package("pane", "pane", &[], &[("lib.mote", "a")]);
        scratch.write("pane/target/pane", "exe");
        scratch.write("pane/deps/wire-1.0.0.zip", "zip");
        scratch.write("pane/.git/config", "git");
        scratch.write("pane/.gitignore", "target");
        scratch.write("pane/src/target/keep.mote", "inner");
        let out = scratch.path("out");
        unpack(&pack(&dir).unwrap().bytes, &out).unwrap();
        assert_eq!(names_in(&out), ["mote.toml", "src/lib.mote", "src/target/keep.mote"]);
    }

    #[test]
    fn archive_the_same_files_make_the_same_bytes() {
        let scratch = Scratch::new();
        let dir = scratch.package("pane", "pane", &[], &[("lib.mote", "a"), ("b.mote", "b")]);
        let first = pack(&dir).unwrap();
        fs::write(dir.join("src/b.mote"), "b").unwrap();
        assert_eq!(pack(&dir).unwrap(), first);
    }

    #[cfg(unix)]
    #[test]
    fn archive_a_symbolic_link_cannot_be_packed() {
        let scratch = Scratch::new();
        let dir = scratch.package("pane", "pane", &[], &[("lib.mote", "a")]);
        std::os::unix::fs::symlink("lib.mote", dir.join("src/alias.mote")).unwrap();
        let found = pack(&dir).unwrap_err();
        assert!(found.to_string().ends_with("alias.mote: a symbolic link cannot be packed"), "{found}");
    }

    #[test]
    fn archive_a_directory_without_a_manifest_cannot_be_packed() {
        let scratch = Scratch::new();
        scratch.write("none/src/lib.mote", "");
        assert!(pack(&scratch.path("none")).unwrap_err().to_string().contains("mote.toml: cannot read"));
    }

    #[test]
    fn archive_unpacks_a_zip_with_one_top_folder_without_the_folder() {
        let scratch = Scratch::new();
        let zip = zip_of(&[("pane-main/mote.toml", MANIFEST), ("pane-main/src/lib.mote", "x")]);
        unpack(&zip, &scratch.path("out")).unwrap();
        assert_eq!(names_in(&scratch.path("out")), ["mote.toml", "src/lib.mote"]);
    }

    #[test]
    fn archive_keeps_a_top_folder_that_is_part_of_the_package() {
        let scratch = Scratch::new();
        let zip = zip_of(&[("mote.toml", MANIFEST), ("src/lib.mote", "x")]);
        unpack(&zip, &scratch.path("out")).unwrap();
        assert_eq!(names_in(&scratch.path("out")), ["mote.toml", "src/lib.mote"]);
    }

    #[test]
    fn archive_a_path_that_leaves_the_package_is_an_error() {
        let scratch = Scratch::new();
        for name in ["../escape.mote", "/abs.mote", "src/../../x", "a\\b"] {
            let zip = zip_of(&[("mote.toml", MANIFEST), (name, "x")]);
            let found = unpack(&zip, &scratch.path("out")).unwrap_err();
            assert_eq!(found, format!("`{name}` is outside the package"), "{name}");
            assert!(!scratch.path("escape.mote").exists());
            let _ = fs::remove_dir_all(scratch.path("out"));
        }
    }

    #[test]
    fn archive_a_symbolic_link_in_a_zip_is_an_error() {
        let scratch = Scratch::new();
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer.start_file("mote.toml", SimpleFileOptions::default()).unwrap();
        writer.write_all(MANIFEST.as_bytes()).unwrap();
        writer.add_symlink("link", "/etc/passwd", SimpleFileOptions::default()).unwrap();
        let zip = writer.finish().unwrap().into_inner();
        assert_eq!(unpack(&zip, &scratch.path("out")).unwrap_err(), "`link` is a symbolic link");
    }

    #[test]
    fn archive_a_zip_without_a_manifest_or_that_is_not_a_zip_is_an_error() {
        let scratch = Scratch::new();
        assert_eq!(unpack(&zip_of(&[("src/lib.mote", "x")]), &scratch.path("a")).unwrap_err(), "no mote.toml at the top of the zip");
        assert!(unpack(b"not a zip", &scratch.path("b")).unwrap_err().starts_with("not a zip file"));
    }

    #[test]
    fn archive_a_zip_with_too_many_files_or_bytes_is_an_error() {
        let scratch = Scratch::new();
        let zip = zip_of(&[("mote.toml", MANIFEST), ("a", "1"), ("b", "2")]);
        assert_eq!(unpack_within(&zip, &scratch.path("many"), 2, 1000).unwrap_err(), "more than 2 files");
        assert_eq!(unpack_within(&zip, &scratch.path("big"), 10, 10).unwrap_err(), "more than 10 bytes unpacked");
        assert_eq!(unpack_within(&zip, &scratch.path("fits"), 3, 1000), Ok(()));
    }

    #[test]
    fn archive_the_limits_are_the_documented_ones() {
        assert_eq!((MAX_ENTRIES, MAX_BYTES), (20_000, 512 << 20));
    }
}
