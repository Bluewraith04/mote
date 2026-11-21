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
    mote_packages::shown(path.strip_prefix(root()).unwrap_or(path))
}

/// Builds the shared `demo` library (and static archive when `archive` is true) into `package_dir/lib/<triple>/`.
pub fn build_demo_library(package_dir: &Path, archive: bool) {
    let libs = package_dir.join("lib").join(mote_packages::host_triple());
    std::fs::create_dir_all(&libs).expect("a directory for the library");
    let demo = root().join("integration/libs/demo.c");
    #[cfg(not(target_os = "windows"))]
    {
        let cc = |args: &[&std::ffi::OsStr]| {
            let status = std::process::Command::new("cc").args(args).status().expect("cc runs");
            assert!(status.success(), "cc {args:?}");
        };
        cc(&["-shared".as_ref(), "-fPIC".as_ref(), "-O1".as_ref(), "-o".as_ref(), libs.join("libdemo.so").as_os_str(), demo.as_os_str(), "-lpthread".as_ref()]);
        if archive {
            let object = libs.join("demo.o");
            cc(&["-c".as_ref(), "-fPIC".as_ref(), "-O1".as_ref(), "-Dmote_init=mote_init_demo".as_ref(), "-o".as_ref(), object.as_os_str(), demo.as_os_str()]);
            let made = std::process::Command::new("ar").arg("rcs").arg(libs.join("libdemo.a")).arg(&object).status().expect("ar runs");
            assert!(made.success(), "ar builds the archive");
        }
    }
    #[cfg(target_os = "windows")]
    {
        fn newest_dir(parent: &Path) -> Option<PathBuf> {
            let mut dirs: Vec<PathBuf> = std::fs::read_dir(parent).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect();
            dirs.sort();
            dirs.pop()
        }
        fn find_msvc_tools() -> (PathBuf, PathBuf, Option<std::ffi::OsString>, Option<std::ffi::OsString>, PathBuf) {
            if let Some(vc_tools) = std::env::var_os("VCToolsInstallDir").filter(|v| !v.is_empty()) {
                let bin = PathBuf::from(&vc_tools).join(r"bin\Hostx64\x64");
                return (bin.join("cl.exe"), bin.join("lib.exe"), std::env::var_os("INCLUDE"), std::env::var_os("LIB"), bin);
            }
            let pf86 = std::env::var_os("ProgramFiles(x86)").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"));
            let vswhere = pf86.join(r"Microsoft Visual Studio\Installer\vswhere.exe");
            let out = std::process::Command::new(&vswhere)
                .args(["-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"])
                .output()
                .expect("vswhere runs");
            let vs_root = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            let vc_ver = newest_dir(&vs_root.join(r"VC\Tools\MSVC")).expect("MSVC version dir");
            let bin = vc_ver.join(r"bin\Hostx64\x64");
            let mut inc = vec![vc_ver.join("include")];
            let mut lib = vec![vc_ver.join(r"lib\x64")];
            if let Some(sdk_inc) = newest_dir(&pf86.join(r"Windows Kits\10\Include")) {
                inc.extend([sdk_inc.join("ucrt"), sdk_inc.join("um"), sdk_inc.join("shared")]);
            }
            if let Some(sdk_lib) = newest_dir(&pf86.join(r"Windows Kits\10\Lib")) {
                lib.extend([sdk_lib.join(r"um\x64"), sdk_lib.join(r"ucrt\x64")]);
            }
            let join = |dirs: Vec<PathBuf>| std::env::join_paths(dirs).expect("valid paths");
            (bin.join("cl.exe"), bin.join("lib.exe"), Some(join(inc)), Some(join(lib)), bin)
        }
        let (cl, lib_exe, inc, lib, bin) = find_msvc_tools();
        let mut path_dirs = vec![bin];
        if let Some(existing) = std::env::var_os("PATH") {
            path_dirs.extend(std::env::split_paths(&existing));
        }
        let path_env = std::env::join_paths(path_dirs).expect("valid PATH");
        let run_tool = |exe: &Path, args: &[&std::ffi::OsStr]| {
            let mut cmd = std::process::Command::new(exe);
            cmd.args(args).env("PATH", &path_env);
            if let Some(ref v) = inc {
                cmd.env("INCLUDE", v);
            }
            if let Some(ref v) = lib {
                cmd.env("LIB", v);
            }
            let out = cmd.output().unwrap_or_else(|e| panic!("{} failed to start: {e}", exe.display()));
            assert!(out.status.success(), "{} failed:\n{}\n{}", exe.display(), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        };
        let dll_out = libs.join("demo.dll");
        let dll_obj = libs.join("demo_dll.obj");
        let imp_lib = libs.join("demo_imp.lib");
        let exp_file = libs.join("demo_imp.exp");
        run_tool(&cl, &[
            "/nologo".as_ref(),
            "/O1".as_ref(),
            "/MD".as_ref(),
            "/LD".as_ref(),
            "/DDEMO_DLL".as_ref(),
            format!("/Fo{}", dll_obj.display()).as_ref(),
            format!("/Fe{}", dll_out.display()).as_ref(),
            demo.as_os_str(),
            "/link".as_ref(),
            format!("/IMPLIB:{}", imp_lib.display()).as_ref(),
        ]);
        let _ = std::fs::remove_file(&dll_obj);
        let _ = std::fs::remove_file(&imp_lib);
        let _ = std::fs::remove_file(&exp_file);
        if archive {
            let obj = libs.join("demo.obj");
            run_tool(&cl, &[
                "/nologo".as_ref(),
                "/O1".as_ref(),
                "/MD".as_ref(),
                "/c".as_ref(),
                "/Dmote_init=mote_init_demo".as_ref(),
                format!("/Fo{}", obj.display()).as_ref(),
                demo.as_os_str(),
            ]);
            run_tool(&lib_exe, &[
                "/nologo".as_ref(),
                format!("/OUT:{}", libs.join("demo.lib").display()).as_ref(),
                obj.as_os_str(),
            ]);
            let _ = std::fs::remove_file(&obj);
        }
    }
}
