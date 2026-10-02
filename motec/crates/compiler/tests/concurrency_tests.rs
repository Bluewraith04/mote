//! `scope { }` and `spawn { }` end to end.

mod common;
use common::Compiler;

fn run(src: &str) -> i64 {
    Compiler::run(src)
        .unwrap_or_else(|e| panic!("program failed:\n{e}\n--- source ---\n{src}"))
        .as_int()
        .expect("program result was not an Int")
}

#[test]
fn scope_waits_for_every_spawned_task() {
    let src = r#"
fn main() -> Int {
    var a = 0
    var b = 0
    scope {
        let h1 = spawn {
            var s = 0
            var i = 0
            while i < 100 {
                s = s + i
                i = i + 1
            }
            return s
        }
        let h2 = spawn {
            return 7 * 6
        }
        a = h1.join().unwrap()
        b = h2.join().unwrap()
    }
    return a + b
}
"#;
    assert_eq!(run(src), 4950 + 42);
}

#[test]
fn spawned_tasks_interleave_under_the_budget() {
    let src = r#"
fn main() -> Int {
    var out = 0
    scope {
        let h1 = spawn {
            var x = 0
            var i = 0
            while i < 6000 {
                x = x + 1
                i = i + 1
            }
            return x
        }
        let h2 = spawn {
            var y = 0
            var i = 0
            while i < 6000 {
                y = y + 1
                i = i + 1
            }
            return y
        }
        out = h1.join().unwrap() + h2.join().unwrap()
    }
    return out
}
"#;
    assert_eq!(run(src), 12000);
}

#[test]
fn nested_scope_inside_a_spawned_task() {
    let src = r#"
fn main() -> Int {
    var out = 0
    scope {
        let outer = spawn {
            var inner_total = 0
            scope {
                let h1 = spawn { return 10 }
                let h2 = spawn { return 20 }
                inner_total = h1.join().unwrap() + h2.join().unwrap()
            }
            return inner_total + 1
        }
        out = outer.join().unwrap()
    }
    return out
}
"#;
    assert_eq!(run(src), 31);
}

#[test]
fn a_captured_scalar_is_snapshotted_by_value() {
    let src = r#"
fn main() -> Int {
    let n = 5
    var out = 0
    scope {
        let h = spawn { return n * n }
        out = h.join().unwrap()
    }
    return out
}
"#;
    assert_eq!(run(src), 25);
}

#[test]
fn spawn_outside_a_scope_is_detached_not_an_error() {
    let src = r#"
fn main() -> Int {
    let h = spawn { return 6 * 7 }
    return h.join().unwrap()
}
"#;
    assert_eq!(run(src), 42);
}

#[test]
fn spawn_expr_without_braces_is_sugar_for_a_block() {
    let src = r#"
fn compute() -> Int {
    return 6 * 7
}
fn main() -> Int {
    let h = spawn compute()
    return h.join().unwrap()
}
"#;
    assert_eq!(run(src), 42);
}

#[test]
fn an_indirect_spawn_from_a_scopeless_helper_stays_detached() {
    let src = r#"
fn helper() -> Task<__Any> {
    return spawn { return 99 }
}
fn main() -> Int {
    var out = 0
    scope {
        let h = helper()
        out = h.join().unwrap()
    }
    return out
}
"#;
    assert_eq!(run(src), 99);
}

#[test]
fn an_unobserved_detached_fault_aborts_at_shutdown() {
    let src = r#"
fn main() -> Int {
    spawn {
        let xs = [1, 2]
        xs.get(5)
    }
    let b = spawn { return 0 }
    b.join().unwrap()
    return 0
}
"#;
    let err = Compiler::run(src).unwrap_err();
    assert!(err.contains("out of bounds"), "got: {err}");
}

#[test]
fn an_observed_detached_fault_does_not_abort() {
    let src = r#"
fn main() -> Int {
    let h = spawn {
        let xs = [1, 2]
        xs.get(5)
    }
    if h.join().is_err() {
        return 5
    }
    return 0
}
"#;
    assert_eq!(run(src), 5);
}

#[test]
fn capturing_a_non_sendable_value_is_a_compile_error() {
    let err = Compiler::compile(
        "fn main() -> Int {\n\
         \x20   let xs = [1, 2, 3]\n\
         \x20   scope {\n\
         \x20       spawn { let n = xs.len() }\n\
         \x20   }\n\
         \x20   return 0\n\
         }\n",
        "x",
    )
    .unwrap_err();
    assert!(err.contains("not Sendable"), "got: {err}");
}

