//! The `mote` binary.
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

const USAGE: &str = "\
Usage: mote <command> [args]
       mote <file.mote | file.mbc | file.masm>

Commands:
  run [file]                 Run a file, or the current package
  run -- [args]              Run the current package with command-line arguments
  check [file] [--memory]    Type-check without running; --memory shows where struct literals live
  compile [file] [-o out]    Compile to .mbc
  build [file] [-o out]      Build a standalone executable
  test [--filter text]       Run the package's tests
       [--schedule-seed n]   Vary where tasks switch (one worker); a failure prints the seed
  init [name] [--lib]        Create a package
  add <git url>[@ref]        Add a dependency from a git repository (github:user/repo works too)
  remove <name>              Remove a dependency
  sync [--locked]            Resolve dependencies, update mote.lock and fill .mote_packages
  install [<dir> | <git url>[@ref]]
                             Build a program into ~/.mote/bin with its native libraries
  package                   Create a .mpk archive
  version                    Print the version
  help                       Print this message

Flags:
  --gc <nogc|mark-sweep>     Choose the collector
  --max-heap <size>          Heap limit, such as 2GiB or unlimited (default: half of memory)
  --mem-stats                Print what the run used to stderr
  --workers <n>              Scheduler worker count
  --allow-native             Let std.dev.libtools load native libraries
  --release                  Release build
";

static MEM_STATS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn parse_mem_stats_flag(args: Vec<String>) -> Vec<String> {
    MEM_STATS.store(args.iter().any(|a| a == "--mem-stats"), std::sync::atomic::Ordering::Relaxed);
    args.into_iter().filter(|a| a != "--mem-stats").collect()
}

static MAX_HEAP_FLAG: std::sync::OnceLock<String> = std::sync::OnceLock::new();

static MANIFEST_MAX_HEAP: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn parse_max_heap_flag(mut args: Vec<String>) -> Vec<String> {
    let Some(pos) = args.iter().position(|a| a == "--max-heap") else {
        return args;
    };
    let Some(value) = args.get(pos + 1).cloned() else {
        eprintln!("Error: --max-heap requires a size (such as 2GiB or unlimited)");
        process::exit(1);
    };
    args.drain(pos..=pos + 1);
    let _ = MAX_HEAP_FLAG.set(value);
    args
}

fn heap_limit() -> usize {
    let env = env::var(cli::limits::MAX_HEAP_ENV).ok();
    let available = platform::memory::available_bytes();
    match cli::limits::resolve_max_heap(MAX_HEAP_FLAG.get().map(String::as_str), env.as_deref(), MANIFEST_MAX_HEAP.get().map(String::as_str), available) {
        Ok(limit) => limit.unwrap_or(usize::MAX),
        Err(e) => {
            eprintln!("Error: {e}");
            process::exit(1);
        }
    }
}

fn execute_compiled(compiled: compiler::CompiledProgram, gc: gc::GCConfig, workers: usize) {
    let gc = gc.with_max_heap(heap_limit());
    let mut registry = isa::value::TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let global_count = compiled.global_count as usize;

    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    rt.set_sources(compiled.sources);
    rt.set_global_count(global_count);
    cli::builtins::install(&mut rt);
    if let Err(e) = rt.set_native_table(&compiled.native_table) {
        eprintln!("Native error: {}", e);
        process::exit(1);
    }
    rt.set_heap(gc::build_gc_engine(&gc));
    if let Err(e) = cli::limits::apply(&mut rt) {
        eprintln!("Error: {e}");
        process::exit(1);
    }
    if let Some(&seed) = SCHEDULE_SEED.get() {
        rt.set_schedule_seed(seed);
    }
    let outcome = rt.run_entry_on(workers).map(|_| rt.status());
    if MEM_STATS.load(std::sync::atomic::Ordering::Relaxed) {
        for line in cli::memstats::lines(&rt, platform::memory::peak_rss_bytes()) {
            eprintln!("{line}");
        }
    }
    match outcome {
        Ok(runtime::VmStatus::Exited(code)) => {
            if code != 0 {
                report_seed_on_failure();
            }
            process::exit(code)
        }
        Ok(_status) => {}
        Err(e) => {
            eprintln!("Runtime error: {}", e);
            report_seed_on_failure();
            process::exit(1);
        }
    }
}

