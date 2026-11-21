use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

use mote_abi::Region;

use crate::fixture::{address, config, Env, Held, Fixture, NODE_NEXT, NODE_VALUE};
use crate::{Config, Heap, Marker, Object, Roots, World, BLOCK_SIZE};

#[test]
fn parallel_marking_finds_what_one_worker_finds() {
    let mut env = Env::new();
    let spine = env.alloc(env.fx.nodes, 500);
    for at in 0..500u32 {
        let mut head = None;
        for depth in 0..60 {
            head = Some(env.node(u64::from(at) * 100 + depth, head));
        }
        spine.set_word(16 + 8 * at, address(head));
    }
    for _ in 0..1000 {
        env.node(0, None);
    }
    let one = env.collect(&[spine]);
    assert_eq!(one.marked_objects, 1 + 500 * 60);
    for workers in [2, 3, 8] {
        let many = env.collect_held(&mut Held { objects: vec![spine], frame: None }, workers);
        assert_eq!((many.marked_objects, many.marked_bytes), (one.marked_objects, one.marked_bytes), "{workers} workers");
    }
}

#[test]
fn parallel_marking_a_single_long_chain_terminates_with_idle_workers() {
    let mut env = Env::new();
    let mut head = None;
    for value in 0..50_000 {
        head = Some(env.node(value, head));
    }
    let found = env.collect_held(&mut Held { objects: vec![head.unwrap()], frame: None }, 4);
    assert_eq!(found.marked_objects, 50_000);
}

#[test]
fn parallel_marking_a_wide_object_is_shared_between_workers() {
    let mut env = Env::new();
    let wide = env.alloc(env.fx.nodes, 5000);
    for at in 0..5000u32 {
        let node = env.node(u64::from(at), None);
        wide.set_word(16 + 8 * at, address(Some(node)));
    }
    let found = env.collect_held(&mut Held { objects: vec![wide], frame: None }, 4);
    assert_eq!(found.marked_objects, 5001);
}

#[test]
fn parallel_allocation_from_many_threads_gives_distinct_zeroed_objects() {
    let fx = Fixture::new();
    let heap = Heap::new(fx.table.clone(), Config { limit: 1 << 30, ..config() });
    let all: Vec<Vec<Object>> = std::thread::scope(|scope| {
        let threads: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    let mut region = Region { cursor: 0, limit: 0 };
                    let mut mine = Vec::new();
                    for at in 0..5000 {
                        let node = heap.alloc(&mut region, fx.node, 0).unwrap();
                        assert_eq!((node.word(NODE_VALUE), node.word(NODE_NEXT)), (0, 0));
                        node.set_word(NODE_VALUE, at);
                        mine.push(node);
                    }
                    mine
                })
            })
            .collect();
        threads.into_iter().map(|thread| thread.join().unwrap()).collect()
    });
    let addresses: HashSet<usize> = all.iter().flatten().map(|node| node.address()).collect();
    assert_eq!(addresses.len(), 8 * 5000);
    for mine in &all {
        assert!(mine.iter().enumerate().all(|(at, node)| node.word(NODE_VALUE) == at as u64));
    }
}

#[test]
fn parallel_a_stop_waits_until_every_mutator_is_at_a_safepoint() {
    let world = World::new();
    let counter = AtomicUsize::new(0);
    let running = AtomicBool::new(true);
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                let mutator = world.register();
                while running.load(Ordering::Relaxed) {
                    mutator.poll();
                    counter.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
        while world.mutators() < 4 {
            std::thread::yield_now();
        }
        for _ in 0..20 {
            world.stop(|| {
                let before = counter.load(Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(2));
                assert_eq!(counter.load(Ordering::Relaxed), before, "a mutator ran during the stop");
            });
        }
        running.store(false, Ordering::Relaxed);
    });
    assert!(counter.load(Ordering::Relaxed) > 0);
}

#[test]
fn parallel_a_stop_requesting_runs_its_request_while_a_stop_is_asked_for() {
    let world = World::new();
    let asked = AtomicBool::new(false);
    let polled = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let (world, asked, polled) = (&world, &asked, &polled);
        scope.spawn(move || {
            let mutator = world.register();
            while !asked.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            polled.store(true, Ordering::Release);
            mutator.poll();
        });
        while world.mutators() < 1 {
            std::thread::yield_now();
        }
        world.stop_requesting(
            || {
                assert!(world.stop_requested());
                asked.store(true, Ordering::Release);
            },
            || assert!(polled.load(Ordering::Acquire)),
        );
    });
}

