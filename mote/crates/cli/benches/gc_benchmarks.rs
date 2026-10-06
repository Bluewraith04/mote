//! GC-pressure throughput benchmarks: `NoGC` as an allocation baseline and the default collector, over the same two workloads. Run: `cargo bench -p cli --bench gc_benchmarks`.

use std::ptr::NonNull;
use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, Criterion};

use gc::{GCConfig, GCController};
use isa::value::{TypeDescriptor, Value};
use contracts::Heap;
use runtime::{Runtime, TaskContext};

fn small_value_type() -> NonNull<TypeDescriptor> {
    let leaked = Box::leak(Box::new(TypeDescriptor::new(
        900,
        vec![Some("x".into()), Some("y".into()), Some("z".into())],
    )));
    NonNull::from(leaked)
}

fn node_type() -> NonNull<TypeDescriptor> {
    let leaked = Box::leak(Box::new(TypeDescriptor::with_pointer_mask(
        901,
        vec![Some("val".into()), Some("next".into())],
        0b10,
    )));
    NonNull::from(leaked)
}

fn fresh_runtime() -> (Runtime, TaskContext) {
    let rt = Runtime::new(vec![runtime::CodeObject::new(vec![], vec![], 8, 0)]);
    let task = TaskContext::entry(&rt);
    (rt, task)
}

fn garbage_chunk(gc: &GCController, rt: &Runtime, task: &mut TaskContext, type_ptr: NonNull<TypeDescriptor>, n: usize) {
    for i in 0..n {
        let obj = gc.alloc_object_with_slots(type_ptr, 3);
        unsafe {
            (*obj.as_ptr()).set_field(0, Value::small_int(i as i64));
        }
        task.registers[0] = Value::boxed(obj);
    }
    if gc.should_collect() {
        gc.collect(&mut runtime::RuntimeRoots::new(rt, task));
    }
}

fn chain_chunk(gc: &GCController, rt: &Runtime, task: &mut TaskContext, type_ptr: NonNull<TypeDescriptor>, garbage_per_chunk: usize) {
    for i in 0..garbage_per_chunk {
        let obj = gc.alloc_object_with_slots(type_ptr, 2);
        unsafe {
            (*obj.as_ptr()).set_field(0, Value::small_int(i as i64));
        }
        task.registers[0] = Value::boxed(obj);
    }
    if gc.should_collect() {
        gc.collect(&mut runtime::RuntimeRoots::new(rt, task));
    }
}

fn build_chain(gc: &GCController, task: &mut TaskContext, type_ptr: NonNull<TypeDescriptor>, chain_len: usize) {
    let mut head = Value::null();
    for i in 0..chain_len {
        let node = gc.alloc_object_with_slots(type_ptr, 2);
        unsafe {
            (*node.as_ptr()).set_field(0, Value::small_int(i as i64));
            (*node.as_ptr()).set_field(1, head);
        }
        head = Value::boxed(node);
    }
    task.registers[1] = head;
}

const GARBAGE_CHUNK: usize = 200;
const CHAIN_LEN: usize = 5_000;
const CHAIN_GARBAGE_CHUNK: usize = 150;
const SAMPLE_SIZE: usize = 20;
const MEASUREMENT_TIME: Duration = Duration::from_secs(4);

fn bench_nogc(c: &mut Criterion) {
    let value_type = small_value_type();
    let node_type_ptr = node_type();

    let gc = GCController::new(GCConfig::nogc());
    let (rt, mut task) = fresh_runtime();
    let mut group = c.benchmark_group("gc_nogc");
    group.sample_size(SAMPLE_SIZE);
    group.measurement_time(MEASUREMENT_TIME);
    group.bench_function("garbage_heavy", |b| {
        b.iter(|| garbage_chunk(&gc, &rt, &mut task, value_type, GARBAGE_CHUNK))
    });
    group.finish();

    let gc = GCController::new(GCConfig::nogc());
    let (rt, mut task) = fresh_runtime();
    build_chain(&gc, &mut task, node_type_ptr, CHAIN_LEN);
    let mut group = c.benchmark_group("gc_nogc");
    group.sample_size(SAMPLE_SIZE);
    group.measurement_time(MEASUREMENT_TIME);
    group.bench_function("chain_heavy", |b| {
        b.iter(|| chain_chunk(&gc, &rt, &mut task, node_type_ptr, CHAIN_GARBAGE_CHUNK))
    });
    group.finish();

    black_box(&gc);
}

fn bench_sticky(c: &mut Criterion) {
    let value_type = small_value_type();
    let node_type_ptr = node_type();

    let gc = GCController::new(GCConfig::mark_sweep().with_threshold(256 * 1024));
    let (rt, mut task) = fresh_runtime();
    let mut group = c.benchmark_group("gc_sticky");
    group.sample_size(SAMPLE_SIZE);
    group.measurement_time(MEASUREMENT_TIME);
    group.bench_function("garbage_heavy", |b| {
        b.iter(|| garbage_chunk(&gc, &rt, &mut task, value_type, GARBAGE_CHUNK))
    });
    group.finish();

    let gc = GCController::new(GCConfig::mark_sweep().with_threshold(256 * 1024));
    let (rt, mut task) = fresh_runtime();
    build_chain(&gc, &mut task, node_type_ptr, CHAIN_LEN);
    let mut group = c.benchmark_group("gc_sticky");
    group.sample_size(SAMPLE_SIZE);
    group.measurement_time(MEASUREMENT_TIME);
    group.bench_function("chain_heavy", |b| {
        b.iter(|| chain_chunk(&gc, &rt, &mut task, node_type_ptr, CHAIN_GARBAGE_CHUNK))
    });
    group.finish();

    black_box(&gc);
}

criterion_group!(gc_benches, bench_nogc, bench_sticky);
criterion_main!(gc_benches);