fn parse_gc_flag(mut args: Vec<String>) -> (Vec<String>, gc::GCConfig) {
    let Some(pos) = args.iter().position(|a| a == "--gc") else {
        return (args, gc::GCConfig::default());
    };
    let Some(value) = args.get(pos + 1).cloned() else {
        eprintln!("Error: --gc requires a value (nogc or mark-sweep)");
        process::exit(1);
    };
    let gc = match value.as_str() {
        "nogc" => gc::GCConfig::nogc(),
        "mark-sweep" => gc::GCConfig::mark_sweep(),
        other => {
            eprintln!("Error: unknown --gc value '{}' (expected nogc or mark-sweep)", other);
            process::exit(1);
        }
    };
    args.drain(pos..=pos + 1);
    (args, gc)
}

fn parse_workers_flag(mut args: Vec<String>) -> (Vec<String>, usize) {
    let mut flag = None;
    if let Some(pos) = args.iter().position(|a| a == "--workers") {
        let Some(value) = args.get(pos + 1).cloned() else {
            eprintln!("Error: --workers requires a number");
            process::exit(1);
        };
        args.drain(pos..=pos + 1);
        flag = Some(value);
    }
    match cli::resolve_workers(flag.as_deref(), env::var(cli::WORKERS_ENV).ok().as_deref()) {
        Ok(n) => (args, n),
        Err(e) => {
            eprintln!("Error: {e}");
            process::exit(1);
        }
    }
}

fn env_allows_native() -> bool {
    env::var("MOTE_ALLOW_NATIVE").is_ok_and(|v| v == "1")
}

fn parse_allow_native_flag(args: Vec<String>) -> Vec<String> {
    ffi::builtins::set_allow_native(env_allows_native() || args.iter().any(|a| a == "--allow-native"));
    args.into_iter().filter(|a| a != "--allow-native").collect()
}

static SCHEDULE_SEED: std::sync::OnceLock<u64> = std::sync::OnceLock::new();

fn parse_seed_flag(mut args: Vec<String>, workers: usize) -> (Vec<String>, usize) {
    let Some(pos) = args.iter().position(|a| a == "--schedule-seed") else {
        return (args, workers);
    };
    let Some(seed) = args.get(pos + 1).and_then(|v| v.parse::<u64>().ok()) else {
        eprintln!("Error: --schedule-seed requires a number");
        process::exit(1);
    };
    args.drain(pos..=pos + 1);
    let _ = SCHEDULE_SEED.set(seed);
    (args, 1)
}

fn report_seed_on_failure() {
    if let Some(seed) = SCHEDULE_SEED.get() {
        eprintln!("schedule seed: {seed} (replay with --schedule-seed {seed})");
    }
}

fn parse_filter_flag(mut args: Vec<String>) -> (Vec<String>, Option<String>) {
    let Some(pos) = args.iter().position(|a| a == "--filter") else {
        return (args, None);
    };
    let Some(value) = args.get(pos + 1).cloned() else {
        eprintln!("Error: --filter requires a pattern");
        process::exit(1);
    };
    args.drain(pos..=pos + 1);
    (args, Some(value))
}

fn exe_name(stem: &str) -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(format!("{stem}.exe"))
    }
    #[cfg(not(windows))]
    {
        PathBuf::from(stem)
    }
}

fn ensure_parent(path: &Path) -> Result<(), String> {
    match path.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(dir) => fs::create_dir_all(dir).map_err(|e| format!("Failed to create '{}': {}", dir.display(), e)),
        None => Ok(()),
    }
}

fn discover_package_or_exit(verb: &str) -> (PathBuf, pkg::PackageManifest) {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match pkg::PackageManifest::discover(&cwd) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("Error: no file given to `mote {verb}` and {e}");
            eprintln!("       pass a file, or run `mote init` to create a package");
            process::exit(1);
        }
    }
}

