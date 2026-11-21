//! A C program links the static library and calls the entries.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

use mote_abi::{Entry, FaultKind, FaultTable, Kind, Scalar, Span, TypeImage, TypeTable};

/// The directory of the profile this test was built in, such as `target/debug`.
fn profile_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("the test executable");
    exe.parent().and_then(Path::parent).expect("deps is inside the profile directory").to_path_buf()
}

/// Builds the static library with cargo, once, and gives its path.
fn library() -> &'static Path {
    static LIBRARY: OnceLock<PathBuf> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        let dir = profile_dir();
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../Cargo.toml");
        let mut build = Command::new(env!("CARGO"));
        build.args(["build", "--locked", "--offline", "-p", "mote-runtime", "--manifest-path"]).arg(&manifest);
        if dir.file_name().is_some_and(|name| name == "release") {
            build.arg("--release");
        }
        let status = build.status().expect("cargo runs");
        assert!(status.success(), "cargo build failed");
        let library_name = if cfg!(target_os = "windows") { "mote_runtime.lib" } else { "libmote_runtime.a" };
        let library = dir.join(library_name);
        assert!(library.exists(), "{} was not built", library.display());
        library
    })
}

#[cfg(target_os = "windows")]
fn find_msvc_tools() -> (PathBuf, Vec<PathBuf>, Vec<PathBuf>) {
    fn newest(parent: &Path) -> Option<PathBuf> {
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(parent).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect();
        dirs.sort();
        dirs.pop()
    }
    let pf86 = std::env::var_os("ProgramFiles(x86)").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"));
    let vswhere = pf86.join(r"Microsoft Visual Studio\Installer\vswhere.exe");
    let out = Command::new(&vswhere)
        .args(["-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"])
        .output()
        .expect("vswhere runs");
    let vs_root = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let vc_ver = newest(&vs_root.join(r"VC\Tools\MSVC")).expect("MSVC toolchain dir");
    let bin_dir = vc_ver.join(r"bin\Hostx64\x64");
    let mut includes = vec![vc_ver.join("include")];
    let mut libs = vec![vc_ver.join(r"lib\x64")];
    if let Some(sdk_inc) = newest(&pf86.join(r"Windows Kits\10\Include")) {
        for sub in ["ucrt", "um", "shared"] {
            includes.push(sdk_inc.join(sub));
        }
    }
    if let Some(sdk_lib) = newest(&pf86.join(r"Windows Kits\10\Lib")) {
        for sub in ["um\\x64", "ucrt\\x64"] {
            libs.push(sdk_lib.join(sub));
        }
    }
    (bin_dir, includes, libs)
}

/// The names of the types the C program allocates.
fn tables() -> String {
    let mut table = TypeTable::new();
    let int = table.add("Int", Kind::Scalar(Scalar::I64));
    let point = table.add("Point", Kind::Class(vec![int, int]));
    let text = table.add("String", Kind::String);
    let mut faults = FaultTable::new();
    faults.add(FaultKind::DivideByZero, Span::new(10, 15));
    faults.add(FaultKind::IndexOutOfRange, Span::new(20, 30));
    let array = |name: &str, bytes: &[u8]| {
        let mut text = format!("static const unsigned char {name}[] = {{");
        for byte in bytes {
            write!(text, "{byte},").unwrap();
        }
        text + "};\n"
    };
    let image = TypeImage::new(table).to_bytes().expect("the table is written");
    format!("#define POINT {}\n#define TEXT {}\n{}{}", point.raw(), text.raw(), array("types", &image), array("faults", &faults.to_bytes()))
}