#[test]
fn a_string_capture_is_allowed() {
    let src = r#"
fn main() -> Int {
    let msg = "hello"
    var out = 0
    scope {
        let h = spawn { return msg.len() }
        out = h.join().unwrap()
    }
    return out
}
"#;
    assert_eq!(run(src), 5);
}

#[test]
fn join_returns_the_spawned_tasks_result() {
    let src = r#"
fn main() -> Int {
    var out = 0
    scope {
        let h = spawn {
            return 42
        }
        out = h.join().unwrap()
    }
    return out
}
"#;
    assert_eq!(run(src), 42);
}

#[test]
fn cancel_does_not_stop_completion_before_the_c7_unwind() {
    let src = r#"
fn main() -> Int {
    var out = 0
    scope {
        let h = spawn {
            return 9
        }
        h.cancel()
        out = h.join().unwrap()
    }
    return out
}
"#;
    assert_eq!(run(src), 9);
}

#[test]
fn a_joined_fault_is_observed_and_does_not_abort_the_scope() {
    let src = r#"
fn main() -> Int {
    var out = 0
    scope {
        let h = spawn {
            let xs = [1, 2]
            xs.get(5)
        }
        let r = h.join()
        if r.is_err() {
            out = 1
        }
    }
    return out
}
"#;
    assert_eq!(run(src), 1);
}

#[test]
fn an_unjoined_spawned_tasks_fault_aborts_the_scope() {
    let src = r#"
fn main() -> Int {
    scope {
        spawn {
            let xs = [1, 2]
            xs.get(5)
        }
    }
    return 0
}
"#;
    let err = Compiler::run(src).unwrap_err();
    assert!(err.contains("out of bounds"), "got: {err}");
}

#[test]
fn an_unobserved_fault_in_a_fallible_function_returns_err_instead_of_aborting() {
    let src = r#"
fn run_it() -> Result {
    scope {
        spawn {
            let xs = [1, 2]
            xs.get(5)
        }
    }
    return Ok(1)
}
fn main() -> Int {
    match run_it() {
        Ok(v) => { return v }
        Err(e) => { return 99 }
    }
}
"#;
    assert_eq!(run(src), 99);
}

#[test]
fn send_then_recv_round_trips_within_one_task() {
    let src = r#"
fn main() -> Int {
    let (tx, rx) = Channel<Int>(3)
    tx.send(10)
    tx.send(20)
    let a = rx.recv().unwrap()
    let b = rx.recv().unwrap()
    return a + b
}
"#;
    assert_eq!(run(src), 30);
}

#[test]
fn recv_on_an_empty_closed_channel_is_none() {
    let src = r#"
fn main() -> Int {
    let (tx, rx) = Channel<Int>(2)
    tx.send(1)
    let first = rx.recv()
    tx.close()
    let second = rx.recv()
    var out = 0
    if first.is_some() && second.is_none() {
        out = 1
    }
    return out
}
"#;
    assert_eq!(run(src), 1);
}

#[test]
fn send_on_a_full_channel_with_no_receiver_deadlocks() {
    let err = Compiler::run(
        "fn main() -> Int {\n    let (tx, rx) = Channel<Int>(1)\n    tx.send(1)\n    tx.send(2)\n    return 0\n}\n",
    )
    .unwrap_err();
    assert!(err.contains("deadlock"), "got: {err}");
}

#[test]
fn send_on_a_closed_channel_is_an_error() {
    let err = Compiler::run(
        "fn main() -> Int {\n    let (tx, rx) = Channel<Int>(1)\n    tx.close()\n    tx.send(1)\n    return 0\n}\n",
    )
    .unwrap_err();
    assert!(err.contains("closed"), "got: {err}");
}

#[test]
fn sharing_a_class_makes_it_capturable_into_spawn() {
    let src = r#"
class Box { var n: Int }

fn main() -> Int {
    let b = Box { n: 7 }
    let fb = b.into_shared()
    var out = 0
    scope {
        let h = spawn { return fb.get().n * 6 }
        out = h.join().unwrap()
    }
    return out
}
"#;
    assert_eq!(run(src), 42);
}

#[test]
fn using_a_value_after_into_shared_is_a_compile_error() {
    let err = Compiler::compile(
        "class Box { var n: Int }\n\
         fn main() -> Int {\n\
         \x20   let b = Box { n: 1 }\n\
         \x20   let fb = b.into_shared()\n\
         \x20   return b.n\n\
         }\n",
        "x",
    )
    .unwrap_err();
    assert!(err.contains("consumed by `into_shared`"), "got: {err}");
}

