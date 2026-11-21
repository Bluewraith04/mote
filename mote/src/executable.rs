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
#[cfg(all(target_os = "windows", target_env = "msvc"))]
const RUNTIME: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/mote_runtime.lib"));
#[cfg(not(all(target_os = "windows", target_env = "msvc")))]
const RUNTIME: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/libmote_runtime.a"));

#[cfg(all(target_os = "windows", target_env = "msvc"))]
const RUNTIME_FILE: &str = "mote_runtime.lib";
#[cfg(not(all(target_os = "windows", target_env = "msvc")))]
const RUNTIME_FILE: &str = "libmote_runtime.a";

#[cfg(all(target_os = "windows", target_env = "msvc"))]
const OBJECT_FILE: &str = "program.obj";
#[cfg(not(all(target_os = "windows", target_env = "msvc")))]
const OBJECT_FILE: &str = "program.o";

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
    pipeline.libraries.libraries.iter().map(|library| library.archive()).collect()
}

/// Whether a program of this name is in a directory of the path.
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn found(path: &std::ffi::OsStr, name: &str) -> bool {
    std::env::split_paths(path).any(|dir| dir.join(name).is_file())
}

/// The C compiler that drives the link: `$CC`, else `cc`.
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn compiler_of(cc: Option<String>) -> String {
    cc.filter(|name| !name.is_empty()).unwrap_or_else(|| "cc".to_string())
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
fn compiler() -> String {
    compiler_of(std::env::var("CC").ok())
}

/// The linker to ask the compiler for: `mold` or `lld` when one is on the path, else the system's.
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn linker_of(path: &std::ffi::OsStr) -> Option<&'static str> {
    ["mold", "lld"].into_iter().find(|name| found(path, &format!("ld.{name}")) || found(path, name))
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
fn linker() -> Option<&'static str> {
    std::env::var_os("PATH").and_then(|path| linker_of(&path))
}

/// Writes the object to a directory of its own, links it with the runtime and the system libraries, and leaves the executable at `out`.
pub fn link(pipeline: &mut Pipeline, out: &Path) -> Result<(), Failure> {
    link_entry(pipeline, EntryKind::Main, out)?;
    copy_libraries(pipeline, out)
}

/// Copies each shared library the executable opens, and no archive is linked for, to `lib/` beside it, where it looks first.
fn copy_libraries(pipeline: &Pipeline, out: &Path) -> Result<(), Failure> {
    let dir = out.parent().unwrap_or(Path::new("")).join("lib");
    for library in &pipeline.libraries.libraries {
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

#[cfg(target_os = "windows")]
fn newest_dir(parent: &Path) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(parent).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    dirs.pop()
}

#[cfg(target_os = "windows")]
fn find_msvc_toolchain() -> Option<(PathBuf, Vec<PathBuf>)> {
    if let Some(vc_tools) = std::env::var_os("VCToolsInstallDir").filter(|v| !v.is_empty()) {
        let linker = PathBuf::from(&vc_tools).join(r"bin\Hostx64\x64\link.exe");
        if linker.is_file() {
            return Some((linker, Vec::new()));
        }
    }
    let pf86 = std::env::var_os("ProgramFiles(x86)").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"));
    let vswhere = pf86.join(r"Microsoft Visual Studio\Installer\vswhere.exe");
    let vs_root = if vswhere.is_file() {
        let out = Command::new(&vswhere)
            .args(["-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"])
            .output()
            .ok()?;
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if path.is_empty() { None } else { Some(PathBuf::from(path)) }
    } else {
        None
    }?;
    let vc_ver = newest_dir(&vs_root.join(r"VC\Tools\MSVC"))?;
    let linker = vc_ver.join(r"bin\Hostx64\x64\link.exe");
    if !linker.is_file() {
        return None;
    }
    let mut lib_dirs = vec![vc_ver.join(r"lib\x64")];
    if let Some(sdk_ver) = newest_dir(&pf86.join(r"Windows Kits\10\Lib")) {
        lib_dirs.push(sdk_ver.join(r"um\x64"));
        lib_dirs.push(sdk_ver.join(r"ucrt\x64"));
    }
    Some((linker, lib_dirs))
}

#[cfg(target_os = "windows")]
fn archive_defines_symbol(path: &Path, symbol: &str) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else { return false };
    use std::io::Read;
    let mut header = [0u8; 68];
    if file.read_exact(&mut header).is_err() || &header[..8] != b"!<arch>\n" || &header[8..10] != b"/ " {
        return false;
    }
    let Ok(size_str) = std::str::from_utf8(&header[56..66]) else { return false };
    let Ok(size) = size_str.trim().parse::<usize>() else { return false };
    if !(4..=64 * 1024 * 1024).contains(&size) {
        return false;
    }
    let mut member = vec![0u8; size];
    if file.read_exact(&mut member).is_err() {
        return false;
    }
    let count = u32::from_be_bytes([member[0], member[1], member[2], member[3]]) as usize;
    let table_start = 4 + count.saturating_mul(4);
    if table_start >= member.len() {
        return false;
    }
    member[table_start..].split(|&byte| byte == 0).any(|entry| entry == symbol.as_bytes())
}