#[test]
fn parallel_a_blocked_mutator_does_not_hold_up_a_stop() {
    let world = World::new();
    let (send, receive) = mpsc::channel::<u32>();
    let stopped = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let (world, stopped) = (&world, &stopped);
        scope.spawn(move || {
            let mutator = world.register();
            let value = mutator.blocking(|| receive.recv().unwrap());
            assert_eq!(value, 5);
            assert!(stopped.load(Ordering::SeqCst));
        });
        while world.mutators() < 1 {
            std::thread::yield_now();
        }
        world.stop(|| stopped.store(true, Ordering::SeqCst));
        send.send(5).unwrap();
    });
}

#[test]
fn parallel_a_mutator_can_start_the_stop_and_two_stops_never_overlap() {
    let world = World::new();
    let inside = AtomicUsize::new(0);
    let stops = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                let mutator = world.register();
                for _ in 0..50 {
                    mutator.poll();
                    mutator.stop(|| {
                        assert_eq!(inside.fetch_add(1, Ordering::SeqCst), 0);
                        stops.fetch_add(1, Ordering::SeqCst);
                        std::thread::yield_now();
                        inside.fetch_sub(1, Ordering::SeqCst);
                    });
                }
            });
        }
    });
    assert_eq!(stops.load(Ordering::SeqCst), 200);
}

#[test]
fn parallel_a_thread_registering_during_a_stop_waits_for_it_to_end() {
    let world = World::new();
    let registered = AtomicBool::new(false);
    std::thread::scope(|scope| {
        world.stop(|| {
            scope.spawn(|| {
                let _mutator = world.register();
                registered.store(true, Ordering::SeqCst);
            });
            std::thread::sleep(Duration::from_millis(20));
            assert!(!registered.load(Ordering::SeqCst));
        });
    });
    assert!(registered.load(Ordering::SeqCst));
}

struct Shared {
    objects: Vec<Mutex<Vec<Object>>>,
    regions: Vec<Mutex<Region>>,
}

impl Roots for &Shared {
    fn visit(&mut self, marker: &mut Marker<'_>) {
        for objects in &self.objects {
            for object in objects.lock().unwrap().iter() {
                marker.mark(Some(*object));
            }
        }
    }

    fn retire(&mut self) {
        for region in &self.regions {
            *region.lock().unwrap() = Region { cursor: 0, limit: 0 };
        }
    }
}

#[test]
fn parallel_mutators_allocate_while_a_stopped_world_collects_in_parallel() {
    let fx = Fixture::new();
    let heap = Heap::new(fx.table.clone(), Config { limit: 12 * BLOCK_SIZE, trigger: 2 * BLOCK_SIZE, verify: true, ..config() });
    let world = World::new();
    let threads = 4;
    let shared = Shared {
        objects: (0..threads).map(|_| Mutex::new(Vec::new())).collect(),
        regions: (0..threads).map(|_| Mutex::new(Region { cursor: 0, limit: 0 })).collect(),
    };
    std::thread::scope(|scope| {
        for id in 0..threads {
            let (heap, world, shared, fx) = (&heap, &world, &shared, &fx);
            scope.spawn(move || {
                let mutator = world.register();
                for seq in 0..4000u64 {
                    mutator.poll();
                    let made = loop {
                        let attempt = heap.alloc(&mut shared.regions[id].lock().unwrap(), fx.node, 0);
                        match attempt {
                            Ok(node) if !heap.wants_collection() => break node,
                            Ok(node) => {
                                shared.objects[id].lock().unwrap().push(node);
                                mutator.stop(|| heap.collect_parallel(&mut { shared }, 3));
                                shared.objects[id].lock().unwrap().pop();
                                break node;
                            }
                            Err(_) => {
                                mutator.stop(|| heap.collect_parallel(&mut { shared }, 3));
                            }
                        }
                    };
                    made.set_word(NODE_VALUE, (id as u64) << 32 | seq);
                    let mut mine = shared.objects[id].lock().unwrap();
                    if let (Some(last), true) = (mine.last(), seq % 50 != 0) {
                        made.set_word(NODE_NEXT, address(Some(*last)));
                    }
                    mine.push(made);
                    if mine.len() > 50 {
                        mine.remove(0);
                    }
                }
            });
        }
    });
    let stats = heap.stats();
    assert!(stats.collections >= 3, "{stats}");
    assert_eq!(stats.problems, 0, "{stats}");
    for (id, objects) in shared.objects.iter().enumerate() {
        let objects = objects.lock().unwrap();
        assert_eq!(objects.len(), 50);
        for (at, node) in objects.iter().enumerate() {
            assert_eq!(node.word(NODE_VALUE), (id as u64) << 32 | (3950 + at as u64));
        }
    }
}
