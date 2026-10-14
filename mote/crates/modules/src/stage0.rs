//! Stage 0: runs a `@modifier` function at compile time in a VM that sees only its input.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use compiler::modifiers::Additions;
use compiler::CompiledProgram;
use contracts::{Platform, PlatformError, PlatformErrorKind, PlatformRequest, PlatformResponse, StdStream};
use sha2::{Digest, Sha256};

use crate::driver::MultiFileCompiler;

/// What one application may spend.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Safepoint hits: back-edges, calls and every 1024th instruction.
    pub reductions: u64,
    /// Live heap bytes.
    pub heap: usize,
    /// Bytes the modifier may print.
    pub output: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { reductions: 20_000_000, heap: 64 << 20, output: 1 << 20 }
    }
}

/// The only outside world a modifier has: its input as `args()` and a captured `println`.
struct Sandbox {
    input: String,
    output: Mutex<Vec<u8>>,
    cap: usize,
}

fn refused(what: &str) -> PlatformError {
    PlatformError { kind: PlatformErrorKind::PermissionDenied, message: format!("{what} is not available to a modifier") }
}

impl Platform for Sandbox {
    fn execute(&self, request: PlatformRequest) -> Result<PlatformResponse, PlatformError> {
        match request {
            PlatformRequest::Args => Ok(PlatformResponse::Strings(vec!["modifier".to_string(), self.input.clone()])),
            PlatformRequest::Write { stream: StdStream::Stdout, bytes } => {
                let mut out = self.output.lock().unwrap();
                if out.len() + bytes.len() > self.cap {
                    return Err(PlatformError { kind: PlatformErrorKind::Other, message: format!("output limit of {} bytes reached", self.cap) });
                }
                out.extend(bytes);
                Ok(PlatformResponse::Unit)
            }
            PlatformRequest::Write { stream: StdStream::Stderr, .. } | PlatformRequest::Flush { .. } => Ok(PlatformResponse::Unit),
            other => {
                let text = format!("{other:?}");
                Err(refused(text.split([' ', '{']).next().unwrap_or("this")))
            }
        }
    }

    fn blocking(&self, _request: &PlatformRequest) -> bool {
        false
    }
}

/// A `@modifier` function checked once and applied to any number of declarations.
pub struct Stage0 {
    name: String,
    module: PathBuf,
    compiler: MultiFileCompiler,
    program: Option<CompiledProgram>,
    /// Hash of everything an answer depends on except the declaration.
    key: String,
    reads: Vec<String>,
    cache: Option<PathBuf>,
    limits: Limits,
}

impl Stage0 {
    /// Checks `modifier_fn` of the module at `module` and what it imports.
    pub fn open(module: &Path, root: &Path, modifier_fn: &str, cache: Option<&Path>) -> Result<Stage0, String> {
        let mut compiler = MultiFileCompiler::new(root.to_path_buf());
        if let Some(dir) = cache {
            compiler = compiler.with_cache(dir.to_path_buf());
        }
        let (key, reads) = compiler.stage0_check(module, modifier_fn)?;
        Ok(Stage0 { name: modifier_fn.to_string(), module: module.to_path_buf(), compiler, program: None, key, reads, cache: cache.map(Path::to_path_buf), limits: Limits::default() })
    }

    /// The member attributes the modifier reads, from `@modifier(reads = "…")`.
    pub fn reads(&self) -> &[String] {
        &self.reads
    }

    /// [`open`](Self::open) with no cache and the modifier compiled at once.
    pub fn build(module: &Path, modifier_fn: &str) -> Result<Stage0, String> {
        let mut stage0 = Self::open(module, module.parent().unwrap_or(Path::new(".")), modifier_fn, None)?;
        stage0.compile()?;
        Ok(stage0)
    }

    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    fn compile(&mut self) -> Result<(), String> {
        if self.program.is_none() {
            self.program = Some(self.compiler.stage0_build(&self.module, &self.name)?);
        }
        Ok(())
    }

    fn entry(&self, dir: &Path, decl_json: &str) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(&self.key);
        hasher.update([0]);
        hasher.update(decl_json);
        let key: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        dir.join("stage0").join(&key[..2]).join(format!("{key}.json"))
    }

    /// Runs the modifier on the declaration `decl_json` describes.
    pub fn apply(&mut self, decl_json: &str) -> Result<Additions, String> {
        let file = self.cache.as_ref().map(|dir| self.entry(dir, decl_json));
        let cached = file.as_ref().and_then(|f| fs::read_to_string(f).ok()).and_then(|text| answer(&text));
        if let Some(answer) = cached {
            return answer;
        }
        self.compile()?;
        let text = self.run(decl_json)?;
        let Some(result) = answer(&text) else {
            return Err(format!("modifier `{}` answered nothing readable", self.name));
        };
        if let Some(file) = file {
            store(&file, &text);
        }
        result
    }

    fn run(&self, decl_json: &str) -> Result<String, String> {
        let sandbox = Arc::new(Sandbox { input: decl_json.to_string(), output: Mutex::new(Vec::new()), cap: self.limits.output });
        let program = self.program.clone().expect("compiled before it runs");
        let mut registry = isa::value::TypeRegistry::new();
        for t in &program.type_descriptors {
            let _ = registry.register(t.clone(), None);
        }
        let mut rt = runtime::Runtime::with_type_registry(program.code_objects, &registry);
        rt.set_sources(program.sources);
        rt.set_global_count(program.global_count as usize);
        ffi::builtins::install_lean(&mut rt);
        rt.set_native_table(&program.native_table).map_err(|e| format!("modifier `{}`: {e}", self.name))?;
        rt.set_heap(gc::build_gc_engine(&gc::GCConfig::mark_sweep().with_max_heap(self.limits.heap)));
        rt.set_platform(sandbox.clone());
        rt.set_reduction_limit(self.limits.reductions);
        if let Err(e) = rt.run_entry_on(1) {
            let spent = ["reduction limit", "out of memory", "output limit"].iter().any(|m| e.contains(m));
            return Err(if spent { format!("modifier `{}` did not finish: {e}", self.name) } else { format!("modifier `{}` failed: {e}", self.name) });
        }
        let text = String::from_utf8_lossy(&sandbox.output.lock().unwrap()).trim().to_string();
        Ok(text)
    }
}

fn store(file: &Path, text: &str) {
    let Some(parent) = file.parent() else { return };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let tmp = file.with_extension(format!("tmp{}", std::process::id()));
    if fs::write(&tmp, text).is_ok() && fs::rename(&tmp, file).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

/// The modifier's answer in `text`, or `None` when `text` is not an answer.
fn answer(text: &str) -> Option<Result<Additions, String>> {
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    if let Some(message) = json.get("err").and_then(|m| m.as_str()) {
        return Some(Err(message.to_string()));
    }
    let part = |key: &str| json.get("ok").and_then(|ok| ok.get(key)).and_then(|v| v.as_str()).map(String::from);
    Some(Ok(Additions { public: part("public")?, private: part("private")? }))
}