/// Compiles the C program against the library and runs the scenario.
fn run_c(scenario: &str) -> Output {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("c_link").join(scenario);
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    std::fs::write(dir.join("tables.h"), tables()).expect("the tables are written");
    let main_c = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/c_link/main.c");
    #[cfg(target_os = "windows")]
    let program = {
        let (bin_dir, includes, libs) = find_msvc_tools();
        let exe = dir.join("program.exe");
        let obj = dir.join("main.obj");
        let mut cmd = Command::new(bin_dir.join("cl.exe"));
        cmd.args(["/nologo", "/Od", "/W3", &format!("/Fo{}", obj.display()), &format!("/Fe{}", exe.display())]);
        cmd.arg("/I").arg(&dir);
        for inc in includes {
            cmd.arg("/I").arg(inc);
        }
        cmd.arg(&main_c).arg(library()).arg("/link");
        for lib in libs {
            cmd.arg(format!("/LIBPATH:{}", lib.display()));
        }
        cmd.args(["ws2_32.lib", "userenv.lib", "bcrypt.lib", "ntdll.lib", "advapi32.lib", "kernel32.lib", "synchronization.lib"]);
        let compile = cmd.output().expect("cl.exe runs");
        assert!(compile.status.success(), "{}{}", String::from_utf8_lossy(&compile.stdout), String::from_utf8_lossy(&compile.stderr));
        exe
    };
    #[cfg(not(target_os = "windows"))]
    let program = {
        let exe = dir.join("program");
        let compile = Command::new("cc")
            .args(["-O0", "-g", "-fno-omit-frame-pointer", "-Wall", "-Werror", "-I"])
            .arg(&dir)
            .arg(&main_c)
            .arg(library())
            .args(["-lgcc_s", "-lutil", "-lrt", "-lpthread", "-lm", "-ldl", "-lc", "-o"])
            .arg(&exe)
            .output()
            .expect("cc runs");
        assert!(compile.status.success(), "{}", String::from_utf8_lossy(&compile.stderr));
        exe
    };
    Command::new(&program).arg(scenario).output().expect("the program runs")
}

fn lines(output: &Output) -> Vec<String> {
    String::from_utf8_lossy(&output.stdout).lines().map(str::to_string).collect()
}

#[test]
fn c_link_a_c_program_allocates_through_the_library() {
    let output = run_c("alloc");
    assert_eq!(output.status.code(), Some(0), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(lines(&output), ["point type 2 words 0 0", "text type 3 length 5", "region moved 1"]);
}

#[test]
fn c_link_a_fault_reaches_the_c_handler_with_the_frames_of_the_c_callers() {
    let output = run_c("fault");
    assert_eq!(output.status.code(), Some(42), "{}", String::from_utf8_lossy(&output.stderr));
    let expected_frames = if cfg!(target_os = "windows") { "frames 1" } else { "frames 1 1 1" };
    assert_eq!(lines(&output), ["kind 3", "site 1", "span 20 30", "message index out of range", "fault_site 2", expected_frames]);
}

#[test]
fn c_link_a_message_from_the_program_is_a_raised_fault_without_a_site() {
    let output = run_c("message");
    assert_eq!(output.status.code(), Some(42), "{}", String::from_utf8_lossy(&output.stderr));
    let seen = lines(&output);
    assert_eq!(&seen[..5], ["kind 6", "site -1", "span -1 -1", "message too deep", "fault_site 0"]);
}

#[test]
fn c_link_without_a_handler_the_report_goes_to_standard_error_and_the_process_exits_with_1() {
    let output = run_c("default");
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(error.starts_with("fault: division by zero\n  at 0x"), "{error}");
    assert!(output.stdout.is_empty());
}

#[test]
fn c_link_the_library_defines_a_symbol_for_every_entry_of_the_abi() {
    #[cfg(target_os = "windows")]
    {
        let (bin_dir, _, _) = find_msvc_tools();
        let listing = Command::new(bin_dir.join("dumpbin.exe")).arg("/SYMBOLS").arg(library()).output().expect("dumpbin runs");
        let listing = String::from_utf8_lossy(&listing.stdout).into_owned();
        for entry in Entry::ALL {
            let suffix = format!("| {}", entry.name());
            assert!(listing.lines().any(|line| !line.contains("UNDEF") && line.contains("External") && line.trim_end().ends_with(&suffix)), "{} is not defined", entry.name());
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let listing = Command::new("nm").arg("-g").arg(library()).output().expect("nm runs");
        let listing = String::from_utf8_lossy(&listing.stdout).into_owned();
        for entry in Entry::ALL {
            assert!(listing.lines().any(|line| line.ends_with(&format!(" T {}", entry.name()))), "{} is not defined", entry.name());
        }
    }
}