#[test]
fn reassigning_a_moved_binding_makes_it_usable_again() {
    let src = r#"
fn main() -> Int {
    var xs = [1, 2, 3]
    let frozen = xs.into_shared()
    xs = [4, 5]
    return xs.len()
}
"#;
    assert_eq!(run(src), 2);
}

#[test]
fn calling_a_mutating_method_on_a_shared_list_is_a_compile_error() {
    let err = Compiler::compile(
        "fn main() -> Int {\n\
         \x20   let xs = [1, 2, 3]\n\
         \x20   let frozen = xs.into_shared()\n\
         \x20   frozen.get().push(4)\n\
         \x20   return 0\n\
         }\n",
        "x",
    )
    .unwrap_err();
    assert!(err.contains("read-only"), "got: {err}");
}

#[test]
fn writing_a_field_through_an_any_erased_snapshot_is_a_runtime_fault() {
    let src = r#"
class Box { var n: Int }
fn main() -> Int {
    let fb = Box { n: 1 }.into_shared()
    var bag: List<__Any> = []
    bag.push(fb.get())
    var b: Box = bag[0]
    b.n = 5
    return fb.get().n
}
"#;
    let err = Compiler::run(src).unwrap_err();
    assert!(err.contains("read-only"), "got: {err}");
}

#[test]
fn the_concurrency_done_bar_program() {
    let src = r#"
fn phase1_stream_and_collect() -> Int {
    var total = 0
    scope {
        let (tx, rx) = Channel<Int>(2)
        let rx2 = rx.clone()
        spawn {
            var i = 1
            while i <= 6 {
                tx.send(i)
                i = i + 1
            }
            tx.close()
        }
        let w1 = spawn {
            var sum = 0
            var running = true
            while running {
                let j = rx.recv()
                if j.is_some() {
                    sum = sum + j.unwrap()
                } else {
                    running = false
                }
            }
            return sum
        }
        let w2 = spawn {
            var sum = 0
            var running = true
            while running {
                let j = rx2.recv()
                if j.is_some() {
                    sum = sum + j.unwrap()
                } else {
                    running = false
                }
            }
            return sum
        }
        total = w1.join().unwrap() + w2.join().unwrap()
    }
    return total
}

fn phase2_fault_propagates() -> Result {
    scope {
        spawn {
            let xs = [1, 2]
            xs.get(5)
        }
    }
    return Ok(0)
}

fn phase3_cancel_stuck_worker() -> Int {
    var out = 0
    scope {
        let (stuck_tx, stuck_rx) = Channel<Int>(1)
        let helper = spawn { return 1 }
        let stuck = spawn {
            stuck_rx.recv()
            return 0
        }
        helper.join()
        stuck.cancel()
        let r = stuck.join()
        if r.is_err() {
            out = 1
        }
    }
    return out
}

fn main() -> Int {
    let streamed = phase1_stream_and_collect()
    var fault_ok = 0
    match phase2_fault_propagates() {
        Ok(v) => { fault_ok = 0 }
        Err(e) => { fault_ok = 1 }
    }
    let cancelled_ok = phase3_cancel_stuck_worker()
    return streamed * 100 + fault_ok * 10 + cancelled_ok
}
"#;
    assert_eq!(run(src), 21 * 100 + 10 + 1);
}

#[test]
fn a_producer_task_streams_over_a_bounded_channel_to_the_main_consumer() {
    let src = r#"
fn main() -> Int {
    var total = 0
    scope {
        let (tx, rx) = Channel<Int>(2)
        spawn {
            tx.send(1)
            tx.send(2)
            tx.send(3)
            tx.close()
        }
        var running = true
        while running {
            let v = rx.recv()
            if v.is_some() {
                total = total + v.unwrap()
            } else {
                running = false
            }
        }
    }
    return total
}
"#;
    assert_eq!(run(src), 6);
}

const IMMUTABLE_CLASS: &str = "class Point {\n    x: Int\n    y: Int\n}\nclass Node {\n    id: Int\n    next: Node?\n}\n";

#[test]
fn an_immutable_class_is_sendable_without_a_wrapper() {
    let src = format!(
        "{IMMUTABLE_CLASS}fn main() -> Int {{\n    let p = Point {{ x: 3, y: 4 }}\n    var out = 0\n    scope {{\n        spawn {{ let q = p.x + p.y }}\n    }}\n    return 7\n}}\n"
    );
    assert_eq!(run(&src), 7);
}

