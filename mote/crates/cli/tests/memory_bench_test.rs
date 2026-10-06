//! Memory benchmark set. Run: `cargo test --release -p cli --test memory_bench_test -- --ignored --nocapture`.

use std::process::Command;
use std::time::Instant;

const KEYS: [&str; 8] = ["mem.peak_rss", "mem.heap.peak_live", "mem.heap.peak_in_use", "mem.gc.collections", "mem.gc.pause_total_ms", "mem.gc.pause_max_ms", "mem.tasks.peak_live", "mem.tasks.peak_register_slots"];

fn bench(name: &str, n: u64, extra: &[&str]) {
    let path = format!("{}/tests/mem_bench/{name}.mote", env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(&path).unwrap().replace("{N}", &n.to_string());
    let dir = std::env::temp_dir().join(format!("mote_bench_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.mote");
    std::fs::write(&file, source).unwrap();
    let start = Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(&file).arg("--mem-stats").args(extra).output().unwrap();
    let secs = start.elapsed().as_secs_f64();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    println!("== {name} N={n} {extra:?}: {secs:.2}s, ok={}", out.status.success());
    for line in text.lines().filter(|l| KEYS.iter().any(|k| l.starts_with(&format!("{k} ")))) {
        println!("   {line}");
    }
    if !out.status.success() {
        println!("{text}");
    }
}

#[test]
#[ignore]
fn spawn_parked_tasks() {
    bench("spawn_parked", 100_000, &[]);
}

#[test]
#[ignore]
fn deep_recursion() {
    bench("deep_recursion", 500_000, &[]);
}

#[test]
#[ignore]
fn grow_list() {
    bench("grow_list", 16_000_000, &[]);
}

#[test]
#[ignore]
fn size_class_phase_change() {
    bench("phase_change", 2_000_000, &[]);
}

#[test]
#[ignore]
fn connection_per_task() {
    bench("connection_per_task", 200_000, &[]);
}

#[test]
#[ignore]
fn pause_against_live_size() {
    for n in [1_000_000, 4_000_000, 8_000_000] {
        bench("pause_vs_live", n, &[]);
    }
}

#[test]
#[ignore]
fn struct_calls() {
    bench("struct_calls", 5_000_000, &[]);
}

#[test]
#[ignore]
fn lambda_churn() {
    bench("lambda_churn", 3_000_000, &[]);
}
