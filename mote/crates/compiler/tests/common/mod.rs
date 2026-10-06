//! Test support: compiles with the compiler crate, runs on the real runtime (a dev-dependency only).

use isa::value::Value;

pub struct Compiler;

impl Compiler {
    pub fn compile(source: &str, file_name: &str) -> Result<compiler::CompiledProgram, String> {
        compiler::Compiler::compile(source, file_name)
    }

    pub fn run(source: &str) -> Result<Value, String> {
        let compiled = Self::compile(source, "<input>")?;
        let global_count = compiled.global_count as usize;
        let mut rt = runtime::Runtime::with_types(compiled.code_objects, compiled.type_descriptors);
        rt.set_global_count(global_count);
        ffi::builtins::install(&mut rt);
        rt.set_native_table(&compiled.native_table).unwrap();
        rt.set_heap(Box::new(gc::GCController::new(gc::GCConfig::default())));
        let task = rt.run_entry().map_err(|e| format!("Runtime Error: {:?}", e))?;
        Ok(task.registers[0])
    }
}
