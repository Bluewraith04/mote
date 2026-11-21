use std::sync::{Arc, Mutex};

use crate::fixture::Env;
use crate::Object;

type Log = Arc<Mutex<Vec<(usize, u64)>>>;

fn logging(env: &Env) -> Log {
    let log: Log = Arc::default();
    let sink = log.clone();
    env.heap.set_finalizer(Box::new(move |object: Object| sink.lock().unwrap().push((object.address(), object.word(8)))));
    log
}

#[test]
fn finalize_a_dead_port_runs_the_finalizer_once_with_its_words_intact() {
    let mut env = Env::new();
    let log = logging(&env);
    let port = env.alloc(env.fx.port, 0);
    port.set_word(8, 77);
    let found = env.collect(&[]);
    assert_eq!(found.finalized, 1);
    assert_eq!(*log.lock().unwrap(), [(port.address(), 77)]);
    env.collect(&[]);
    assert_eq!(log.lock().unwrap().len(), 1);
    assert_eq!(env.heap.stats().finalized, 1);
}

#[test]
fn finalize_a_live_port_is_kept_until_it_dies() {
    let mut env = Env::new();
    let log = logging(&env);
    let port = env.alloc(env.fx.port, 0);
    port.set_word(8, 5);
    assert_eq!(env.collect(&[port]).finalized, 0);
    assert_eq!(env.collect(&[port]).finalized, 0);
    assert!(log.lock().unwrap().is_empty());
    assert_eq!(env.collect(&[]).finalized, 1);
    assert_eq!(*log.lock().unwrap(), [(port.address(), 5)]);
}

#[test]
fn finalize_only_ports_are_finalizable() {
    let mut env = Env::new();
    let log = logging(&env);
    env.node(1, None);
    env.alloc(env.fx.string, 3);
    env.alloc(env.fx.ints, 5000);
    assert_eq!(env.collect(&[]).finalized, 0);
    assert!(log.lock().unwrap().is_empty());
}

#[test]
fn finalize_each_dying_port_is_finalized_and_the_rest_survive() {
    let mut env = Env::new();
    let log = logging(&env);
    let ports: Vec<Object> = (0..10)
        .map(|at| {
            let port = env.alloc(env.fx.port, 0);
            port.set_word(8, at);
            port
        })
        .collect();
    let kept: Vec<Object> = ports.iter().copied().filter(|port| port.word(8) % 3 == 0).collect();
    assert_eq!(env.collect(&kept).finalized, 6);
    let mut values: Vec<u64> = log.lock().unwrap().iter().map(|(_, value)| *value).collect();
    values.sort();
    assert_eq!(values, [1, 2, 4, 5, 7, 8]);
}

#[test]
fn finalize_without_a_finalizer_still_forgets_the_dead_port() {
    let mut env = Env::new();
    env.alloc(env.fx.port, 0);
    assert_eq!(env.collect(&[]).finalized, 1);
    assert_eq!(env.collect(&[]).finalized, 0);
}

#[test]
fn finalize_runs_before_the_memory_is_reused() {
    let mut env = Env::new();
    let log = logging(&env);
    let port = env.alloc(env.fx.port, 0);
    port.set_word(8, 9);
    env.collect(&[]);
    assert_eq!(log.lock().unwrap().len(), 1);
    let reused = env.alloc(env.fx.port, 0);
    assert_eq!(reused.word(8), 0);
}
