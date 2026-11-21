use cranelift_codegen::ir::{types as clif, AbiParam};
use mote::{Options, Pipeline, Target};
use mote_abi::{FaultKind, FLAG_STATIC};
use mote_packages::testing::Scratch;

use super::*;

/// The program built without the optimizer, so its operations stay.
fn built(source: &str) -> Pipeline {
    let scratch = Scratch::new();
    let dir = scratch.package("app", "app", &[], &[("main.mote", source)]);
    Pipeline::build(&dir, Target::Run, &Options { optimize: false, ..Options::default() }).unwrap_or_else(|failure| panic!("{failure}"))
}

const CHECKS: &str = "fn grow(a: Int8, b: Float32) -> Int8 {\n    if b > 1.5 {\n        return a\n    }\n    return a + 1\n}\n\nfn main() {\n    let n = 4\n    println(grow(1, 2.0))\n    println(100 / n)\n    println(1 << n)\n    println(n * n)\n}\n";

#[test]
fn jit_a_function_is_tail_convention_with_the_context_first() {
    let mut pipeline = built(CHECKS);
    let types = typetable::build(&pipeline.module, &mut pipeline.program, &pipeline.interner);
    let signatures: Vec<Signature> = pipeline.module.functions.iter().filter_map(|(_, function)| function_signature(function, false, &types).ok()).collect();
    assert!(!signatures.is_empty());
    for signature in &signatures {
        assert_eq!(signature.call_conv, CallConv::Tail);
        assert_eq!(signature.params[0], AbiParam::new(clif::I64));
    }
    let grow = AbiParam::new(clif::I8);
    assert!(signatures.iter().any(|signature| signature.params == [AbiParam::new(clif::I64), grow, AbiParam::new(clif::F32)] && signature.returns == [grow]));
}

#[test]
fn jit_each_check_has_a_site_with_its_kind_and_the_prologue_has_the_stack_site() {
    let mut pipeline = built(CHECKS);
    let compiled = compile(&pipeline.module, &mut pipeline.program, &pipeline.interner, EntryKind::Main).unwrap();
    let kinds: Vec<FaultKind> = (0..compiled.faults.len() as u32).map(|raw| compiled.faults.get_raw(raw).unwrap().kind).collect();
    for kind in [FaultKind::StackDepth, FaultKind::IntegerOverflow, FaultKind::DivideByZero, FaultKind::ShiftOutOfRange] {
        assert!(kinds.contains(&kind), "no site of kind {kind:?} in {kinds:?}");
    }
}

const SHAPES: &str = "struct Wide {\n    a: Int\n    b: Int\n    c: Int\n    d: Int\n    e: Int\n}\n\nstruct Pair {\n    x: Float\n    y: Float\n}\n\nfn total(w: Wide, p: Pair) -> Int {\n    return w.a + w.e + (p.x as Int)\n}\n\nfn make(seed: Int) -> Wide {\n    return Wide { a: seed, b: 1, c: 2, d: 3, e: 4 }\n}\n\nfn bump(var n: Int, by: Int8) {\n    n += by as Int\n}\n\nfn main() {\n    var n = 1\n    bump(n, 2)\n    let w = make(n)\n    println(total(w, Pair { x: 1.5, y: 2.5 }))\n}\n";

/// The signature of the function with the name in the program.
fn signature_of(source: &str, name: &str, closure_conv: bool) -> Signature {
    let mut pipeline = built(source);
    let types = typetable::build(&pipeline.module, &mut pipeline.program, &pipeline.interner);
    let (_, function) = pipeline
        .module
        .functions
        .iter()
        .find(|(_, function)| matches!(&function.kind, mote_ir::FunctionKind::Instance(instance) if pipeline.interner.resolve(pipeline.program.defs[instance.def].name) == name))
        .unwrap_or_else(|| panic!("no function {name}"));
    function_signature(function, closure_conv, &types).unwrap()
}