#[cfg(target_os = "windows")]
fn missing_inits(pipeline: &Pipeline) -> Vec<String> {
    pipeline
        .libraries
        .libraries
        .iter()
        .filter_map(|library| {
            let archive = library.archive()?;
            let init = mote_codegen::init_symbol(&library.name);
            (!archive_defines_symbol(&archive, &init)).then_some(init)
        })
        .collect()
}

#[cfg(target_os = "windows")]
fn invoke_linker(out: &Path, object_file: &Path, archives: &[PathBuf], fallback_inits: &[String], runtime: &Path) -> Result<(), Failure> {
    const WIN_LIBS: &[&str] = &[
        "kernel32.lib",
        "user32.lib",
        "gdi32.lib",
        "winmm.lib",
        "opengl32.lib",
        "shell32.lib",
        "advapi32.lib",
        "ole32.lib",
        "comdlg32.lib",
        "ws2_32.lib",
        "bcrypt.lib",
        "crypt32.lib",
        "ncrypt.lib",
        "secur32.lib",
        "userenv.lib",
        "ntdll.lib",
        "synchronization.lib",
        "iphlpapi.lib",
        "legacy_stdio_definitions.lib",
        "msvcrt.lib",
        "vcruntime.lib",
        "ucrt.lib",
    ];
    let (linker_exe, lib_dirs) = find_msvc_toolchain().ok_or_else(|| failure("cannot find MSVC link.exe (install Visual Studio Build Tools or run from a VS Developer Prompt)"))?;
    let mut command = Command::new(&linker_exe);
    command.arg("/NOLOGO").arg("/SUBSYSTEM:CONSOLE").arg("/BREPRO").arg(format!("/OUT:{}", out.display()));
    for dir in lib_dirs {
        command.arg(format!("/LIBPATH:{}", dir.display()));
    }
    for init in fallback_inits {
        command.arg(format!("/ALTERNATENAME:{init}=mote_init_noop"));
    }
    command.arg(object_file).args(archives).arg(runtime).args(WIN_LIBS);
    let output = command.output().map_err(|error| failure(format!("cannot run `{}`: {error}", linker_exe.display())))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        Err(failure(format!("the link failed:\n{stdout}{stderr}")))
    }
}

#[cfg(not(target_os = "windows"))]
fn invoke_linker(out: &Path, object_file: &Path, archives: &[PathBuf], _fallback_inits: &[String], runtime: &Path) -> Result<(), Failure> {
    let mut command = Command::new(compiler());
    command.arg("-o").arg(out).arg(object_file).args(archives).arg(runtime);
    if let Some(linker) = linker() {
        command.arg(format!("-fuse-ld={linker}"));
    }
    command.args(["-lpthread", "-ldl", "-lm"]);
    let output = command.output().map_err(|error| failure(format!("cannot run `{}`: {error}", compiler())))?;
    if output.status.success() { Ok(()) } else { Err(failure(format!("the link failed:\n{}", String::from_utf8_lossy(&output.stderr)))) }
}

/// Like `link`, for one entry such as a `test` block.
pub fn link_entry(pipeline: &mut Pipeline, kind: EntryKind, out: &Path) -> Result<(), Failure> {
    let object = object_of(pipeline, kind)?;
    let archives: Vec<PathBuf> = archives(pipeline).into_iter().flatten().collect();
    #[cfg(target_os = "windows")]
    let fallback_inits = missing_inits(pipeline);
    #[cfg(not(target_os = "windows"))]
    let fallback_inits: Vec<String> = Vec::new();
    static LINKS: AtomicUsize = AtomicUsize::new(0);
    let work: PathBuf = std::env::temp_dir().join(format!("mote-link-{}-{}", std::process::id(), LINKS.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&work).map_err(|error| failure(format!("cannot make a directory to link in: {error}")))?;
    let linked = (|| {
        let (object_file, runtime) = (work.join(OBJECT_FILE), work.join(RUNTIME_FILE));
        std::fs::write(&object_file, &object).map_err(|error| failure(format!("cannot write the object file: {error}")))?;
        std::fs::write(&runtime, RUNTIME).map_err(|error| failure(format!("cannot write the runtime library: {error}")))?;
        invoke_linker(out, &object_file, &archives, &fallback_inits, &runtime)
    })();
    let _ = std::fs::remove_dir_all(&work);
    linked
}

#[cfg(test)]
mod tests;
