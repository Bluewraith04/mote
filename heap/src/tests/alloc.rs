use mote_abi::{FaultKind, Header, FLAG_FINALIZE, FLAG_LARGE};

use crate::fixture::{config, Env, Held, NODE_NEXT, NODE_VALUE};
use crate::{Config, Full, OutOfMemory, BLOCK_SIZE, LARGE_THRESHOLD};

#[test]
fn alloc_returns_zeroed_memory_with_the_header_written() {
    let mut env = Env::new();
    let node = env.alloc(env.fx.node, 0);
    assert_eq!(node.header(), Header::new(env.fx.node, 0));
    assert_eq!((node.word(NODE_VALUE), node.word(NODE_NEXT)), (0, 0));
    assert_eq!(node.address() % 8, 0);
}

#[test]
fn alloc_objects_from_one_region_follow_each_other() {
    let mut env = Env::new();
    let first = env.alloc(env.fx.node, 0);
    let second = env.alloc(env.fx.node, 0);
    assert_eq!(second.address(), first.address() + 24);
    assert_eq!(env.region.cursor, second.address() as u64 + 24);
}

#[test]
fn alloc_a_string_and_an_array_keep_their_length_after_the_header() {
    let mut env = Env::new();
    let text = env.alloc(env.fx.string, 5);
    let ints = env.alloc(env.fx.ints, 3);
    assert_eq!((text.length(), ints.length()), (5, 3));
    assert_eq!(ints.address(), text.address() + 24);
    let after = env.alloc(env.fx.node, 0);
    assert_eq!(after.address(), ints.address() + 16 + 24);
    for offset in (16..40).step_by(8) {
        assert_eq!(ints.word(offset), 0);
    }
}

#[test]
fn alloc_an_empty_array_is_a_header_and_a_length() {
    let mut env = Env::new();
    let none = env.alloc(env.fx.ints, 0);
    assert_eq!(none.length(), 0);
    assert_eq!(env.alloc(env.fx.node, 0).address(), none.address() + 16);
}

#[test]
fn alloc_a_full_region_takes_a_new_hole() {
    let mut env = Env::new();
    let per_block = BLOCK_SIZE / 24;
    let first = env.alloc(env.fx.node, 0);
    for _ in 1..per_block {
        env.alloc(env.fx.node, 0);
    }
    assert_eq!(env.heap.stats().blocks_handed, 1);
    let next = env.alloc(env.fx.node, 0);
    assert_eq!(env.heap.stats().blocks_handed, 2);
    assert_eq!(env.heap.used_bytes(), 2 * BLOCK_SIZE);
    assert_ne!(next.address() / BLOCK_SIZE, first.address() / BLOCK_SIZE);
}

#[test]
fn alloc_objects_up_to_the_threshold_stay_in_a_block() {
    let mut env = Env::new();
    let words = (LARGE_THRESHOLD as u64 - 16) / 8;
    let small = env.alloc(env.fx.ints, words);
    assert!(!small.header().has(FLAG_LARGE));
    assert_eq!(env.heap.census().large, 0);
    assert_eq!(small.address() % BLOCK_SIZE, 0);
}

#[test]
fn alloc_an_object_past_the_threshold_gets_its_own_mapping_flagged_large() {
    let mut env = Env::new();
    let words = (LARGE_THRESHOLD as u64 - 16) / 8 + 1;
    let large = env.alloc(env.fx.ints, words);
    assert!(large.header().has(FLAG_LARGE));
    assert_eq!(large.length(), words);
    assert_eq!(large.address() % 4096, 0);
    assert_eq!(env.heap.census().large, 1);
    assert_eq!(env.heap.used_bytes(), 12288);
    assert_eq!(large.word(16 + 8 * (words as u32 - 1)), 0);
}

#[test]
fn alloc_the_limit_stops_blocks_and_large_objects() {
    let mut env = Env::with(Config { limit: 2 * BLOCK_SIZE, ..config() });
    let per_block = BLOCK_SIZE / 24;
    for _ in 0..2 * per_block {
        env.alloc(env.fx.node, 0);
    }
    assert_eq!(env.heap.alloc(&mut env.region, env.fx.node, 0), Err(Full));
    assert_eq!(env.heap.used_bytes(), 2 * BLOCK_SIZE);
    assert_eq!(env.heap.alloc(&mut env.region, env.fx.ints, 5000), Err(Full));
}

#[test]
fn alloc_a_large_object_counts_against_the_limit() {
    let mut env = Env::with(Config { limit: 40960 + BLOCK_SIZE, ..config() });
    env.alloc(env.fx.ints, 5000);
    assert_eq!(env.heap.used_bytes(), 40960);
    assert_eq!(env.heap.alloc(&mut env.region, env.fx.ints, 5000), Err(Full));
    env.alloc(env.fx.ints, 1);
    assert_eq!(env.heap.used_bytes(), 40960 + BLOCK_SIZE);
}

