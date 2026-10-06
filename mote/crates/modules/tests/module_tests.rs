use std::fs;
use std::path::{Path, PathBuf};
use compiler::ast::*;
use compiler::span::Span;
use isa::value::TypeRegistry;
use modules::{CanonicalModuleId, DependencyGraph, ModuleResolver, MultiFileCompiler, VisibilityChecker};

fn setup_temp_dir(test_name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mote_mod_test_{}_{}", test_name, std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn test_module_resolution_file_and_dir() {
    let temp = setup_temp_dir("res_file_dir");
    let math_file = temp.join("math.mote");
    fs::write(&math_file, "pub fn add(a: Int, b: Int) -> Int { return a + b }").unwrap();

    let geom_dir = temp.join("geometry");
    fs::create_dir_all(&geom_dir).unwrap();
    let mod_file = geom_dir.join("mod.mote");
    fs::write(&mod_file, "pub fn area(w: Int, h: Int) -> Int { return w * h }").unwrap();

    let mut resolver = ModuleResolver::new(temp.clone());

    let math_path = ModulePath::new(vec!["math".into()], false, 0, Span::new(0, 0, 1, 1));
    let (id1, _) = resolver.resolve_path(&math_path, &temp).unwrap();
    assert!(id1.0.contains("math.mote"));

    let geom_path = ModulePath::new(vec!["geometry".into()], false, 0, Span::new(0, 0, 1, 1));
    let (id2, _) = resolver.resolve_path(&geom_path, &temp).unwrap();
    assert!(id2.0.contains("mod.mote"));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_multifile_compilation_and_execution() {
    let temp = setup_temp_dir("multifile_exec");
    let helper_file = temp.join("helper.mote");
    fs::write(&helper_file, "pub fn compute(x: Int) -> Int { return x * 10 }").unwrap();

    let main_file = temp.join("main.mote");
    fs::write(
        &main_file,
        "import { * } from .helper\nfn main() -> Int {\n    let val = compute(5)\n    return val + 2\n}\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler.compile_program(&main_file).unwrap();

    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(52));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_multifile_list_intrinsic_and_gc() {
    let temp = setup_temp_dir("multifile_list");
    fs::write(
        temp.join("mk.mote"),
        "pub fn seed() -> List<Int> {\n    var xs = [1, 2, 3]\n    return xs\n}",
    )
    .unwrap();
    let main_file = temp.join("main.mote");
    fs::write(
        &main_file,
        "import { * } from .mk\nfn main() -> Int {\n    var xs = seed()\n    for i in 0..100 {\n        xs.push(i)\n    }\n    return xs.get(102) + xs.len()\n}\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler.compile_program(&main_file).unwrap();

    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    rt.set_heap(Box::new(gc::GCController::new(gc::GCConfig::default())));
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(99 + 103));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_std_iter_eager_adapters() {
    let temp = setup_temp_dir("std_iter");
    let main_file = temp.join("main.mote");
    fs::write(
        &main_file,
        "\
import std.iter as itr
fn inc(n: Int) -> Int { return n + 1 }
fn main() -> Int {
    let xs = itr.range(5)          // [0,1,2,3,4]
    let ys = itr.map(xs, inc)      // [1,2,3,4,5]
    let zs = itr.reverse(ys)       // [5,4,3,2,1]
    return itr.sum(zs) + zs.get(0) + itr.take(xs, 2).len()
}
return main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler.compile_program(&main_file).unwrap();

    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    rt.set_heap(Box::new(gc::GCController::new(gc::GCConfig::default())));
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(22));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_std_collections_set_algebra() {
    let temp = setup_temp_dir("std_collections");
    let main_file = temp.join("main.mote");
    fs::write(
        &main_file,
        "\
import std.collections as col
fn main() -> Int {
    let a = col.set_from([1, 2, 3, 4, 5])
    let b = col.set_from([4, 5, 6, 7])
    let u = col.union(a, b)                 // {1..7}
    let i = col.intersection(a, b)          // {4, 5}
    let d = col.difference(a, b)            // {1, 2, 3}
    return u.len() * 100 + i.len() * 10 + d.len()
}
return main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler.compile_program(&main_file).unwrap();

    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    rt.set_heap(Box::new(gc::GCController::new(gc::GCConfig::default())));
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(7 * 100 + 2 * 10 + 3));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_std_string_helpers() {
    let temp = setup_temp_dir("std_string");
    let main_file = temp.join("main.mote");
    fs::write(
        &main_file,
        "\
import std.string as str
fn main() -> Int {
    let parts = \"a.b.c.d\".split(\".\")   // [a, b, c, d]
    let joined = str.join(parts, \"/\")   // a/b/c/d
    let padded = str.pad_start(\"7\", 3, \"0\")  // 007
    var ok = 0
    if joined == \"a/b/c/d\" { ok = ok + 1 }
    if padded == \"007\" { ok = ok + 1 }
    if str.lines(\"x\\ny\").len() == 2 { ok = ok + 1 }
    if str.is_blank(\"   \") { ok = ok + 1 }
    return ok
}
return main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler.compile_program(&main_file).unwrap();

    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    rt.set_heap(Box::new(gc::GCController::new(gc::GCConfig::default())));
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(4));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_std_math() {
    let temp = setup_temp_dir("std_math");
    let main_file = temp.join("main.mote");
    fs::write(
        &main_file,
        "\
import std.math as math
fn main() -> Int {
    let a = math.to_int(math.sqrt(144.0))      // 12
    let b = math.gcd(48, 18)                    // 6
    let c = math.clamp(99, 0, 10)              // 10
    let d = math.ipow(2, 8)                    // 256
    let e = math.to_int(math.floor(math.pi())) // 3
    let f = math.saturating_add(9223372036854775807, 5) - 9223372036854775807  // 0
    let g = math.abs(0 - 4) + math.max(2, 7)   // 4 + 7 = 11
    return a + b + c + d + e + f + g            // 12+6+10+256+3+0+11 = 298
}
return main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler.compile_program(&main_file).unwrap();

    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    rt.set_heap(Box::new(gc::GCController::new(gc::GCConfig::default())));
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(298));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_std_env() {
    let temp = setup_temp_dir("std_env");
    let main_file = temp.join("main.mote");
    fs::write(
        &main_file,
        "\
import std.sys.env as env

fn main() -> Int {
    let a = env.args()
    let missing = env.get_var(\"MOTE_TEST_DEFINITELY_UNSET_VAR_XYZ\")
    let vars = env.vars()
    var n = a.len()
    if missing.is_none() {
        n = n + 1
    }
    if vars.len() > 0 {
        n = n + 1
    }
    return n
}
return main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler.compile_program(&main_file).unwrap();

    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    rt.set_heap(Box::new(gc::GCController::new(gc::GCConfig::default())));
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(2));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_struct_methods_and_copy_across_modules() {
    let temp = setup_temp_dir("struct_methods_multifile");
    fs::write(
        temp.join("geom.mote"),
        "\
pub struct Vec2 {
    var x: Int
    var y: Int

    pub fn of(x: Int, y: Int) -> Vec2 { return Vec2 { x: x, y: y } }
    pub fn sum(self) -> Int { return self.x + self.y }
    pub fn shift(var self, dx: Int) { self.x = self.x + dx }
}
",
    )
    .unwrap();
    let main_file = temp.join("main.mote");
    fs::write(
        &main_file,
        "\
import { Vec2 } from .geom

fn main() -> Int {
    let a = Vec2.of(1, 2)
    var b = a
    b.shift(10)
    return a.sum() * 100 + b.sum()
}
return main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler.compile_program(&main_file).unwrap();
    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    rt.set_heap(Box::new(gc::GCController::new(gc::GCConfig::default())));
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(313));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_std_iter_sort_with_comparator_lambda() {
    let temp = setup_temp_dir("std_sort");
    let main_file = temp.join("main.mote");
    fs::write(
        &main_file,
        "\
import std.iter as itr
fn main() -> Int {
    let xs = [5, 2, 8, 1, 9, 3]
    let asc = itr.sort(xs)                       // [1,2,3,5,8,9]
    let desc = itr.sort_by(xs, |a, b| a > b)     // [9,8,5,3,2,1]
    // asc.get(0)=1, desc.get(0)=9, xs untouched (xs.get(0)=5), asc.len()=6
    return asc.get(0) * 100 + desc.get(0) * 10 + xs.get(0) + asc.len()
}
return main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler.compile_program(&main_file).unwrap();
    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    rt.set_heap(Box::new(gc::GCController::new(gc::GCConfig::default())));
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(201));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_stdlib_done_bar_program() {
    let temp = setup_temp_dir("done_bar");
    let main_file = temp.join("main.mote");
    fs::write(
        &main_file,
        "\
import std.iter as itr
import std.math as math

// A job record has a name and a weight; `Map<String,Int>` holds name -> weight.
struct Job {
    name: String
    weight: Int
}

fn job(name: String, weight: Int) -> Job {
    return Job { name: name, weight: weight }
}

fn main() -> Int {
    let jobs = [job(\"build\", 3), job(\"test\", 5), job(\"deploy\", 1), job(\"lint\", 4)]

    var weights: Map<String, Int> = Map()
    for j in jobs {
        weights.set(j.name, j.weight)
    }

    // Rank by descending weight (comparator lambda over the record lists).
    let ranked = itr.sort_by(jobs, |a, b| a.weight > b.weight)

    var report = \"\"
    var total = 0
    for j in ranked {
        let w = j.weight
        let r = math.to_int(math.round(math.sqrt(math.to_float(w)) * 100.0))
        report = report + \"${j.name}=${weights.get(j.name)}(${r}) \"
        total = total + w
    }

    // \"test=5(224) lint=4(200) build=3(173) deploy=1(100) \"
    if report == \"test=5(224) lint=4(200) build=3(173) deploy=1(100) \" {
        return total + itr.sum(itr.map(jobs, |j| j.weight))   // 13 + 13
    }
    return 0
}
return main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler.compile_program(&main_file).unwrap();
    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    rt.set_heap(Box::new(gc::GCController::new(gc::GCConfig::default())));
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(26));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_circular_import_cycle_detection() {
    let temp = setup_temp_dir("cycle_detect");
    let a_file = temp.join("a.mote");
    let b_file = temp.join("b.mote");

    fs::write(&a_file, "import .b\npub fn fa() -> Int { return 1 }").unwrap();
    fs::write(&b_file, "import .a\npub fn fb() -> Int { return 2 }").unwrap();

    let mut resolver = ModuleResolver::new(temp.clone());
    let graph_res = DependencyGraph::build(a_file, &mut resolver);
    assert!(graph_res.is_err());
    let err_msg = graph_res.unwrap_err();
    assert!(err_msg.contains("Circular import detected"));
    assert!(!err_msg.contains(temp.to_string_lossy().as_ref()));
    assert!(err_msg.contains("a -> b -> a") || err_msg.contains("b -> a -> b"));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_circular_import_cycle_names_disambiguate_on_collision() {
    let temp = setup_temp_dir("cycle_detect_collide");
    let sub_dir = temp.join("sub");
    fs::create_dir_all(&sub_dir).unwrap();

    let util_file = temp.join("util.mote");
    let sub_util_file = sub_dir.join("util.mote");

    fs::write(&util_file, "import .sub.util\npub fn fa() -> Int { return 1 }").unwrap();
    fs::write(&sub_util_file, "import ..util\npub fn fb() -> Int { return 2 }").unwrap();

    let mut resolver = ModuleResolver::new(temp.clone());
    let graph_res = DependencyGraph::build(util_file.clone(), &mut resolver);
    assert!(graph_res.is_err());
    let err_msg = graph_res.unwrap_err();
    assert!(err_msg.contains("Circular import detected"));
    assert!(err_msg.contains(&util_file.to_string_lossy().to_string())
        || err_msg.contains(&sub_util_file.to_string_lossy().to_string()));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_visibility_pub_vs_private() {
    let mut vis = VisibilityChecker::new();
    let mod_id = CanonicalModuleId::new("math");

    let prog = Program {
        items: vec![
            Item::Function(FunctionDecl {
                name: "pub_calc".into(),
                generic_params: vec![],
                params: vec![],
                return_type: None,
                body: vec![],
                is_pub: true,
                span: Span::new(0, 0, 1, 1),
            }),
            Item::Function(FunctionDecl {
                name: "priv_calc".into(),
                generic_params: vec![],
                params: vec![],
                return_type: None,
                body: vec![],
                is_pub: false,
                span: Span::new(0, 0, 1, 1),
            }),
        ],
        span: Span::new(0, 0, 1, 1),
        stable_marks: vec![],
        test_marks: vec![],
    };

    let mut resolver = ModuleResolver::new(PathBuf::from("."));
    vis.register_module_exports(&mod_id, &prog, &mut resolver, Path::new("."))
        .unwrap();
    assert!(vis.is_symbol_exported(&mod_id, "pub_calc"));
    assert!(!vis.is_symbol_exported(&mod_id, "priv_calc"));

    assert!(vis.check_import_visibility(&mod_id, "pub_calc").is_ok());
    assert!(vis.check_import_visibility(&mod_id, "priv_calc").is_err());
}

#[test]
fn test_selective_import_of_private_symbol_is_rejected() {
    let temp = setup_temp_dir("sel_import_private");
    fs::write(
        temp.join("helper.mote"),
        "fn secret() -> Int { return 7 }\npub fn ok() -> Int { return 1 }",
    )
    .unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { secret } from .helper\nfn main() -> Int { return secret() }\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let err = compiler
        .compile_program(&temp.join("main.mote"))
        .expect_err("importing a private symbol must fail");
    assert!(err.contains("private symbol 'secret'"), "got: {err}");

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_selective_import_of_pub_symbol_compiles() {
    let temp = setup_temp_dir("sel_import_pub");
    fs::write(
        temp.join("helper.mote"),
        "pub fn compute(x: Int) -> Int { return x * 10 }",
    )
    .unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { compute } from .helper\nfn main() -> Int { return compute(5) }\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler
        .compile_program(&temp.join("main.mote"))
        .expect("selective import of a pub symbol should compile");

    let registry = TypeRegistry::new();
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(50));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_qualified_symbol_resolution() {
    let temp = setup_temp_dir("qual_sym");
    let utils_file = temp.join("utils.mote");
    fs::write(&utils_file, "pub fn double_val(x: Int) -> Int { return x * 2 }").unwrap();

    let main_file = temp.join("main.mote");
    fs::write(
        &main_file,
        "import .utils as u\nfn main() -> Int {\n    let res = u.double_val(21)\n    return res\n}\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler.compile_program(&main_file).unwrap();
    let registry = TypeRegistry::new();
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(42));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_bare_import_allows_qualified_calls() {
    let temp = setup_temp_dir("bare_qual");
    fs::write(
        temp.join("utils.mote"),
        "pub fn double_val(x: Int) -> Int { return x * 2 }",
    )
    .unwrap();
    fs::write(
        temp.join("main.mote"),
        "import .utils\nfn main() -> Int { return utils.double_val(21) }\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler.compile_program(&temp.join("main.mote")).unwrap();
    let registry = TypeRegistry::new();
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(42));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_super_import_resolves_to_parent_directory() {
    let temp = setup_temp_dir("super_import");
    fs::write(
        temp.join("config.mote"),
        "pub fn limit() -> Int { return 256 }",
    )
    .unwrap();
    let sub = temp.join("sub");
    fs::create_dir_all(&sub).unwrap();
    fs::write(
        sub.join("worker.mote"),
        "import super.config\npub fn cap() -> Int { return config.limit() }",
    )
    .unwrap();
    fs::write(
        temp.join("main.mote"),
        "import .sub.worker\nfn main() -> Int { return worker.cap() }\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler.compile_program(&temp.join("main.mote")).unwrap();
    let registry = TypeRegistry::new();
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(256));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_qualified_call_to_private_symbol_is_rejected() {
    let temp = setup_temp_dir("qual_private");
    fs::write(
        temp.join("utils.mote"),
        "fn hidden() -> Int { return 1 }\npub fn shown() -> Int { return 2 }",
    )
    .unwrap();
    fs::write(
        temp.join("main.mote"),
        "import .utils as u\nfn main() -> Int { return u.hidden() }\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let err = compiler
        .compile_program(&temp.join("main.mote"))
        .expect_err("a qualified call to a private symbol must fail");
    assert!(err.contains("hidden"), "got: {err}");

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_two_modules_may_define_the_same_fn_name() {
    let temp = setup_temp_dir("same_name");
    fs::write(temp.join("a.mote"), "pub fn helper() -> Int { return 10 }").unwrap();
    fs::write(temp.join("b.mote"), "pub fn helper() -> Int { return 20 }").unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { helper as ha } from .a\nimport { helper as hb } from .b\n\
         fn main() -> Int { return ha() + hb() }\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler
        .compile_program(&temp.join("main.mote"))
        .expect("two modules defining `helper` must not collide");
    let registry = TypeRegistry::new();
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(30));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_struct_type_crosses_module_boundary() {
    let temp = setup_temp_dir("cross_struct");
    fs::write(
        temp.join("geom.mote"),
        "pub struct Point {\n  pub x: Int\n  pub y: Int\n  pub fn new(x: Int, y: Int) -> Point { return Point { x: x, y: y } }\n}",
    )
    .unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { Point } from .geom\n\
         fn main() -> Int {\n  let p = Point.new(3, 4)\n  return p.x + p.y\n}\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler
        .compile_program(&temp.join("main.mote"))
        .expect("an imported struct type should resolve and lay out correctly");
    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(7));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_selective_import_of_pub_import_reexport_compiles_and_runs() {
    let temp = setup_temp_dir("reexport_selective");
    fs::write(temp.join("origin.mote"), "pub fn real_helper(x: Int) -> Int { return x + 1 }").unwrap();
    fs::write(
        temp.join("facade.mote"),
        "pub import { real_helper as helper } from .origin",
    )
    .unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { helper } from .facade\nfn main() -> Int { return helper(41) }\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler
        .compile_program(&temp.join("main.mote"))
        .expect("a selective import of a pub-import re-export should resolve to its real source");
    let registry = TypeRegistry::new();
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(42));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_reexport_chain_through_two_facades() {
    let temp = setup_temp_dir("reexport_chain");
    fs::write(temp.join("origin.mote"), "pub fn real() -> Int { return 9 }").unwrap();
    fs::write(temp.join("inner.mote"), "pub import { real } from .origin").unwrap();
    fs::write(temp.join("outer.mote"), "pub import { real } from .inner").unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { real } from .outer\nfn main() -> Int { return real() }\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler
        .compile_program(&temp.join("main.mote"))
        .expect("a chained re-export should resolve through both facades to the real definition");
    let registry = TypeRegistry::new();
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(9));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_reexport_of_private_symbol_is_rejected() {
    let temp = setup_temp_dir("reexport_private");
    fs::write(temp.join("origin.mote"), "fn secret() -> Int { return 1 }").unwrap();
    fs::write(
        temp.join("facade.mote"),
        "pub import { secret } from .origin",
    )
    .unwrap();
    fs::write(temp.join("main.mote"), "import { secret } from .facade\nreturn 0").unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let err = compiler
        .compile_program(&temp.join("main.mote"))
        .expect_err("re-exporting a private symbol must fail, not silently succeed");
    assert!(err.contains("secret"));
    assert!(err.contains("not `pub`") || err.contains("pub"));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_glob_and_qualified_alias_both_reach_a_reexport() {
    let temp = setup_temp_dir("reexport_glob_alias");
    fs::write(
        temp.join("origin.mote"),
        "pub fn real_fn() -> Int { return 5 }\npub struct Real {\n  pub n: Int\n  pub fn new(n: Int) -> Real { return Real { n: n } }\n}",
    )
    .unwrap();
    fs::write(
        temp.join("facade.mote"),
        "pub import { real_fn, Real } from .origin",
    )
    .unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { * } from .facade\n\
         fn main() -> Int {\n  let r = Real.new(real_fn())\n  return r.n\n}\nreturn main()",
    )
    .unwrap();
    fs::write(
        temp.join("qualified.mote"),
        "import .facade as f\nfn main() -> Int { return f.real_fn() }\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler
        .compile_program(&temp.join("main.mote"))
        .expect("a bare glob import should reach a re-exported fn and struct");
    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(5));

    let mut compiler2 = MultiFileCompiler::new(temp.clone());
    let compiled2 = compiler2
        .compile_program(&temp.join("qualified.mote"))
        .expect("a qualified alias.f() call should reach a re-exported fn");
    let registry2 = TypeRegistry::new();
    let mut rt2 = runtime::Runtime::with_type_registry(compiled2.code_objects, &registry2);
    let task2 = rt2.run_entry().unwrap();
    assert_eq!(task2.registers[0].as_int(), Some(5));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_unimported_symbol_is_not_reachable() {
    let temp = setup_temp_dir("unimported");
    fs::write(temp.join("helper.mote"), "pub fn secret_value() -> Int { return 7 }").unwrap();
    fs::write(
        temp.join("main.mote"),
        "fn main() -> Int { return secret_value() }\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let err = compiler
        .compile_program(&temp.join("main.mote"))
        .expect_err("an unimported cross-module symbol must not be reachable");
    assert!(
        err.contains("secret_value"),
        "expected an unresolved-symbol error, got: {err}"
    );

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_glob_import_does_not_expose_private_symbols() {
    let temp = setup_temp_dir("glob_private");
    fs::write(
        temp.join("helper.mote"),
        "fn secret() -> Int { return 9 }\npub fn ok() -> Int { return 1 }",
    )
    .unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { * } from .helper\nfn main() -> Int { return secret() }\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let err = compiler
        .compile_program(&temp.join("main.mote"))
        .expect_err("a private symbol must not be glob-imported");
    assert!(err.contains("secret"), "got: {err}");

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_ambiguous_glob_import_is_rejected() {
    let temp = setup_temp_dir("glob_ambig");
    fs::write(temp.join("a.mote"), "pub fn thing() -> Int { return 1 }").unwrap();
    fs::write(temp.join("b.mote"), "pub fn thing() -> Int { return 2 }").unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { * } from .a\nimport { * } from .b\nfn main() -> Int { return thing() }\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let err = compiler
        .compile_program(&temp.join("main.mote"))
        .expect_err("`thing` glob-imported from two modules must be ambiguous");
    assert!(err.contains("thing"), "got: {err}");

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_one_name_glob_imported_through_a_facade_and_its_module_is_not_ambiguous() {
    let temp = setup_temp_dir("glob_facade");
    fs::write(temp.join("origin.mote"), "pub fn thing() -> Int { return 7 }").unwrap();
    fs::write(temp.join("facade.mote"), "pub import { thing } from .origin").unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { * } from .facade\nimport { * } from .origin\nfn main() -> Int { return thing() }\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    compiler.compile_program(&temp.join("main.mote")).expect("the same function through two paths is one name");

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_a_package_module_imports_by_the_package_and_file_name() {
    let temp = setup_temp_dir("pkg_module");
    let src = temp.join(".mote_packages").join("kit").join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("lib.mote"), "pub fn base() -> Int { return 1 }").unwrap();
    fs::write(src.join("extra.mote"), "pub fn more() -> Int { return 40 }").unwrap();
    fs::write(
        temp.join("main.mote"),
        "import kit\nimport kit.extra\nfn main() -> Int { return kit.base() + extra.more() }\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler.compile_program(&temp.join("main.mote")).unwrap();
    let registry = TypeRegistry::new();
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(41));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_a_glob_import_beats_the_prelude_for_the_same_name() {
    let temp = setup_temp_dir("glob_prelude");
    fs::write(
        temp.join("main.mote"),
        "import { * } from std.sys.gui\nfn main() -> Int {\n    let d = Display.Flex\n    return 1\n}\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    compiler.compile_program(&temp.join("main.mote")).expect("`Display` from std.sys.gui must win over the prelude's");

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_directory_package_and_submodules() {
    let temp = setup_temp_dir("dir_pkg");
    let linalg_dir = temp.join("linalg");
    fs::create_dir_all(&linalg_dir).unwrap();
    let vec_file = linalg_dir.join("vector.mote");
    fs::write(&vec_file, "pub fn dot2(x1: Int, y1: Int, x2: Int, y2: Int) -> Int { return x1 * x2 + y1 * y2 }").unwrap();

    let mod_file = linalg_dir.join("mod.mote");
    fs::write(&mod_file, "import { * } from .vector\npub fn calc() -> Int { return dot2(2, 3, 4, 5) }").unwrap();

    let main_file = temp.join("main.mote");
    fs::write(&main_file, "import { * } from .linalg\nfn main() -> Int { return calc() }\nreturn main()").unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler.compile_program(&main_file).unwrap();
    let registry = TypeRegistry::new();
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(23));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_module_initialization_ordering() {
    let temp = setup_temp_dir("init_order");
    let dep_file = temp.join("dep.mote");
    fs::write(&dep_file, "pub fn get_flag() -> Int { let init_flag = 100\n return init_flag }").unwrap();

    let main_file = temp.join("main.mote");
    fs::write(&main_file, "import { * } from .dep\nfn main() -> Int { return get_flag() }\nreturn main()").unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler.compile_program(&main_file).unwrap();
    let registry = TypeRegistry::new();
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(100));

    fs::remove_dir_all(temp).ok();
}

fn run_multifile(temp: &Path, entry: &str) -> i64 {
    let mut compiler = MultiFileCompiler::new(temp.to_path_buf());
    let compiled = compiler
        .compile_program(&temp.join(entry))
        .unwrap_or_else(|e| panic!("compile failed: {e}"));
    let registry = TypeRegistry::new();
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    rt.set_global_count(compiled.global_count as usize);
    let task = rt.run_entry().unwrap();
    task.registers[0].as_int().expect("r0 should be an Int")
}

#[test]
fn test_selective_import_of_a_pub_global() {
    let temp = setup_temp_dir("import_pub_global");
    fs::write(temp.join("config.mote"), "pub let MAX = 42\nlet SECRET = 7").unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { MAX } from .config\nfn main() -> Int { return MAX }\nreturn main()",
    )
    .unwrap();

    assert_eq!(run_multifile(&temp, "main.mote"), 42);
    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_importing_a_private_global_is_rejected() {
    let temp = setup_temp_dir("import_private_global");
    fs::write(temp.join("config.mote"), "let SECRET = 7").unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { SECRET } from .config\nfn main() -> Int { return SECRET }\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let err = compiler
        .compile_program(&temp.join("main.mote"))
        .expect_err("importing a non-`pub` global must fail");
    assert!(err.contains("private symbol 'SECRET'"), "got: {err}");
    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_glob_import_brings_in_pub_globals() {
    let temp = setup_temp_dir("glob_pub_global");
    fs::write(temp.join("config.mote"), "pub let MAX = 42").unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { * } from .config\nfn main() -> Int { return MAX + 1 }\nreturn main()",
    )
    .unwrap();

    assert_eq!(run_multifile(&temp, "main.mote"), 43);
    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_imported_pub_var_is_readable_but_not_writable_from_a_function() {
    let temp = setup_temp_dir("import_pub_var");
    fs::write(temp.join("config.mote"), "pub var hits = 0").unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { hits } from .config\n\
         fn read() -> Int { return hits }\n\
         fn main() -> Int { return read() }\n\
         return main()",
    )
    .unwrap();

    assert_eq!(run_multifile(&temp, "main.mote"), 0);
    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_writing_an_imported_pub_var_from_a_function_is_a_globals_frozen_error() {
    let temp = setup_temp_dir("import_pub_var_write_from_fn");
    fs::write(temp.join("config.mote"), "pub var hits = 0").unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { hits } from .config\n\
         fn bump() { hits = hits + 1 }\n\
         fn main() -> Int { bump()\n return hits }\n\
         return main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let err = compiler
        .compile_program(&temp.join("main.mote"))
        .expect_err("writing a global from a function is a globals-frozen error");
    assert!(err.contains("global"), "got: {err}");
    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_assigning_an_imported_pub_let_is_an_error() {
    let temp = setup_temp_dir("import_pub_let_immutable");
    fs::write(temp.join("config.mote"), "pub let MAX = 42").unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { MAX } from .config\nfn main() -> Int { MAX = 1\n return MAX }\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let err = compiler
        .compile_program(&temp.join("main.mote"))
        .expect_err("an imported `pub let` is immutable");
    assert!(err.contains("immutable constant"), "got: {err}");
    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_pub_global_re_exported_through_a_facade() {
    let temp = setup_temp_dir("reexport_pub_global");
    fs::write(temp.join("config.mote"), "pub let MAX = 42").unwrap();
    fs::write(temp.join("facade.mote"), "pub import { MAX } from .config").unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { MAX } from .facade\nfn main() -> Int { return MAX }\nreturn main()",
    )
    .unwrap();

    assert_eq!(run_multifile(&temp, "main.mote"), 42);
    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_a_non_entry_modules_top_level_runs_once_before_main() {
    let temp = setup_temp_dir("modinit_once");
    fs::write(
        temp.join("util.mote"),
        "pub var boot = 0\nboot = boot + 1\npub fn get() -> Int { return boot }",
    )
    .unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { boot } from .util\nimport { * } from .util\nfn main() -> Int { return get() + boot }\nreturn main()",
    )
    .unwrap();

    assert_eq!(run_multifile(&temp, "main.mote"), 2);
    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_module_inits_run_deepest_dependency_first() {
    let temp = setup_temp_dir("modinit_order");
    fs::write(temp.join("c.mote"), "pub var trace = 0\ntrace = trace * 10 + 3").unwrap();
    fs::write(
        temp.join("b.mote"),
        "import { trace } from .c\ntrace = trace * 10 + 2\npub fn bb() -> Int { return 0 }",
    )
    .unwrap();
    fs::write(
        temp.join("a.mote"),
        "import { trace } from .c\nimport { * } from .b\ntrace = trace * 10 + 1\npub fn aa() -> Int { return bb() }",
    )
    .unwrap();
    fs::write(
        temp.join("main.mote"),
        "import .a\nimport { trace } from .c\nfn main() -> Int { return trace }\nreturn main()",
    )
    .unwrap();

    assert_eq!(run_multifile(&temp, "main.mote"), 321);
    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_module_with_no_top_level_gets_no_init_object() {
    let temp = setup_temp_dir("modinit_none");
    fs::write(temp.join("lib.mote"), "pub fn double(x: Int) -> Int { return x * 2 }").unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { double } from .lib\nfn main() -> Int { return double(21) }\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler.compile_program(&temp.join("main.mote")).unwrap();
    assert_eq!(compiled.code_objects.len(), 9);

    assert_eq!(run_multifile(&temp, "main.mote"), 42);
    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_module_with_top_level_adds_one_init_object() {
    let temp = setup_temp_dir("modinit_count");
    fs::write(temp.join("lib.mote"), "pub var ready = 0\nready = 9").unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { ready } from .lib\nfn main() -> Int { return ready }\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler.compile_program(&temp.join("main.mote")).unwrap();
    assert_eq!(compiled.code_objects.len(), 9);

    assert_eq!(run_multifile(&temp, "main.mote"), 9);
    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_module_import_error_names_the_module_file() {
    let temp = setup_temp_dir("relative_error_path");
    fs::write(temp.join("ext.mote"), "let SECRET = 1").unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { SECRET } from .ext\nfn main() -> Int { return SECRET }\nreturn main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let err = compiler
        .compile_program(&temp.join("main.mote"))
        .expect_err("importing a private symbol must fail");
    assert!(err.contains("ext.mote") && err.contains("main.mote"), "got: {err}");
    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_topological_sort_ties_are_deterministic_by_discovery_order() {
    let temp = setup_temp_dir("topo_tie_break");
    fs::write(temp.join("shared.mote"), "pub fn fs() -> Int { return 3 }").unwrap();
    fs::write(temp.join("a.mote"), "import { * } from .shared\npub fn fa() -> Int { return fs() }").unwrap();
    fs::write(temp.join("b.mote"), "import { * } from .shared\npub fn fb() -> Int { return fs() }").unwrap();
    let main_file = temp.join("main.mote");
    fs::write(&main_file, "import .a\nimport .b\npub fn noop() -> Int { return 0 }").unwrap();

    let mut resolver = ModuleResolver::new(temp.clone());
    let graph = DependencyGraph::build(main_file, &mut resolver).unwrap();

    let expected: Vec<&str> = vec!["shared", "a", "b", "main"];
    for _ in 0..25 {
        let order = graph.topological_sort().unwrap();
        let names: Vec<&str> = order
            .iter()
            .filter(|id| !id.as_str().starts_with("<std>/"))
            .map(|id| id.display_name())
            .collect();
        assert_eq!(names, expected);
    }

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_import_native_is_an_error() {
    let temp = setup_temp_dir("import_native_gone");
    let main_file = temp.join("main.mote");
    fs::write(&main_file, "import native sqlite3\nfn main() -> Int {\n    return 1\n}\nreturn main()").unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let err = compiler.compile_program(&main_file).unwrap_err();
    assert!(err.contains("there is no `import native`"), "got: {err}");

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_native_fn_in_a_module_is_an_error() {
    let temp = setup_temp_dir("native_fn_gone");
    fs::write(temp.join("mathlib.mote"), "pub native fn add(a: Int, b: Int) -> Int").unwrap();
    let main_file = temp.join("main.mote");
    fs::write(&main_file, "import .mathlib\nreturn 0").unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let err = compiler.compile_program(&main_file).unwrap_err();
    assert!(err.contains("`native fn` is only for the compiler's own builtins"), "got: {err}");

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_cross_module_enum_import_and_match() {
    let temp = setup_temp_dir("cross_module_enum");
    fs::write(
        temp.join("shapes.mote"),
        "pub enum Shape { Circle(Int), Square(Int), Point }",
    )
    .unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { Shape } from .shapes\n\
fn area(s: Shape) -> Int {\n\
    match s {\n\
        Circle(r) => { return r * r * 3 }\n\
        Square(w) => { return w * w }\n\
        Point => { return 0 }\n\
    }\n\
}\n\
fn main() -> Int { return area(Shape.Circle(4)) + area(Shape.Square(5)) }\n\
return main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler
        .compile_program(&temp.join("main.mote"))
        .expect("cross-module enum should compile");

    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(48 + 25));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_prelude_option_is_auto_imported_into_every_module() {
    let temp = setup_temp_dir("auto_prelude_option");
    fs::write(
        temp.join("pick.mote"),
        "pub fn pick(n: Int) -> Option<Int> {\n\
             if n > 0 { return Some(n * 2) }\n\
             return None\n\
         }",
    )
    .unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { pick } from .pick\n\
         fn main() -> Int {\n\
             match pick(21) {\n\
                 Some(v) => { return v }\n\
                 None => { return -1 }\n\
             }\n\
         }\n\
         return main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler
        .compile_program(&temp.join("main.mote"))
        .expect("auto-prelude Option should compile");

    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(42));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_unknown_std_module_is_a_clean_error() {
    let temp = setup_temp_dir("unknown_std");
    fs::write(
        temp.join("main.mote"),
        "import std.telepathy\nfn main() -> Int { return 1 }\nreturn main()",
    )
    .unwrap();
    let mut compiler = MultiFileCompiler::new(temp.clone());
    let err = compiler.compile_program(&temp.join("main.mote")).unwrap_err();
    assert!(err.contains("standard-library module"), "got: {err}");
    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_try_operator_across_modules() {
    let temp = setup_temp_dir("try_across_modules");
    fs::write(
        temp.join("db.mote"),
        "pub fn get(k: Int) -> Result<Int, Int> {\n\
             if k > 0 { return Ok(k * 3) }\n\
             return Err(0)\n\
         }",
    )
    .unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { get } from .db\n\
         fn tripled_plus_one(k: Int) -> Result<Int, Int> {\n\
             let v = get(k)?\n\
             return Ok(v + 1)\n\
         }\n\
         fn main() -> Int {\n\
             match tripled_plus_one(14) {\n\
                 Ok(v) => { return v }\n\
                 Err(e) => { return -1 }\n\
             }\n\
         }\n\
         return main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler
        .compile_program(&temp.join("main.mote"))
        .expect("cross-module `?` should compile");

    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(43));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_builtin_method_dispatch_across_modules() {
    let temp = setup_temp_dir("method_dispatch_modules");
    fs::write(
        temp.join("lookup.mote"),
        "pub fn lookup(k: Int) -> Option<Int> {\n\
             if k > 10 { return Some(k) }\n\
             return None\n\
         }",
    )
    .unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { lookup } from .lookup\n\
         fn main() -> Int {\n\
             let a = lookup(42).unwrap_or(0)\n\
             var found = 0\n\
             if lookup(3).is_none() { found = 1 }\n\
             return a + found\n\
         }\n\
         return main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler
        .compile_program(&temp.join("main.mote"))
        .expect("cross-module method dispatch should compile");

    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(43));

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_type_checking_error_renders_a_readable_source_line_not_a_raw_span_dump() {
    let temp = setup_temp_dir("readable_type_error");
    fs::write(
        temp.join("helper.mote"),
        "import { Any } from std.experimental.types\npub fn identity(val: Any) -> Float {\n    return val\n}\n",
    )
    .unwrap();
    fs::write(
        temp.join("main.mote"),
        "import { identity } from .helper\n\
         fn main() -> Int {\n\
         \x20   let v: Float = 10.0\n\
         \x20   let w: Int = identity(v)\n\
         \x20   return w\n\
         }\n\
         return main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let err = compiler
        .compile_program(&temp.join("main.mote"))
        .expect_err("assigning a Float-returning call to an Int variable must fail");

    assert!(!err.contains("Span {"), "should not leak a raw Span debug dump, got: {err}");
    assert!(
        err.contains("let w: Int = identity(v)"),
        "should reconstruct the actual offending source line, got: {err}"
    );
    assert!(err.contains("main.mote:4"), "should name the file and line, got: {err}");
    assert!(err.contains('^'), "should underline the span, got: {err}");

    fs::remove_dir_all(temp).ok();
}

#[test]
fn any_boundary_check_faults_across_a_module_import_the_exact_original_repro() {
    let temp = setup_temp_dir("any_boundary_multifile");
    fs::create_dir_all(temp.join("tests")).unwrap();
    fs::write(
        temp.join("tests").join("test_any.mote"),
        "import { Any } from std.experimental.types\npub fn test_func_any(val: Any) -> String {\n    return val\n}\n",
    )
    .unwrap();
    fs::write(
        temp.join("main.mote"),
        "import {test_func_any} from .tests.test_any\n\
         fn main() -> Int {\n\
         \x20   let v: Float = 10.75\n\
         \x20   let w: String = test_func_any(v)\n\
         \x20   return 0\n\
         }\n\
         return main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler
        .compile_program(&temp.join("main.mote"))
        .expect("compiles cleanly — the checker can't see through Any, so this must be a runtime check");

    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    let err = rt.run_entry().unwrap_err();
    assert!(format!("{:?}", err).contains("expected"), "got: {:?}", err);

    fs::remove_dir_all(temp).ok();
}

#[test]
fn test_std_random_seeded_determinism_and_ranges() {
    let temp = setup_temp_dir("std_random");
    let main_file = temp.join("main.mote");
    fs::write(
        &main_file,
        "\
import { Rng, random } from std.random

fn main() -> Int {
    var a = Rng.seeded(12345)
    var b = Rng.seeded(12345)
    var i = 0
    while i < 50 {
        if a.next_int(0, 1000) != b.next_int(0, 1000) { return 1 }
        i = i + 1
    }
    var c = Rng.seeded(1)
    var d = Rng.seeded(2)
    if c.next_bits() == d.next_bits() { return 2 }

    var r = Rng.seeded(7)
    var seen_lo = false
    var seen_hi = false
    var k = 0
    while k < 500 {
        let n = r.next_int(-3, 4)
        if n < -3 { return 3 }
        if n > 3 { return 4 }
        if n == -3 { seen_lo = true }
        if n == 3 { seen_hi = true }
        let f = r.next_float()
        if f < 0.0 { return 5 }
        if f >= 1.0 { return 6 }
        k = k + 1
    }
    if !seen_lo { return 7 }
    if !seen_hi { return 8 }

    let g = random()
    if g < 0.0 { return 9 }
    if g >= 1.0 { return 10 }

    var e = Rng.from_entropy()
    let empty: List<Int> = []
    match e.choice(empty) {
        Some(v) => { return 11 }
        None => {}
    }

    var xs = [1, 2, 3, 4, 5, 6, 7, 8]
    var s1 = Rng.seeded(99)
    s1.shuffle(xs)
    var total = 0
    for x in xs { total = total + x }
    if total != 36 { return 12 }
    if xs.len() != 8 { return 13 }
    var moved = false
    var m = 0
    while m < 8 {
        if xs.get(m) != m + 1 { moved = true }
        m = m + 1
    }
    if !moved { return 14 }

    var s2 = Rng.seeded(5)
    match s2.choice([\"x\", \"y\", \"z\"]) {
        Some(v) => { if v.len() != 1 { return 15 } }
        None => { return 16 }
    }
    return 42
}
return main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(temp.clone());
    let compiled = compiler.compile_program(&main_file).unwrap();
    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    rt.set_heap(Box::new(gc::GCController::new(gc::GCConfig::default())));
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(42));

    fs::remove_dir_all(temp).ok();
}
