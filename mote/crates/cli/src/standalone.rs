//! Runs a program bundled into an executable; shared by `mote` and `mote-rt`.
use std::env;
use std::process;

fn fail(message: impl std::fmt::Display) -> ! {
    eprintln!("Error: {message}");
    process::exit(1)
}

/// The bundled program of the running executable: `None` when nothing is attached.
pub fn attached() -> Option<Result<compiler::CompiledProgram, String>> {
    pkg::StandaloneBundler::detect_and_read_payload()
}

/// Runs `compiled` on the scheduler and exits with its status; `TIER` is the natives to install.
pub fn run<const TIER: u8>(compiled: compiler::CompiledProgram) -> ! {
    ffi::builtins::set_script_args(env::args().collect());
    ffi::builtins::set_allow_native(env::var("MOTE_ALLOW_NATIVE").is_ok_and(|v| v == "1"));
    if let Ok(exe) = env::current_exe() {
        ffi::builtins::set_native_packages(pkg::native::built_grants(&exe));
    }
    let mut registry = isa::value::TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let global_count = compiled.global_count as usize;
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    rt.set_sources(compiled.sources);
    rt.set_global_count(global_count);
    if TIER >= crate::builtins::TIER_GUI {
        crate::builtins::install(&mut rt);
    } else if TIER >= crate::builtins::TIER_FULL {
        crate::builtins::install_full(&mut rt);
    } else {
        crate::builtins::install_lean(&mut rt);
    }
    rt.set_native_table(&compiled.native_table).expect("native table");
    let env_heap = env::var(crate::limits::MAX_HEAP_ENV).ok();
    let limit = crate::limits::resolve_max_heap(None, env_heap.as_deref(), None, platform::memory::available_bytes()).unwrap_or_else(|e| fail(e));
    rt.set_heap(gc::build_gc_engine(&gc::GCConfig::default().with_max_heap(limit.unwrap_or(usize::MAX))));
    if let Err(e) = crate::limits::apply(&mut rt) {
        fail(e);
    }
    let workers = crate::resolve_workers(None, env::var(crate::WORKERS_ENV).ok().as_deref()).unwrap_or_else(|e| fail(e));
    match rt.run_entry_on(workers).map(|_| rt.status()) {
        Ok(runtime::VmStatus::Exited(code)) => process::exit(code),
        Ok(_) => process::exit(0),
        Err(e) => {
            eprintln!("Runtime error: {e}");
            process::exit(1)
        }
    }
}

/// Runs the attached program, or says what is missing.
pub fn main<const TIER: u8>() -> ! {
    match attached() {
        Some(Ok(compiled)) => run::<TIER>(compiled),
        Some(Err(e)) => {
            eprintln!("Error executing standalone payload: {e}");
            process::exit(1)
        }
        None => {
            eprintln!("this runs a program that `mote build` attached to it");
            process::exit(2)
        }
    }
}