#[test]
fn jit_a_value_past_four_leaves_goes_by_address_and_a_small_one_by_its_leaves() {
    let (i64_, f64_) = (AbiParam::new(clif::I64), AbiParam::new(clif::F64));
    assert_eq!(signature_of(SHAPES, "total", false).params, [i64_, i64_, f64_, f64_]);
    assert_eq!(signature_of(SHAPES, "make", false).params, [i64_, i64_, i64_]);
    assert!(signature_of(SHAPES, "make", false).returns.is_empty());
}

#[test]
fn jit_a_var_parameter_adds_one_last_parameter_for_the_block_of_final_values() {
    let (i64_, i8_) = (AbiParam::new(clif::I64), AbiParam::new(clif::I8));
    assert_eq!(signature_of(SHAPES, "bump", false).params, [i64_, i64_, i8_, i64_]);
}

const VALUES: &str = "fn double(n: Int) -> Int {\n    return n * 2\n}\n\nfn main() {\n    let f: (Int) -> Int = double\n    println(f(2))\n}\n";

#[test]
fn jit_a_function_called_through_a_closure_takes_it_after_the_context() {
    let i64_ = AbiParam::new(clif::I64);
    assert_eq!(signature_of(VALUES, "double", false).params, [i64_, i64_]);
    assert_eq!(signature_of(VALUES, "double", true).params, [i64_, i64_, i64_]);
}

#[test]
fn jit_an_entry_that_returns_an_err_gives_the_message_of_its_error() {
    let mut pipeline = built("fn main() -> Result<(), Error> {\n    println(\"before\")\n    return Err(Error.other(\"boom\"))\n}\n");
    let ran = compile(&pipeline.module, &mut pipeline.program, &pipeline.interner, EntryKind::Main).unwrap().run();
    assert_eq!((ran.output.as_str(), ran.err.as_deref(), ran.fault), ("before\n", Some("boom"), None));
}

#[test]
fn jit_an_entry_that_returns_an_ok_has_no_error() {
    let mut pipeline = built("fn main() -> Result<Int, Error> {\n    return Ok(3)\n}\n");
    let ran = compile(&pipeline.module, &mut pipeline.program, &pipeline.interner, EntryKind::Main).unwrap().run();
    assert_eq!((ran.err, ran.fault), (None, None));
}

#[test]
fn jit_a_run_returns_what_it_printed() {
    let mut pipeline = built(CHECKS);
    let ran = compile(&pipeline.module, &mut pipeline.program, &pipeline.interner, EntryKind::Main).unwrap().run();
    assert_eq!(ran.output, "1\n25\n16\n16\n");
    assert_eq!(ran.fault, None);
}

#[test]
fn jit_the_perf_map_lists_each_function_with_its_start_size_and_name() {
    let mut pipeline = built(CHECKS);
    let compiled = compile(&pipeline.module, &mut pipeline.program, &pipeline.interner, EntryKind::Main).unwrap();
    let map = compiled.perf_map();
    let mut lines: Vec<(usize, usize, &str)> = map
        .lines()
        .map(|line| {
            let mut words = line.splitn(3, ' ');
            let (start, size) = (usize::from_str_radix(words.next().unwrap(), 16).unwrap(), usize::from_str_radix(words.next().unwrap(), 16).unwrap());
            (start, size, words.next().unwrap())
        })
        .collect();
    assert!(lines.iter().any(|(_, _, name)| *name == "main") && lines.iter().any(|(_, _, name)| *name == "grow"), "{map}");
    lines.sort();
    for pair in lines.windows(2) {
        assert!(pair[0].0 + pair[0].1 <= pair[1].0, "{map}");
    }
    assert!(lines.iter().all(|(_, size, _)| *size > 0));
}

#[test]
fn jit_a_string_constant_is_an_object_with_a_header_a_length_and_its_bytes_padded_to_a_word() {
    let id = mote_abi::TypeId::from_raw(7);
    let object = string_object(id, "hello");
    assert_eq!(object.len(), 24);
    assert_eq!(Header::from_bits(u64::from_le_bytes(object[0..8].try_into().unwrap())), Header::new(id, FLAG_STATIC));
    assert_eq!(u64::from_le_bytes(object[8..16].try_into().unwrap()), 5);
    assert_eq!(&object[16..21], b"hello");
    assert_eq!(&object[21..], [0, 0, 0]);
    assert_eq!(string_object(id, "").len(), 16);
}

