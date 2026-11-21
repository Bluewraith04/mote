use std::collections::HashSet;

use mote_abi::Region;

use crate::fixture::{config, Env, NODE_NEXT, NODE_VALUE};
use crate::{Config, Object, BLOCK_SIZE};

const PER_BLOCK: usize = BLOCK_SIZE / 24;

/// Fills `blocks` blocks with nodes valued `0xAAAA` and returns every `every`th of them.
fn fill(env: &mut Env, blocks: usize, every: usize) -> Vec<Object> {
    let mut kept = Vec::new();
    for at in 0..blocks * PER_BLOCK {
        let node = env.node(0xAAAA, None);
        if at % every == 0 {
            kept.push(node);
        }
    }
    kept
}

#[test]
fn sweep_blocks_with_nothing_marked_return_to_free_when_swept() {
    let mut env = Env::new();
    fill(&mut env, 6, 1);
    assert_eq!(env.heap.census().used, 6);
    env.collect(&[]);
    assert_eq!((env.heap.census().unswept, env.heap.census().used), (6, 0));
    env.heap.sweep_all();
    let census = env.heap.census();
    assert_eq!((census.unswept, census.recyclable, census.used), (0, 0, 0));
    assert_eq!(env.heap.used_bytes(), 0);
    assert_eq!(env.heap.stats().swept_blocks, 6);
}

#[test]
fn sweep_is_lazy_and_a_partly_live_block_is_recycled_before_the_next_is_swept() {
    let mut env = Env::new();
    let kept = fill(&mut env, 4, 100);
    env.collect(&kept);
    assert_eq!(env.heap.census().unswept, 4);
    let fresh = env.alloc(env.fx.node, 0);
    let census = env.heap.census();
    assert_eq!((census.unswept, census.recyclable), (3, 1));
    assert_eq!(env.heap.stats().swept_blocks, 1);
    assert!(kept.iter().all(|node| node.address() != fresh.address()));
}

#[test]
fn sweep_a_reused_hole_is_zeroed() {
    let mut env = Env::new();
    let kept = fill(&mut env, 3, 100);
    env.collect(&kept);
    let reused: Vec<Object> = (0..2 * PER_BLOCK).map(|_| env.alloc(env.fx.node, 0)).collect();
    for node in &reused {
        assert_eq!((node.word(NODE_VALUE), node.word(NODE_NEXT)), (0, 0));
    }
    assert_eq!(env.heap.stats().blocks_handed, 3);
}

#[test]
fn sweep_live_objects_are_never_moved_or_overwritten() {
    let mut env = Env::new();
    let kept = fill(&mut env, 4, 37);
    for (at, node) in kept.iter().enumerate() {
        node.set_word(NODE_VALUE, at as u64);
    }
    let addresses: Vec<usize> = kept.iter().map(|node| node.address()).collect();
    for _ in 0..5 {
        env.collect(&kept);
        for _ in 0..3 * PER_BLOCK {
            let node = env.alloc(env.fx.node, 0);
            node.set_word(NODE_VALUE, 0xBBBB);
        }
    }
    assert_eq!(addresses, kept.iter().map(|node| node.address()).collect::<Vec<_>>());
    for (at, node) in kept.iter().enumerate() {
        assert_eq!(node.word(NODE_VALUE), at as u64);
    }
}

#[test]
fn sweep_new_objects_never_overlap_live_ones() {
    let mut env = Env::new();
    let kept = fill(&mut env, 3, 10);
    env.collect(&kept);
    let live: HashSet<usize> = kept.iter().map(|node| node.address()).collect();
    for _ in 0..3 * PER_BLOCK {
        let node = env.alloc(env.fx.node, 0);
        for held in &live {
            assert!(node.address() + 24 <= *held || *held + 24 <= node.address());
        }
    }
}

#[test]
fn sweep_a_hole_is_a_run_of_unmarked_lines_so_a_larger_object_skips_short_ones() {
    let mut env = Env::new();
    let kept = fill(&mut env, 1, 5);
    env.collect(&kept);
    let wide = env.alloc(env.fx.ints, 100);
    for node in &kept {
        assert!(node.address() + 24 <= wide.address() || wide.address() + 816 <= node.address());
    }
}

#[test]
fn sweep_empty_blocks_past_the_retain_count_are_given_back() {
    let mut env = Env::with(Config { retain: 2, ..config() });
    fill(&mut env, 8, 1);
    env.collect(&[]);
    env.heap.sweep_all();
    let stats = env.heap.stats();
    assert_eq!(stats.discarded_blocks, 6);
    assert_eq!(env.heap.census().free, crate::CHUNK_BLOCKS);
    let node = env.node(1, None);
    assert_eq!((node.word(NODE_VALUE), node.word(NODE_NEXT)), (1, 0));
    let again = env.alloc(env.fx.node, 0);
    assert_eq!(again.word(NODE_VALUE), 0);
}

#[test]
fn sweep_a_discarded_block_reads_zero_when_it_is_handed_out_again() {
    let mut env = Env::with(Config { retain: 0, ..config() });
    fill(&mut env, 3, 1);
    env.collect(&[]);
    env.heap.sweep_all();
    assert_eq!(env.heap.stats().discarded_blocks, 3);
    let nodes: Vec<Object> = (0..3 * PER_BLOCK).map(|_| env.alloc(env.fx.node, 0)).collect();
    assert!(nodes.iter().all(|node| node.word(NODE_VALUE) == 0));
}

#[test]
fn sweep_large_objects_are_given_back_by_the_collection_itself() {
    let mut env = Env::new();
    env.alloc(env.fx.ints, 5000);
    env.alloc(env.fx.ints, 9000);
    assert_eq!(env.heap.used_bytes(), 40960 + 73728);
    env.collect(&[]);
    assert_eq!((env.heap.used_bytes(), env.heap.census().large), (0, 0));
    assert_eq!(env.heap.stats().large_freed, 2);
}

#[test]
fn sweep_a_full_heap_has_room_again_after_a_collection() {
    let mut env = Env::with(Config { limit: 3 * BLOCK_SIZE, ..config() });
    let kept = fill(&mut env, 3, 1000);
    assert!(env.heap.alloc(&mut env.region, env.fx.node, 0).is_err());
    env.collect(&kept);
    for _ in 0..2 * PER_BLOCK {
        env.alloc(env.fx.node, 0);
    }
    assert!(env.heap.used_bytes() <= 3 * BLOCK_SIZE);
}

#[test]
fn sweep_abandons_a_region_so_the_swept_memory_has_one_owner() {
    let mut env = Env::new();
    env.node(1, None);
    let region = env.region;
    assert!(region.limit > region.cursor);
    env.collect(&[]);
    assert_eq!(env.region, Region { cursor: 0, limit: 0 });
}
