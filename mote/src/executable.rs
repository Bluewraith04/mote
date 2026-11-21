//! Compiles the program of a pipeline to an object file and links it with the runtime into an executable.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use mote_codegen::ObjectLibrary;
use mote_ir::EntryKind;
use mote_packages::Location;

use crate::pipeline::{Failure, Pipeline};
use crate::run::{compile_failure, library_files};

/// The runtime an executable links: the static library `build.rs` made.
const RUNTIME: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/libmote_runtime.a"));

fn failure(message: impl std::fmt::Display) -> Failure {
    Failure { messages: vec![format!("error: {message}\n")] }
}

/// The relocatable object file of the program's start-up and `main`.
pub fn object(pipeline: &mut Pipeline) -> Result<Vec<u8>, Failure> {
    object_of(pipeline, EntryKind::Main)
}

/// The relocatable object file of the program's start-up and one entry.
pub fn object_of(pipeline: &mut Pipeline, kind: EntryKind) -> Result<Vec<u8>, Failure> {
    let error_text = pipeline.error_text.as_ref().and_then(|instance| pipeline.module.function(instance));
    let archives = archives(pipeline);
    let libraries = library_files(pipeline).into_iter().zip(archives).map(|(file, archive)| ObjectLibrary { file, archive: archive.is_some() }).collect();
    let root = mote_packages::shown(&pipeline.packages.root().dir.join(""));
    let map = &pipeline.map;
    let resolve = |position: u32| {
        let at = map.lookup(position)?;
        let name = &map.file(at.file).name;
        Some((name.strip_prefix(&root).unwrap_or(name).to_string(), at.line, at.column))
    };
    mote_codegen::compile_object(&pipeline.module, &mut pipeline.program, &pipeline.interner, kind, error_text, libraries, &resolve).map_err(compile_failure)
}

/// The archive of each library of the program, in the order of its link table.
fn archives(pipeline: &Pipeline) -> Vec<Option<PathBuf>> {
    pipeline.natives.libraries.iter().map(|library| library.archive()).collect()
}

/// Whether a program of this name is in a directory of the path.
fn found(path: &std::ffi::OsStr, name: &str) -> bool {
    std::env::split_paths(path).any(|dir| dir.join(name).is_file())
}

/// The C compiler that drives the link: `$CC`, else `cc`.
fn compiler_of(cc: Option<String>) -> String {
    cc.filter(|name| !name.is_empty()).unwrap_or_else(|| "cc".to_string())
}

fn compiler() -> String {
    compiler_of(std::env::var("CC").ok())
}

/// The linker to ask the compiler for: `mold` or `lld` when one is on the path, else the system's.
fn linker_of(path: &std::ffi::OsStr) -> Option<&'static str> {
    ["mold", "lld"].into_iter().find(|name| found(path, &format!("ld.{name}")) || found(path, name))
}

fn linker() -> Option<&'static str> {
    std::env::var_os("PATH").and_then(|path| linker_of(&path))
}

/// Writes the object to a directory of its own, links it with the runtime and the system libraries, and leaves the executable at `out`.
pub fn link(pipeline: &mut Pipeline, out: &Path) -> Result<(), Failure> {
    link_entry(pipeline, EntryKind::Main, out)?;
    copy_natives(pipeline, out)
}

/// Copies each shared library the executable opens, and no archive is linked for, to `native/` beside it, where it looks first.
fn copy_natives(pipeline: &Pipeline, out: &Path) -> Result<(), Failure> {
    let dir = out.parent().unwrap_or(Path::new("")).join("native");
    for library in &pipeline.natives.libraries {
        let Location::File(file) = &library.location else { continue };
        if library.archive().is_some() {
            continue;
        }
        let name = file.file_name().ok_or_else(|| failure(format!("{} has no name", file.display())))?;
        std::fs::create_dir_all(&dir).map_err(|error| failure(format!("cannot make {}: {error}", mote_packages::shown(&dir))))?;
        let to = dir.join(name);
        let temp = dir.join(format!(".{}.tmp", name.to_string_lossy()));
        std::fs::copy(file, &temp).and_then(|_| std::fs::rename(&temp, &to)).map_err(|error| failure(format!("cannot write {}: {error}", mote_packages::shown(&to))))?;
    }
    Ok(())
}

/// Like `link`, for one entry such as a `test` block.
pub fn link_entry(pipeline: &mut Pipeline, kind: EntryKind, out: &Path) -> Result<(), Failure> {
    let object = object_of(pipeline, kind)?;
    let archives: Vec<PathBuf> = archives(pipeline).into_iter().flatten().collect();
    static LINKS: AtomicUsize = AtomicUsize::new(0);
    let work: PathBuf = std::env::temp_dir().join(format!("mote-link-{}-{}", std::process::id(), LINKS.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&work).map_err(|error| failure(format!("cannot make a directory to link in: {error}")))?;
    let linked = (|| {
        let (object_file, runtime) = (work.join("program.o"), work.join("libmote_runtime.a"));
        std::fs::write(&object_file, &object).map_err(|error| failure(format!("cannot write the object file: {error}")))?;
        std::fs::write(&runtime, RUNTIME).map_err(|error| failure(format!("cannot write the runtime library: {error}")))?;
        let mut command = Command::new(compiler());
        command.arg("-o").arg(out).arg(&object_file).args(&archives).arg(&runtime);
        if let Some(linker) = linker() {
            command.arg(format!("-fuse-ld={linker}"));
        }
        command.args(["-lpthread", "-ldl", "-lm"]);
        let output = command.output().map_err(|error| failure(format!("cannot run `{}`: {error}", compiler())))?;
        if output.status.success() { Ok(()) } else { Err(failure(format!("the link failed:\n{}", String::from_utf8_lossy(&output.stderr)))) }
    })();
    let _ = std::fs::remove_dir_all(&work);
    linked
}

#[cfg(test)]
mod tests;