#[test]
fn a_class_naming_itself_in_a_field_is_still_sendable() {
    let src = format!(
        "{IMMUTABLE_CLASS}fn main() -> Int {{\n    let n = Node {{ id: 5, next: null }}\n    scope {{\n        spawn {{ let k = n.id }}\n    }}\n    return 5\n}}\n"
    );
    assert_eq!(run(&src), 5);
}

#[test]
fn a_class_with_a_var_field_is_not_sendable() {
    let src = "class Counter {\n    var n: Int\n}\nfn main() -> Int {\n    let c = Counter { n: 0 }\n    scope {\n        spawn { let m = c.n }\n    }\n    return 0\n}\n";
    let err = Compiler::compile(src, "x").unwrap_err();
    assert!(err.contains("not Sendable"), "got: {err}");
}

#[test]
fn shared_copies_and_leaves_the_original_mutable() {
    let src = r#"
class Box { var n: Int }
fn main() -> Int {
    let b = Box { n: 7 }
    let s = Shared(b)
    b.n = 9
    var out = 0
    scope {
        let h = spawn { return s.get().n }
        out = h.join().unwrap()
    }
    return out * 10 + b.n
}
"#;
    assert_eq!(run(src), 79);
}

#[test]
fn shared_keeps_substructure_shared_and_survives_a_cycle() {
    let src = r#"
class Leaf { var v: Int }
class Node {
    var a: Leaf
    var b: Leaf
    var me: __Any
}
fn main() -> Int {
    let leaf = Leaf { v: 5 }
    let n = Node { a: leaf, b: leaf, me: null }
    n.me = n
    let s = Shared(n)
    return s.get().a.v + s.get().b.v
}
"#;
    assert_eq!(run(src), 10);
}

#[test]
fn shared_copies_a_list_of_lists_deeply() {
    let src = r#"
fn main() -> Int {
    let xs = [[1, 2], [3, 4]]
    let s = Shared(xs)
    xs.get(0).push(99)
    return s.get().get(0).len() * 10 + xs.get(0).len()
}
"#;
    assert_eq!(run(src), 23);
}

#[test]
fn a_shared_value_is_not_assignable_to_its_plain_type() {
    let src = "class Box { var n: Int }\nfn main() -> Int {\n    let b = Box { n: 1 }\n    let s = Shared(b)\n    let c: Box = s\n    return 0\n}\n";
    assert!(Compiler::compile(src, "x").is_err());
}

#[test]
fn a_channel_inside_a_shared_graph_is_the_same_channel() {
    let src = r#"
class Job { var tx: Sender<Int> }
fn main() -> Int {
    let (tx, rx) = Channel<Int>(4)
    let j = Job { tx: tx }
    let s = Shared(j)
    scope {
        spawn { s.get().tx.send(41) }
    }
    return rx.recv().unwrap() + 1
}
"#;
    assert_eq!(run(src), 42);
}

#[test]
fn a_cell_holds_a_string_or_a_list() {
    let src = r#"
fn main() -> Int {
    let s = Shared("hi")
    let xs = Shared([1, 2, 3])
    return s.get().len() + xs.get().len()
}
"#;
    assert_eq!(run(src), 5);
}

#[test]
fn several_tasks_read_one_shared_table() {
    let src = r#"
fn weigh(table: Shared<Map<String, Int>>, keys: List<String>) -> Int {
    var total = 0
    for k in keys {
        total = total + table.get().get(k)
    }
    return total
}

fn main() -> Int {
    var prices: Map<String, Int> = Map()
    prices.set("tea", 3)
    prices.set("jam", 5)
    prices.set("rye", 7)
    let table = Shared(prices)
    prices.set("tea", 100)
    var sum = 0
    scope {
        let a = spawn { return weigh(table, ["tea", "jam"]) }
        let b = spawn { return weigh(table, ["jam", "rye"]) }
        let c = spawn { return weigh(table, ["tea", "rye"]) }
        sum = a.join().unwrap() + b.join().unwrap() + c.join().unwrap()
    }
    return sum + prices.get("tea")
}
"#;
    assert_eq!(run(src), 8 + 12 + 10 + 100);
}

#[test]
fn nested_generic_closers_and_shift_operators_both_parse() {
    let src = r#"
fn count(m: Map<String, List<Int>>, deep: List<List<List<Int>>>) -> Int {
    return m.len() + deep.len()
}
fn main() -> Int {
    var m: Map<String, List<Int>> = Map()
    m.set("a", [1])
    return count(m, [[[1]]]) + (8 >> 1) + (256 >> 4 >> 2)
}
"#;
    assert_eq!(run(src), 2 + 4 + 4);
}
