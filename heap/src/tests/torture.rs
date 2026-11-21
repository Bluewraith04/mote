use crate::fixture::{address, config, Env, Held, NODE_NEXT, NODE_VALUE};
use crate::{Config, BLOCK_SIZE};

/// A small deterministic generator.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, below: usize) -> usize {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) % below as u64) as usize
    }
}

fn torture() -> Config {
    Config { torture: true, verify: true, limit: 64 * BLOCK_SIZE, ..config() }
}

#[test]
fn torture_collects_at_every_allocation_and_keeps_a_rooted_list() {
    let mut env = Env::with(torture());
    let mut held = Held::default();
    let mut head = None;
    for value in 0..200u64 {
        let node = env.heap.alloc_collecting(&mut env.region, &mut held, env.fx.node, 0).unwrap();
        node.set_word(NODE_VALUE, value);
        node.set_word(NODE_NEXT, address(head));
        head = Some(node);
        held.objects = vec![node];
    }
    let stats = env.heap.stats();
    assert_eq!((stats.collections, stats.problems), (200, 0));
    let mut seen = Vec::new();
    let mut at = head;
    while let Some(node) = at {
        seen.push(node.word(NODE_VALUE));
        at = crate::Object::from_address(node.word(NODE_NEXT) as usize);
    }
    assert_eq!(seen, (0..200).rev().collect::<Vec<_>>());
}

#[test]
fn torture_a_random_graph_of_every_shape_stays_sound() {
    let mut env = Env::with(torture());
    let mut held = Held::default();
    let mut ids: Vec<u64> = Vec::new();
    let mut random = Lcg(7);
    let mut next_id = 1u64;
    for _ in 0..600 {
        let node = match random.next(10) {
            0 => {
                let array = env.heap.alloc_collecting(&mut env.region, &mut held, env.fx.nodes, 1 + random.next(40) as u64).unwrap();
                for at in 0..array.length() as u32 {
                    if !held.objects.is_empty() && random.next(2) == 0 {
                        array.set_word(16 + 8 * at, address(Some(held.objects[random.next(held.objects.len())])));
                    }
                }
                held.objects.push(array);
                ids.push(0);
                continue;
            }
            1 => {
                let big = env.heap.alloc_collecting(&mut env.region, &mut held, env.fx.nodes, 1100).unwrap();
                if !held.objects.is_empty() {
                    big.set_word(16, address(Some(held.objects[random.next(held.objects.len())])));
                }
                held.objects.push(big);
                ids.push(0);
                continue;
            }
            _ => env.heap.alloc_collecting(&mut env.region, &mut held, env.fx.node, 0).unwrap(),
        };
        node.set_word(NODE_VALUE, next_id);
        if !held.objects.is_empty() && random.next(3) != 0 {
            let target = held.objects[random.next(held.objects.len())];
            if target.header().type_id() == env.fx.node {
                node.set_word(NODE_NEXT, address(Some(target)));
            }
        }
        held.objects.push(node);
        ids.push(next_id);
        next_id += 1;
        if held.objects.len() > 30 {
            let drop = random.next(held.objects.len());
            held.objects.swap_remove(drop);
            ids.swap_remove(drop);
        }
    }
    for (object, id) in held.objects.iter().zip(&ids) {
        if *id != 0 {
            assert_eq!(object.word(NODE_VALUE), *id);
        }
    }
    let stats = env.heap.stats();
    assert_eq!(stats.problems, 0, "{stats}");
    assert!(stats.collections >= 600);
}

#[test]
fn torture_the_verifier_passes_after_a_sound_collection() {
    let mut env = Env::new();
    let a = env.node(1, None);
    let b = env.node(2, Some(a));
    let big = env.alloc(env.fx.nodes, 1100);
    big.set_word(16, address(Some(b)));
    env.collect(&[b, big]);
    let mut held = Held { objects: vec![b, big], frame: None };
    assert_eq!(env.heap.verify(&mut held), Vec::<String>::new());
}

#[test]
fn torture_the_verifier_reports_a_reachable_object_without_a_mark() {
    let mut env = Env::new();
    let a = env.node(1, None);
    env.collect(&[a]);
    env.heap.corrupt_marks(a, true);
    let mut held = Held { objects: vec![a], frame: None };
    let problems = env.heap.verify(&mut held);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("reachable but not marked"), "{problems:?}");
}

#[test]
fn torture_the_verifier_reports_an_object_on_a_line_that_is_not_marked() {
    let mut env = Env::new();
    let a = env.node(1, None);
    env.collect(&[a]);
    env.heap.corrupt_marks(a, false);
    let mut held = Held { objects: vec![a], frame: None };
    let problems = env.heap.verify(&mut held);
    assert!(problems.iter().any(|problem| problem.contains("line that is not marked")), "{problems:?}");
}

#[test]
fn torture_the_verifier_reports_references_that_dangle() {
    let mut env = Env::new();
    let a = env.node(1, None);
    env.collect(&[a]);
    let mut held = Held::default();
    for (target, expect) in [(a.address() + 4, "word-aligned"), (16, "outside the heap"), (a.address() + 8, "names no object type")] {
        a.set_word(NODE_NEXT, target as u64);
        held.objects = vec![a];
        let problems = env.heap.verify(&mut held);
        assert!(problems.iter().any(|problem| problem.contains(expect)), "{expect}: {problems:?}");
    }
    a.set_word(NODE_NEXT, 0);
}

#[test]
fn torture_the_verifier_reports_a_reference_into_a_free_block() {
    let mut env = Env::new();
    let per_block = BLOCK_SIZE / 24;
    let old: Vec<_> = (0..3 * per_block).map(|value| env.node(value as u64, None)).collect();
    env.collect(&[]);
    env.heap.sweep_all();
    let a = env.node(1, None);
    env.collect(&[a]);
    let free = old.iter().find(|node| node.address() / BLOCK_SIZE != a.address() / BLOCK_SIZE).copied().unwrap();
    a.set_word(NODE_NEXT, address(Some(free)));
    let mut held = Held { objects: vec![a], frame: None };
    let problems = env.heap.verify(&mut held);
    assert!(problems.iter().any(|problem| problem.contains("free block")), "{problems:?}");
    a.set_word(NODE_NEXT, 0);
}

#[test]
fn torture_stats_report_what_the_heap_did() {
    let mut env = Env::new();
    env.node(1, None);
    env.alloc(env.fx.ints, 5000);
    env.collect(&[]);
    let report = env.heap.stats().to_string();
    for name in ["allocations", "collections", "large freed", "marked objects", "peak used bytes", "pool bytes", "problems"] {
        assert!(report.contains(name), "{name}: {report}");
    }
    let line = report.lines().find(|line| line.starts_with("collections")).unwrap();
    assert_eq!(line.split_whitespace().last(), Some("1"));
    assert_eq!(env.heap.stats().allocations, 2);
}

#[test]
fn torture_a_full_collection_per_allocation_reuses_blocks_without_growing() {
    let mut env = Env::with(torture());
    let mut held = Held::default();
    for _ in 0..3000 {
        env.heap.alloc_collecting(&mut env.region, &mut held, env.fx.node, 0).unwrap();
    }
    assert!(env.heap.stats().peak_used <= 2 * BLOCK_SIZE);
}