#[test]
fn alloc_a_count_too_large_to_hold_is_full_not_an_overflow() {
    let mut env = Env::new();
    assert_eq!(env.heap.alloc(&mut env.region, env.fx.ints, u64::MAX / 2), Err(Full));
    let mut held = Held::default();
    let found = env.heap.alloc_collecting(&mut env.region, &mut held, env.fx.string, u64::MAX / 4);
    assert_eq!(found, Err(OutOfMemory { limit: 256 << 20 }));
}

#[test]
#[should_panic(expected = "only a string or an array has elements")]
fn alloc_a_count_for_an_object_without_elements_is_a_bug() {
    let mut env = Env::new();
    env.alloc(env.fx.node, 2);
}

#[test]
fn alloc_collecting_reuses_the_memory_of_dead_objects() {
    let mut env = Env::with(Config { limit: 2 * BLOCK_SIZE, ..config() });
    let mut held = Held::default();
    for _ in 0..20_000 {
        env.heap.alloc_collecting(&mut env.region, &mut held, env.fx.node, 0).expect("garbage is collected");
    }
    let stats = env.heap.stats();
    assert!(stats.collections >= 5, "{stats}");
    assert!(stats.used_bytes <= 2 * BLOCK_SIZE);
}

#[test]
fn alloc_collecting_past_the_limit_with_everything_live_is_the_memory_limit_fault() {
    let mut env = Env::with(Config { limit: 2 * BLOCK_SIZE, ..config() });
    let mut held = Held::default();
    let outcome = loop {
        match env.heap.alloc_collecting(&mut env.region, &mut held, env.fx.node, 0) {
            Ok(node) => held.objects.push(node),
            Err(error) => break error,
        }
    };
    assert_eq!(outcome.fault(), FaultKind::MemoryLimit);
    assert_eq!(held.objects.len(), 2 * (BLOCK_SIZE / 24));
}

#[test]
fn alloc_a_trigger_asks_for_a_collection_after_that_many_bytes() {
    let mut env = Env::with(Config { trigger: 2 * BLOCK_SIZE, ..config() });
    assert!(!env.heap.wants_collection());
    for _ in 0..BLOCK_SIZE / 24 {
        env.alloc(env.fx.node, 0);
    }
    assert!(!env.heap.wants_collection());
    env.alloc(env.fx.node, 0);
    for _ in 0..BLOCK_SIZE / 24 {
        env.alloc(env.fx.node, 0);
    }
    assert!(env.heap.wants_collection());
    env.collect(&[]);
    assert!(!env.heap.wants_collection());
}

#[test]
fn alloc_a_finalizable_kind_carries_the_finalize_flag() {
    let mut env = Env::new();
    let port = env.alloc(env.fx.port, 0);
    assert!(port.header().has(FLAG_FINALIZE));
    assert!(!env.alloc(env.fx.node, 0).header().has(FLAG_FINALIZE));
}

#[test]
fn alloc_a_task_channel_mutex_or_once_carries_the_finalize_flag_and_an_end_does_not() {
    use mote_abi::{HandleKind, Kind, Region, TypeTable};

    let mut table = TypeTable::new();
    let kinds = [HandleKind::Task, HandleKind::Channel, HandleKind::Mutex, HandleKind::Once, HandleKind::Sender, HandleKind::Receiver];
    let int = table.add("Int", Kind::Scalar(mote_abi::Scalar::I64));
    let ids: Vec<_> = kinds.iter().enumerate().map(|(at, kind)| table.add(format!("{kind:?}"), Kind::Handle(*kind, if at < 4 { vec![int] } else { Vec::new() }))).collect();
    let heap = crate::Heap::new(std::sync::Arc::new(table), config());
    let mut region = Region { cursor: 0, limit: 0 };
    let flags: Vec<bool> = ids.iter().map(|id| heap.alloc(&mut region, *id, 0).expect("room in the heap").header().has(FLAG_FINALIZE)).collect();
    assert_eq!(flags, [true, true, true, true, false, false]);
}

#[test]
fn alloc_counts_every_allocation_in_the_stats() {
    let mut env = Env::new();
    for _ in 0..7 {
        env.alloc(env.fx.node, 0);
    }
    let stats = env.heap.stats();
    assert_eq!((stats.allocations, stats.holes + stats.blocks_handed), (7, 1));
    assert_eq!(env.region.limit - env.region.cursor, (BLOCK_SIZE - 7 * 24) as u64);
}
