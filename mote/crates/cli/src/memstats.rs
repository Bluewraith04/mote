//! `--mem-stats`: what a run used, one `mem.key = value` line each.

use runtime::Runtime;

/// `n` bytes as `512 B`, `64.0 KiB`, `4.0 GiB`.
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut value = n as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

fn bytes_line(key: &str, n: u64) -> String {
    format!("mem.{key} = {n} ({})", human_bytes(n))
}

fn ms_line(key: &str, ns: u64) -> String {
    format!("mem.{key} = {:.3}", ns as f64 / 1e6)
}

/// The lines `--mem-stats` prints for a finished run; `peak_rss` is the process's peak resident memory, if known.
pub fn lines(rt: &Runtime, peak_rss: Option<u64>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(rss) = peak_rss {
        out.push(bytes_line("peak_rss", rss));
    }
    if let Some(heap) = rt.gc_stats() {
        out.push(bytes_line("heap.peak_live", heap.peak_live_bytes as u64));
        out.push(bytes_line("heap.peak_in_use", heap.peak_in_use_bytes as u64));
        out.push(bytes_line("heap.live_at_exit", heap.live_bytes as u64));
        out.push(format!("mem.heap.chunks = {} ({})", heap.chunks, human_bytes((heap.chunks * gc::chunk::CHUNK_BYTES) as u64)));
        out.push(format!("mem.heap.free_blocks = {} ({})", heap.free_blocks, human_bytes((heap.free_blocks * gc::chunk::CHUNK_BYTES) as u64)));
        out.push(format!("mem.heap.slabs = {} ({})", heap.slabs, human_bytes((heap.slabs * gc::chunk::SLAB_BYTES) as u64)));
        out.push(format!("mem.heap.peak_slabs = {} ({})", heap.peak_slabs, human_bytes((heap.peak_slabs * gc::chunk::SLAB_BYTES) as u64)));
        out.push(format!("mem.heap.large_allocations = {}", heap.large_allocations));
        out.push(bytes_line("heap.large_bytes", heap.large_bytes as u64));
        out.push(format!("mem.gc.collections = {}", heap.collections));
        out.push(ms_line("gc.pause_total_ms", heap.pause_total_ns));
        out.push(ms_line("gc.pause_max_ms", heap.pause_max_ns));
        let histogram = rt.size_histogram();
        let last = histogram.len().saturating_sub(1);
        for (units, &count) in histogram.iter().enumerate().filter(|(_, c)| **c > 0) {
            if units == last {
                out.push(format!("mem.heap.alloc.large = {count}"));
            } else {
                out.push(format!("mem.heap.alloc.units_{units} = {count}"));
            }
        }
    }
    let tasks = rt.task_stats();
    out.push(format!("mem.tasks.spawned = {}", tasks.spawned));
    out.push(format!("mem.tasks.peak_live = {}", tasks.peak_live));
    out.push(format!("mem.tasks.peak_register_slots = {}", tasks.peak_register_slots));
    let regions = &tasks.regions;
    out.push(format!("mem.regions.scopes = {}", regions.scopes_entered));
    out.push(format!("mem.regions.objects = {}", regions.objects));
    out.push(format!("mem.regions.bytes = {}", regions.bytes));
    out.push(format!("mem.regions.peak_fill = {}", regions.peak_fill));
    out.push(format!("mem.regions.peak_depth = {}", regions.peak_depth));
    out.push(bytes_line("regions.peak_segment_bytes", regions.peak_segment_bytes as u64));
    out.push(format!("mem.regions.too_deep = {}", regions.too_deep));
    for (i, &count) in regions.fill_histogram.iter().enumerate().filter(|(_, c)| **c > 0) {
        match runtime::arena::FILL_BUCKETS.get(i) {
            Some(bound) => out.push(format!("mem.regions.fill_le_{bound} = {count}")),
            None => out.push(format!("mem.regions.fill_over_{} = {count}", runtime::arena::FILL_BUCKETS[i - 1])),
        }
    }
    for (i, &count) in regions.object_histogram.iter().enumerate().filter(|(_, c)| **c > 0) {
        match runtime::arena::OBJECT_BUCKETS.get(i) {
            Some(bound) => out.push(format!("mem.regions.object_slots_le_{bound} = {count}")),
            None => out.push(format!("mem.regions.object_slots_over_{} = {count}", runtime::arena::OBJECT_BUCKETS[i - 1])),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::human_bytes;

    #[test]
    fn sizes_read_in_binary_units() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(64 * 1024), "64.0 KiB");
        assert_eq!(human_bytes(3 << 30), "3.0 GiB");
    }
}