/// `mote install`: builds the package `target` names into `<mote home>/bin`, with the native libraries of its dependencies beside it.
fn install_program(target: Option<&str>) -> Result<(), String> {
    let cwd = env::current_dir().map_err(|e| format!("no current directory: {e}"))?;
    let source = pkg::program::locate(target, &cwd)?;
    let name = source.manifest.package.name.clone();
    if !pkg::program::has_main(&source.root, &source.manifest) {
        return Err(format!("{name} is a library; there is no program to install"));
    }
    pkg::PackageManager::install_dependencies(&source.root, false)?;
    let (_mbc, compiled) = pkg::PackageManager::compile_program(&source.root)?;
    let bin = pkg::store::bin_dir()?;
    let out = bin.join(exe_name(&name));
    pkg::StandaloneBundler::bundle_program(&compiled, &out, &cli::builtins::tier_of)?;
    let libs = pkg::native::install_libraries(&source.root, &out)?;
    println!("Installed {name} {} to {}", source.manifest.package.version, out.display());
    if libs.copied + libs.kept > 0 {
        println!("{} native libraries copied, {} kept", libs.copied, libs.kept);
    }
    let on_path = env::var_os("PATH").is_some_and(|p| env::split_paths(&p).any(|d| d == bin));
    if !on_path {
        println!("note: {} is not on your PATH", bin.display());
        #[cfg(not(windows))]
        println!("      add it with: export PATH=\"{}:$PATH\"", bin.display());
    }
    Ok(())
}

/// Lets the run open the native libraries of the packages `mote.toml` in `root` grants.
fn grant_natives(root: &Path) {
    ffi::builtins::set_native_packages(pkg::native::grants(root));
}

/// The entry `mote test` compiles for a package: its own entry, or a generated one that also imports the `.mote` files in `tests/`.
fn package_test_entry(root: &Path, entry: &Path) -> Result<PathBuf, String> {
    let Ok(read) = fs::read_dir(root.join("tests")) else { return Ok(entry.to_path_buf()) };
    let mut stems: Vec<String> = read
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "mote"))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    if stems.is_empty() {
        return Ok(entry.to_path_buf());
    }
    stems.sort();
    let rel = entry.strip_prefix(root).unwrap_or(entry).with_extension("");
    let segments: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let mut text = format!("import ..{} as package_entry\n", segments.join("."));
    for stem in &stems {
        text.push_str(&format!("import ..tests.{stem} as test_{stem}\n"));
    }
    let generated = root.join(".build").join("test_entry.mote");
    ensure_parent(&generated)?;
    fs::write(&generated, text).map_err(|e| format!("Failed to write '{}': {}", generated.display(), e))?;
    Ok(generated)
}

/// Runs the current package, with `program_args` as the program's command line after its name.
fn run_package_entry(gc: gc::GCConfig, workers: usize, program_args: Vec<String>) {
    let (root, manifest) = discover_package_or_exit("run");
    grant_natives(&root);
    if !program_args.is_empty() {
        let mut argv = vec![manifest.package.name.clone()];
        argv.extend(program_args);
        ffi::builtins::set_script_args(argv);
    }
    if let Some(value) = manifest.run.and_then(|run| run.max_heap) {
        let _ = MANIFEST_MAX_HEAP.set(value);
    }
    match pkg::PackageManager::compile_program(&root) {
        Ok((_mbc_path, compiled)) => execute_compiled(compiled, gc, workers),
        Err(e) => {
            eprintln!("{}", e);
            process::exit(1);
        }
    }
}

