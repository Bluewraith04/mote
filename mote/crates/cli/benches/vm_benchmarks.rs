use std::sync::Arc;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use cli::assemble;
use runtime::Runtime;

fn bench_arithmetic_comparison(c: &mut Criterion) {
    let source = include_str!("../../../tests/programs/arithmetic_comparison.masm");
    let prog = assemble(source).unwrap();
    c.bench_function("mote_arithmetic_comparison", |b| {
        b.iter(|| {
            let mut rt = Runtime::with_types(prog.code_objects.clone(), prog.type_descriptors.clone());
            let task = rt.run_entry().unwrap();
            black_box(&task.registers[2]);
        })
    });
}

fn bench_control_flow(c: &mut Criterion) {
    let source = include_str!("../../../tests/programs/control_flow.masm");
    let prog = assemble(source).unwrap();
    c.bench_function("mote_control_flow_loop", |b| {
        b.iter(|| {
            let mut rt = Runtime::with_types(prog.code_objects.clone(), prog.type_descriptors.clone());
            let task = rt.run_entry().unwrap();
            black_box(&task.registers[2]);
        })
    });
}

fn bench_function_calls(c: &mut Criterion) {
    let source = include_str!("../../../tests/programs/function_calls.masm");
    let prog = assemble(source).unwrap();
    c.bench_function("mote_function_calls_fib10", |b| {
        b.iter(|| {
            let mut rt = Runtime::with_types(prog.code_objects.clone(), prog.type_descriptors.clone());
            let task = rt.run_entry().unwrap();
            black_box(&task.registers[4]);
        })
    });
}

fn bench_objects(c: &mut Criterion) {
    let source = include_str!("../../../tests/programs/objects.masm");
    let prog = assemble(source).unwrap();
    c.bench_function("mote_objects_allocation_traversal", |b| {
        b.iter(|| {
            let mut rt = Runtime::with_types(prog.code_objects.clone(), prog.type_descriptors.clone());
            let task = rt.run_entry().unwrap();
            black_box(&task.registers[16]);
        })
    });
}

fn bench_alloc_paths(c: &mut Criterion) {
    let arena_src = r#"
    .type @Point id=100 fields=x,y

    .func @main regs=8 params=0
        ENTERARENA
        ARENAALLOC r0, @Point
        LOADI r1, 10
        LOADI r2, 20
        SETFIELD r0, 0, r1
        SETFIELD r0, 1, r2
        EXITARENA
        HALT
    .end
    "#;
    let heap_src = r#"
    .type @Point id=100 fields=x,y

    .func @main regs=8 params=0
        NEWOBJ r0, @Point
        LOADI r1, 10
        LOADI r2, 20
        SETFIELD r0, 0, r1
        SETFIELD r0, 1, r2
        HALT
    .end
    "#;

    let prog_arena = assemble(arena_src).unwrap();
    let prog_heap = assemble(heap_src).unwrap();

    c.bench_function("mote_arena_alloc_scope", |b| {
        b.iter(|| {
            let mut rt = Runtime::with_types(prog_arena.code_objects.clone(), prog_arena.type_descriptors.clone());
            let task = rt.run_entry().unwrap();
            black_box(&task.registers[0]);
        })
    });

    c.bench_function("mote_newobj_bump_alloc", |b| {
        b.iter(|| {
            let mut rt = Runtime::with_types(prog_heap.code_objects.clone(), prog_heap.type_descriptors.clone());
            let task = rt.run_entry().unwrap();
            black_box(&task.registers[0]);
        })
    });
}

fn bench_integer_add(c: &mut Criterion) {
    let vt_src = r#"
    .func @main regs=8 params=0
        LOADI r0, 10
        LOADI r1, 20
        ADD r2, r0, r1
        HALT
    .end
    "#;
    let prog_vt = assemble(vt_src).unwrap();

    c.bench_function("mote_integer_add_loop", |b| {
        b.iter(|| {
            let mut rt = Runtime::with_types(prog_vt.code_objects.clone(), prog_vt.type_descriptors.clone());
            let task = rt.run_entry().unwrap();
            black_box(&task.registers[2]);
        })
    });
}