#[test]
fn jit_every_store_is_emitted_by_one_function() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut emitting = Vec::new();
    for entry in std::fs::read_dir(&src).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            for (line, text) in text.lines().enumerate() {
                if text.contains(".store(") && text.contains("ins()") || text.contains("ins().istore") {
                    emitting.push(format!("{}:{}", path.file_name().unwrap().to_string_lossy(), line + 1));
                }
            }
        }
    }
    assert_eq!(emitting.len(), 1, "{emitting:?}");
    assert!(emitting[0].starts_with("values.rs:"));
}

const HELD: &str = "fn pick(names: List<String>, at: Int) -> String {\n    let first = names[0]\n    println(at)\n    return if at == 0 { first } else { names[at] }\n}\n\nfn main() {\n    var names: List<String> = List.new()\n    names.push(\"a\")\n    names.push(\"b\")\n    println(pick(names, 1))\n}\n";

#[test]
fn jit_each_call_has_a_descriptor_that_lists_the_words_of_the_references_and_never_the_probe() {
    let mut pipeline = built(HELD);
    let compiled = compile(&pipeline.module, &mut pipeline.program, &pipeline.interner, EntryKind::Main).unwrap();
    let descriptors: Vec<_> = compiled.code.descriptors().collect();
    assert!(descriptors.len() > 4, "{}", descriptors.len());
    for (_, descriptor) in &descriptors {
        assert!(descriptor.refs.iter().all(|&offset| offset != 0 && offset < descriptor.frame_size), "{descriptor:?}");
    }
    assert!(descriptors.iter().any(|(_, descriptor)| descriptor.refs.len() >= 2));
}

#[test]
fn jit_detects_single_threaded_and_runs_direct_fast_path() {
    let mut pipeline = built("import Instant from std.time\nfn main() {\n    var xs = [1, 2, 3]\n    xs.push(4)\n    println(xs.len())\n    let t = Instant.now()\n    println(t.elapsed().as_nanos() >= 0)\n}\n");
    let compiled = compile(&pipeline.module, &mut pipeline.program, &pipeline.interner, EntryKind::Main).unwrap();
    assert!(compiled.single_threaded);
    let ran = compiled.run();
    assert!(ran.fault.is_none(), "{:?}", ran.fault);
    assert_eq!(ran.output, "4\ntrue\n");

    let mut spawned = built("fn main() {\n    let t = spawn { 1 + 1 }\n    println(t.await())\n}\n");
    let compiled = compile(&spawned.module, &mut spawned.program, &spawned.interner, EntryKind::Main).unwrap();
    assert!(!compiled.single_threaded);
}

#[test]
fn jit_pointer_and_buffer_fault_on_null_oob_and_use_after_close() {
    let mut null_load = built("import std.mem.*\nfn main() {\n    let p = Pointer<Int>.nil()\n    println(p.load())\n}\n");
    let ran = compile(&null_load.module, &mut null_load.program, &null_load.interner, EntryKind::Main).unwrap().run();
    assert_eq!(ran.fault.as_ref().map(|f| f.kind), Some(mote_abi::FaultKind::NullPointer));

    let mut neg_alloc = built("import std.mem.*\nfn main() {\n    let p = Pointer<Int>.alloc(-1)\n}\n");
    let ran = compile(&neg_alloc.module, &mut neg_alloc.program, &neg_alloc.interner, EntryKind::Main).unwrap().run();
    assert_eq!(ran.fault.as_ref().map(|f| f.kind), Some(mote_abi::FaultKind::IndexOutOfRange));

    let mut closed = built("import std.mem.{Buffer}\nfn main() {\n    let b = Buffer<Int>.alloc(2)\n    let _ = b.close()\n    println(b.get(0))\n}\n");
    let ran = compile(&closed.module, &mut closed.program, &closed.interner, EntryKind::Main).unwrap().run();
    assert_eq!(ran.fault.as_ref().map(|f| f.kind), Some(mote_abi::FaultKind::Raised));
    assert!(ran.fault.as_ref().unwrap().message.contains("closed"));
}