fn main() {
    match cli::standalone::attached() {
        Some(Ok(compiled)) => cli::standalone::run::<{ cli::builtins::TIER_GUI }>(compiled),
        Some(Err(e)) => {
            eprintln!("Error executing standalone payload: {}", e);
            process::exit(1);
        }
        None => {}
    }

    let args: Vec<String> = env::args().collect();
    let (args, gc) = parse_gc_flag(args);
    let args = parse_allow_native_flag(args);
    let args = parse_max_heap_flag(args);
    let args = parse_mem_stats_flag(args);
    let (args, workers) = parse_workers_flag(args);
    let (args, workers) = parse_seed_flag(args, workers);
    let (args, filter) = parse_filter_flag(args);
    let release = args.iter().any(|a| a == "--release");
    compiler::span::set_release(release);
    let args: Vec<String> = args.into_iter().filter(|a| a != "--release").collect();
    if args.len() < 2 {
        print!("{USAGE}");
        return;
    }

    let command = &args[1];
    match command.as_str() {
        "--version" | "-V" | "version" => {
            println!("mote {}", cli::VERSION);
        }
        "--help" | "-h" | "help" => {
            print!("{USAGE}");
        }
        "build" | "bundle" => {
            if command == "bundle" {
                eprintln!("note: `mote bundle` is deprecated; use `mote build`");
            }
            let explicit_out = args
                .iter()
                .position(|a| a == "-o")
                .and_then(|pos| args.get(pos + 1))
                .map(PathBuf::from);
            let entry_file = args.get(2).filter(|a| !a.starts_with('-'));
            let tier_of = cli::builtins::tier_of;

            let (result, output_exe): (Result<(), String>, PathBuf) = match entry_file {
                Some(entry_file) => {
                    let stem = Path::new(entry_file)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("app")
                        .to_string();
                    let out = explicit_out.clone().unwrap_or_else(|| Path::new("dist").join(exe_name(&stem)));
                    (
                        ensure_parent(&out).and_then(|()| pkg::StandaloneBundler::bundle(Path::new(entry_file), &out, &tier_of)),
                        out,
                    )
                }
                None => {
                    let (root, manifest) = discover_package_or_exit("build");
                    let out = explicit_out.clone().unwrap_or_else(|| root.join("dist").join(exe_name(&manifest.package.name)));
                    let r = pkg::PackageManager::compile_program(&root)
                        .and_then(|(_p, compiled)| pkg::StandaloneBundler::bundle_program(&compiled, &out, &tier_of))
                        .and_then(|()| pkg::native::copy_libraries(&root, &out))
                        .map(|count| {
                            if count > 0 {
                                println!("Copied native libraries of {count} package(s) to {}", pkg::native::lib_dir(&out).display());
                            }
                        });
                    (r, out)
                }
            };

            match result {
                Ok(()) => println!("Built standalone executable: {}", output_exe.display()),
                Err(e) => {
                    eprintln!("Error building standalone executable: {}", e);
                    process::exit(1);
                }
            }
        }
        "init" => {
            let is_lib = args.iter().any(|a| a == "--lib");
            let name = args.get(2).filter(|a| !a.starts_with("--")).map(|s| s.as_str());
            match pkg::PackageManager::init_project(Path::new("."), name, is_lib) {
                Ok(()) => println!("Initialized new Mote package successfully."),
                Err(e) => {
                    eprintln!("Error initializing package: {}", e);
                    process::exit(1);
                }
            }
        }
        "add" => {
            if args.len() < 3 {
                eprintln!("Error: missing package name for 'add' command");
                process::exit(1);
            }
            let pkg = &args[2];
            match pkg::PackageManager::add_dependency(Path::new("."), pkg) {
                Ok(()) => println!("Added dependency '{}' and updated lockfile.", pkg),
                Err(e) => {
                    eprintln!("Error adding dependency: {}", e);
                    process::exit(1);
                }
            }
        }
        "remove" => {
            if args.len() < 3 {
                eprintln!("Error: missing package name for 'remove' command");
                process::exit(1);
            }
            let pkg = &args[2];
            match pkg::PackageManager::remove_dependency(Path::new("."), pkg) {
                Ok(()) => println!("Removed dependency '{}' and updated lockfile.", pkg),
                Err(e) => {
                    eprintln!("Error removing dependency: {}", e);
                    process::exit(1);
                }
            }
        }
        "sync" => {
            let locked = args.iter().skip(2).any(|a| a == "--locked");
            match pkg::PackageManager::install_dependencies(Path::new("."), locked) {
                Ok(pkgs) => println!("Synced {} dependencies successfully.", pkgs.len()),
                Err(e) => {
                    eprintln!("Error syncing dependencies: {}", e);
                    process::exit(1);
                }
            }
        }
        "install" => {
            let target = args.get(2).filter(|a| !a.starts_with('-')).map(String::as_str);
            if let Err(e) = install_program(target) {
                eprintln!("Error installing: {e}");
                process::exit(1);
            }
        }
        "compile" | "-c" => {
            let explicit_out = args.iter().position(|a| a == "-o").and_then(|pos| args.get(pos + 1)).map(PathBuf::from);
            let result = match args.get(2).filter(|a| !a.starts_with('-')) {
                Some(file) => {
                    let file = Path::new(file);
                    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("app");
                    let out = explicit_out.unwrap_or_else(|| Path::new("dist").join(format!("{stem}.mbc")));
                    let root = file.parent().unwrap_or(Path::new(".")).to_path_buf();
                    modules::MultiFileCompiler::new(root)
                        .compile_program(file)
                        .and_then(|compiled| ensure_parent(&out).and_then(|()| pkg::MbcFile::write(&compiled, &out)))
                        .map(|()| out)
                }
                None => {
                    let (root, _manifest) = discover_package_or_exit("compile");
                    match explicit_out {
                        Some(out) => pkg::PackageManager::compile_program(&root)
                            .and_then(|(_p, compiled)| ensure_parent(&out).and_then(|()| pkg::MbcFile::write(&compiled, &out)))
                            .map(|()| out),
                        None => pkg::PackageManager::compile(&root),
                    }
                }
            };
            match result {
                Ok(path) => println!("Compiled bytecode: {}", path.display()),
                Err(e) => {
                    eprintln!("{}", e);
                    process::exit(1);
                }
            }
        }
        "package" => {
            let (root, _manifest) = discover_package_or_exit("package");
            match pkg::PackageManager::package(&root) {
                Ok(path) => println!("Package archive: {}", path.display()),
                Err(e) => {
                    eprintln!("Error packaging artifact: {}", e);
                    process::exit(1);
                }
            }
        }
        "run" => {
            if args.len() < 3 || args[2] == "--" {
                run_package_entry(gc, workers, args.iter().skip(3).cloned().collect());
                return;
            }
            let filename = &args[2];
            let file_path = PathBuf::from(filename);
            ffi::builtins::set_script_args(args[2..].to_vec());

            if filename.ends_with(".mote") {
                let root_dir = file_path.parent().unwrap_or(Path::new(".")).to_path_buf();
                grant_natives(&root_dir);
                let mut compiler = modules::MultiFileCompiler::new(root_dir);
                match compiler.compile_program(&file_path) {
                    Ok(compiled) => execute_compiled(compiled, gc.clone(), workers),
                    Err(e) => {
                        eprintln!("{}", e);
                        process::exit(1);
                    }
                }
            } else if filename.ends_with(".mbc") {
                grant_natives(file_path.parent().unwrap_or(Path::new(".")));
                match pkg::MbcFile::read(&file_path) {
                    Ok(compiled) => execute_compiled(compiled, gc.clone(), workers),
                    Err(e) => {
                        eprintln!("{}", e);
                        process::exit(1);
                    }
                }
            } else {
                let source = match fs::read_to_string(filename) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("Error reading file '{}': {}", filename, e);
                        process::exit(1);
                    }
                };
                match cli::run_source(&source) {
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("Runtime error: {}", e);
                        process::exit(1);
                    }
                }
            }
        }
        "test" => {
            let filter_ref = filter.as_deref();
            let has_file = args.get(2).is_some_and(|a| !a.starts_with('-'));
            if !has_file {
                let (root, manifest) = discover_package_or_exit("test");
                let mut grants = pkg::native::grants(&root);
                let own = root.join("native").join(pkg::native::host_triple());
                if own.is_dir() {
                    grants.push((manifest.package.name.clone(), own));
                }
                ffi::builtins::set_native_packages(grants);
                let entry = manifest.entry_path(&root);
                if !entry.is_file() {
                    eprintln!(
                        "Error: entry '{}' from {}/mote.toml does not exist",
                        manifest.package.entry,
                        root.display()
                    );
                    process::exit(1);
                }
                let entry = match package_test_entry(&root, &entry) {
                    Ok(entry) => entry,
                    Err(e) => {
                        eprintln!("Error: {e}");
                        process::exit(1);
                    }
                };
                let mut compiler = modules::MultiFileCompiler::new(root.clone()).with_cache(root.join(".build"));
                match compiler.compile_program_for_test(&entry, filter_ref) {
                    Ok(compiled) => execute_compiled(compiled, gc, workers),
                    Err(e) => {
                        eprintln!("{}", e);
                        process::exit(1);
                    }
                }
                return;
            }
            let filename = &args[2];
            let file_path = PathBuf::from(filename);
            if !filename.ends_with(".mote") {
                eprintln!("Error: `mote test` needs a `.mote` source file");
                process::exit(1);
            }
            let root_dir = file_path.parent().unwrap_or(Path::new(".")).to_path_buf();
            grant_natives(&root_dir);
            let mut compiler = modules::MultiFileCompiler::new(root_dir);
            match compiler.compile_program_for_test(&file_path, filter_ref) {
                Ok(compiled) => execute_compiled(compiled, gc.clone(), workers),
                Err(e) => {
                    eprintln!("{}", e);
                    process::exit(1);
                }
            }
        }
        "check" => {
            let memory = args.iter().any(|a| a == "--memory");
            let args: Vec<String> = args.into_iter().filter(|a| a != "--memory").collect();
            let with_report = |c: modules::MultiFileCompiler| if memory { c.with_memory_report() } else { c };
            let print_report = |prog: &compiler::codegen::CompiledProgram| {
                if memory {
                    for line in cli::memreport::lines(&prog.memory_sites) {
                        println!("{line}");
                    }
                }
            };
            if args.len() < 3 {
                let (root, manifest) = discover_package_or_exit("check");
                let entry = manifest.entry_path(&root);
                if !entry.is_file() {
                    eprintln!(
                        "Error: entry '{}' from {}/mote.toml does not exist",
                        manifest.package.entry,
                        root.display()
                    );
                    process::exit(1);
                }
                let mut compiler = with_report(modules::MultiFileCompiler::new(root.clone()).with_cache(root.join(".build")));
                match compiler.compile_program(&entry) {
                    Ok(prog) => {
                        print_report(&prog);
                        println!(
                            "Compile OK. {} function(s), {} type(s).",
                            prog.code_objects.len(),
                            prog.type_descriptors.len()
                        );
                        return;
                    }
                    Err(e) => {
                        eprintln!("{}", e);
                        process::exit(1);
                    }
                }
            }
            let filename = &args[2];
            let file_path = PathBuf::from(filename);

            if filename.ends_with(".mote") {
                let root_dir = file_path.parent().unwrap_or(Path::new(".")).to_path_buf();
                let mut compiler = with_report(modules::MultiFileCompiler::new(root_dir));
                match compiler.compile_program(&file_path) {
                    Ok(prog) => {
                        print_report(&prog);
                        println!("Compile OK. {} function(s), {} type(s).", prog.code_objects.len(), prog.type_descriptors.len());
                    }
                    Err(e) => {
                        eprintln!("{}", e);
                        process::exit(1);
                    }
                }
            } else if filename.ends_with(".mbc") {
                match pkg::MbcFile::read(&file_path) {
                    Ok(prog) => {
                        println!("Loaded OK. {} function(s), {} type(s).", prog.code_objects.len(), prog.type_descriptors.len());
                    }
                    Err(e) => {
                        eprintln!("{}", e);
                        process::exit(1);
                    }
                }
            } else {
                let source = match fs::read_to_string(filename) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("Error reading file '{}': {}", filename, e);
                        process::exit(1);
                    }
                };
                match cli::assemble(&source) {
                    Ok(prog) => {
                        println!("Assembly OK. {} function(s), {} type(s).", prog.code_objects.len(), prog.type_descriptors.len());
                    }
                    Err(e) => {
                        eprintln!("Assembly error: {}", e);
                        process::exit(1);
                    }
                }
            }
        }
        other => {
            let file_path = PathBuf::from(other);
            if other.ends_with(".mote") && file_path.exists() {
                let root_dir = file_path.parent().unwrap_or(Path::new(".")).to_path_buf();
                let mut compiler = modules::MultiFileCompiler::new(root_dir);
                match compiler.compile_program(&file_path) {
                    Ok(compiled) => execute_compiled(compiled, gc.clone(), workers),
                    Err(e) => {
                        eprintln!("{}", e);
                        process::exit(1);
                    }
                }
            } else if other.ends_with(".mbc") && file_path.exists() {
                match pkg::MbcFile::read(&file_path) {
                    Ok(compiled) => execute_compiled(compiled, gc.clone(), workers),
                    Err(e) => {
                        eprintln!("{}", e);
                        process::exit(1);
                    }
                }
            } else {
                let source = match fs::read_to_string(other) {
                    Ok(s) => s,
                    Err(_) => {
                        eprintln!("Unknown command or file '{}'", other);
                        process::exit(1);
                    }
                };
                match cli::run_source(&source) {
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("Runtime error: {}", e);
                        process::exit(1);
                    }
                }
            }
        }
    }
}