fn bench_native_calls(c: &mut Criterion) {
    let native_reg = ffi::NativeFunctionRegistry::new();
    native_reg.register("bench_add", |ctx| {
        let a = ctx.arg(0).and_then(|v| v.as_int()).unwrap_or(0);
        let b = ctx.arg(1).and_then(|v| v.as_int()).unwrap_or(0);
        Ok(isa::value::Value::int(a + b))
    });

    let src = r#"
    .func @main regs=8 params=0
        LOADI r0, 10
        LOADI r1, 20
        CALLNATIVE r2, 0, r0
        HALT
    .end
    "#;
    let prog = assemble(src).unwrap();

    let reg_clone = native_reg.clone();
    c.bench_function("mote_native_call_roundtrip", |b| {
        b.iter(|| {
            let mut rt = Runtime::with_types(prog.code_objects.clone(), prog.type_descriptors.clone());
            let reg = reg_clone.clone();
            rt.set_native_dispatcher(Arc::new(move |idx, heap: &mut dyn runtime::NativeCtx, args| {
                reg.call(idx, args, heap)
            }));
            let task = rt.run_entry().unwrap();
            black_box(&task.registers[2]);
        })
    });
}

fn bench_compiler_throughput(c: &mut Criterion) {
    let source = "fn fib(n: Int) -> Int {\n    if n < 2 {\n        return n\n    }\n    return fib(n - 1) + fib(n - 2)\n}\nlet res = fib(10)\n";
    c.bench_function("mote_compiler_throughput", |b| {
        b.iter(|| {
            let compiled = compiler::Compiler::compile(source, "fib.mote").unwrap();
            black_box(compiled.code_objects.len());
        })
    });
}

fn bench_compiler_end_to_end(c: &mut Criterion) {
    let source = "let a = 10\nlet b = 20\nlet c = a + b * 2\n";
    c.bench_function("mote_compiler_end_to_end", |b| {
        b.iter(|| {
            let res = compiler::Compiler::compile(source, "bench.mote").unwrap();
            black_box(res);
        })
    });
}

fn bench_multifile_compilation_throughput(c: &mut Criterion) {
    let temp_dir = std::env::temp_dir().join(format!("mote_bench_mod_{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let mod_a = temp_dir.join("mod_a.mote");
    let mod_b = temp_dir.join("mod_b.mote");
    std::fs::write(&mod_a, "pub fn helper(x: Int) -> Int { return x + 1 }\n").unwrap();
    std::fs::write(&mod_b, "import .mod_a\nfn main() -> Int { return helper(10) }\nmain()\n").unwrap();

    c.bench_function("mote_multifile_compilation_throughput", |b| {
        b.iter(|| {
            let mut compiler = modules::MultiFileCompiler::new(temp_dir.clone());
            let compiled = compiler.compile_program(&mod_b).unwrap();
            black_box(compiled.code_objects.len());
        })
    });

    std::fs::remove_dir_all(temp_dir).ok();
}

fn bench_dependency_resolution_performance(c: &mut Criterion) {
    let mut solver = pkg::DependencyResolver::new();
    for i in 0..10 {
        let name = format!("pkg_{}", i);
        let mut deps = std::collections::HashMap::new();
        if i > 0 {
            deps.insert(format!("pkg_{}", i - 1), "^1.0.0".to_string());
        }
        solver.add_available_package(pkg::AvailablePackage {
            name,
            version: pkg::Version::new(1, 0, 0),
            dependencies: deps,
            source: "registry".into(),
        });
    }

    let manifest_toml = r#"
[package]
name = "root"
version = "1.0.0"

[dependencies]
pkg_9 = "^1.0.0"
"#;
    let manifest = pkg::PackageManifest::from_toml_str(manifest_toml).unwrap();

    c.bench_function("mote_dependency_resolution_performance", |b| {
        b.iter(|| {
            let resolved = solver.resolve(&manifest).unwrap();
            black_box(resolved.len());
        })
    });
}

criterion_group!(
    benches,
    bench_arithmetic_comparison,
    bench_control_flow,
    bench_function_calls,
    bench_objects,
    bench_alloc_paths,
    bench_integer_add,
    bench_native_calls,
    bench_compiler_throughput,
    bench_compiler_end_to_end,
    bench_multifile_compilation_throughput,
    bench_dependency_resolution_performance
);
criterion_main!(benches);

