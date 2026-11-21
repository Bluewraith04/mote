# Concurrency

* `Channel` comes from `std.sync`; `Sender`, `Receiver`, `Mutex`, `Once`, `Task`, `AtomicInt`, and `AtomicBool` are universal builtins. <!-- block -->

```mote
import Channel from std.sync

fn types(a: AtomicInt, b: AtomicBool, tx: Sender<Int>, rx: Receiver<Int>, m: Mutex<Int>, o: Once<Int>, t: Task<Int>) {}
```

## Tasks

* `spawn { … }` starts a task. It is an expression of type `Task<T>`, where `T` is what the block returns, and `spawn f(args)` starts a task that calls `f`. <!-- block --> <!-- test: spawn_makes_a_task_of_what_its_body_returns -->
* `h.await()` waits and gives `Ok(value)`, or `Err(error)` when the task faulted; `?` unwraps it in a function that returns a `Result`. <!-- block -->

```mote
fn twice(n: Int) -> Int {
    return n * 2
}

fn total(h: Task<Int>) -> Result<Int, Error> {
    let m = h.await()?
    return Ok(m + 1)
}

fn main() {
    let h = spawn { return 6 * 7 }
    println(h.await().unwrap())
    let g = spawn twice(8)
    println(g.await().unwrap())
    println(total(spawn { return 1 }).unwrap())
    let zero = 0
    let bad = spawn { return 1 / zero }
    match bad.await() {
        Ok(v) => println(v)
        Err(e) => println(e.message)
    }
}
```

```output
42
16
2
division by zero
```

<!-- test: differential_tasks_the_compiled_code_shows_the_recorded_results, differential_tasks_the_compiled_code_shows_the_recorded_results -->
| Call on `Task<T>` | Result |
|---|---|
| `h.await()` | waits: `Ok(value)`, or `Err(error)` if the task faulted |
| `h.await_timeout(ms)` | the same, or `None` when `ms` pass first |
| `h.cancel()` | asks the task to stop at its next wait; never waits |
| `h.is_ready()` | `true` once the task has finished by returning, faulting or being cancelled; never waits |

* A cancelled task ends at its next wait; its `await` gives an `Err`, and that is not a fault. `await_timeout` returns `None` and leaves the task running. <!-- block --> <!-- test: differential_timed_the_compiled_code_shows_the_recorded_results, differential_cancel_the_compiled_code_shows_the_recorded_results -->

```mote
import sleep from std.task

fn main() {
    let slow = spawn {
        sleep(500)
        return 1
    }
    println(slow.await_timeout(100))
    println(slow.is_ready())
    slow.cancel()
    match slow.await() {
        Ok(v) => println("finished")
        Err(e) => println("cancelled")
    }
    println(slow.is_ready())
}
```

```output
null
false
cancelled
true
```

* A cancelled task undoes the `with` and `scope` blocks it is inside, innermost first: it calls each `close()`, and for each `scope` it cancels the tasks spawned in it and waits for them. A task that faults does not. <!-- test: differential_cancel_a_cancelled_task_closes_its_resources_innermost_first_and_a_fault_closes_none, differential_cancel_a_cancelled_task_closes_its_resources_innermost_first_and_a_fault_closes_none, differential_cancel_a_cancelled_task_cancels_and_waits_for_the_tasks_of_its_scope, differential_cancel_the_compiled_code_shows_the_recorded_results, differential_cancel_a_cancelled_task_closes_its_resources_innermost_first_and_a_fault_closes_none -->
* A task that loops without waiting gives way to the others after a time slice. <!-- test: preempt_the_ticker_sets_the_poll_flag_of_a_task_that_runs_a_whole_slice -->
* `sleep(ms)` moves the virtual clock, not the wall clock, and tasks that sleep wake in time order. <!-- test: differential_timed_the_compiled_code_shows_the_recorded_results, differential_timed_the_compiled_code_shows_the_recorded_results -->

## Scopes

* `scope { … }` waits at its closing brace until every task spawned inside it has finished. <!-- block -->

```mote
fn main() {
    var out = 0
    scope {
        let h = spawn { return 20 + 1 }
        out = h.await().unwrap() * 2
    }
    println(out)
}
```

```output
42
```

* A `return`, `break` or `continue` out of a scope waits for its tasks first. <!-- test: a_return_out_of_a_scope_waits_first, break_and_continue_out_of_a_scope_wait_first, a_scope_waits_for_its_tasks_at_its_end -->
* A `spawn` outside a scope is detached: it may outlive its function and is dropped when `main` returns. <!-- test: differential_tasks_the_compiled_code_shows_the_recorded_results -->
* Membership is by where the `spawn` is written: a `spawn` inside a lambda is not in the scope the lambda is written in. <!-- test: a_spawn_in_a_scope_is_scoped_but_not_through_a_lambda -->
* `await_all(tasks)` waits for a list of tasks and gives each `Result` in order. A `scope` waits for tasks it did not name; `await_all` waits for the ones it holds. <!-- test: differential_tasks_the_compiled_code_shows_the_recorded_results -->

## Faults in a task

<!-- test: differential_tasks_the_compiled_code_shows_the_recorded_results, differential_tasks_the_compiled_code_shows_the_recorded_results, differential_tasks_the_compiled_code_shows_the_recorded_results, differential_cancel_the_compiled_code_shows_the_recorded_results, differential_cancel_a_scope_with_a_faulted_task_gives_an_error_to_a_function_returning_a_result, differential_cancel_a_scope_with_a_faulted_task_gives_an_error_to_a_function_returning_a_result, differential_cancel_the_compiled_code_shows_the_recorded_results, differential_cancel_a_scope_with_a_faulted_task_gives_an_error_to_a_function_returning_a_result -->
| Situation | Outcome |
|---|---|
| the parent awaits and reads the `Result` | the parent handles it |
| nobody awaits, inside a scope, in a function returning `Result` | the scope's closing brace returns `Err` |
| nobody awaits, anywhere else | the program stops at the closing brace |
| a task in a scope faults | its siblings are cancelled |
| a task is cancelled | not a fault |

* The `Err` of a scope closes the `with` resources outside it, and an inner scope's `Err` waits for the outer scope's tasks first. <!-- test: differential_cancel_a_scope_with_a_faulted_task_gives_an_error_to_a_function_returning_a_result, differential_cancel_a_scope_with_a_faulted_task_gives_an_error_to_a_function_returning_a_result -->

```mote
fn main() {
    let zero = 0
    scope {
        spawn { return 1 / zero } // faults DivideByZero
    }
}
```

## What a task may capture

* A `spawn` block uses only Sendable values: capturing a `var`, or a `let` or parameter whose type is not Sendable, is an error. <!-- test: a_spawn_block_uses_only_sendable_values -->

<!-- test: a_spawn_block_captures_numbers_text_and_handles_and_no_collection_that_can_change, a_spawn_block_uses_only_sendable_values, an_array_of_plain_elements_is_sendable_by_copy, a_mutex_and_a_once_are_sendable_handles, a_port_end_is_a_sendable_handle_a_struct_may_hold_and_a_native_may_take -->
| Sendable | Not Sendable |
|---|---|
| numbers, `Bool`, `String` | a `List`, `Map`, `Set`, a `Stream` |
| an array `[T]` or `[T; N]` of numbers, `Bool`, `String` or such arrays | an array of anything else |
| structs of Sendable fields, classes with no `var` and Sendable fields | a class with a `var` field |
| tuples, unions, optionals, `Result`, enums of Sendable payloads | an enum with a payload that is not Sendable |
| `Sender` `Receiver` `Task` `Mutex` `Once` `atomic.Int64` `atomic.Bool` `PortEnd` | a `Pointer`, a type parameter, a function value |

* A function parameter that `spawn` will call is written `f: Send (Int) -> Int`. A named function takes that type where it is expected; a lambda must capture no `var` and only Sendable values; a function value not typed `Send` does not fit. <!-- test: a_send_function_takes_named_functions_and_lambdas_of_sendable_captures, a_send_lambda_captures_no_var -->
* A task's result is Sendable: `spawn { return make_list() }` is an error. A `Send` function returns a Sendable value too. <!-- test: a_task_returns_only_a_sendable_value, a_send_function_returns_only_a_sendable_value -->
* An array is copied where it enters another task: by `send`, by a `spawn` capture and by a task's result. <!-- test: an_array_enters_a_task_as_a_copy_going_in_and_coming_out, an_array_sent_on_a_channel_is_copied_before_it_goes -->

## Channels

* `Channel<T>(n)` returns a `(Sender<T>, Receiver<T>)` pair. `n` is the buffer; `0` is a rendezvous. <!-- block -->

```mote
import Channel from std.sync

fn main() {
    let (tx, rx) = Channel<Int>(3)
    tx.send(10)
    tx.close()
    println(rx.recv())
    println(rx.recv())
    let (rtx, rrx) = Channel<Int>(0)
    let h = spawn {
        var sum = 0
        for v in rrx { sum += v }
        return sum
    }
    rtx.send(4)
    rtx.send(5)
    rtx.close()
    println(h.await().unwrap())
}
```

```output
10
null
9
```

<!-- test: differential_channels_the_compiled_code_shows_the_recorded_results, differential_channels_the_compiled_code_shows_the_recorded_results, differential_channels_the_compiled_code_shows_the_recorded_results, differential_channels_the_compiled_code_shows_the_recorded_results, differential_streams_the_compiled_code_shows_the_recorded_results -->
| Call | Result |
|---|---|
| `tx.send(v)` | queues `v`; parks while full; faults on a closed `Sender` |
| `rx.recv()` | `Some(v)`, parking while empty; `None` once closed and drained |
| `tx.close()` | this `Sender` is finished |
| `tx.clone()` `rx.clone()` | another end of the same channel |
| `rx.stream()` `for x in rx` | every message until close |

* A channel closes when its last `Sender` is gone, closed or owned by an ended task. <!-- test: differential_channels_the_compiled_code_shows_the_recorded_results -->
* `T` of a channel must be Sendable and is not an optional, so a `null` from `recv` is a closed channel. <!-- test: a_channel_carries_only_what_may_enter_another_task, a_channel_of_a_type_parameter_is_not_checked_until_it_is_written_out, a_channel_of_optional_messages_is_refused -->
* An end that a `spawn` captures moves into it; `clone()` it first to keep one. A use after the `spawn` on any path, including the next turn of a loop, is an error. <!-- test: a_channel_end_is_moved_by_the_spawn_that_captures_it, a_channel_end_is_moved_on_some_path_when_one_branch_spawns, a_spawn_in_a_loop_moves_the_end_for_the_next_turn, a_lambda_over_a_moved_end_is_a_use, other_captures_and_handles_are_not_moved -->

## Producer and consumer

* A producer task sends, and the consumer reads until the channel closes. <!-- block -->

```mote
import Channel from std.sync

fn main() {
    var total = 0
    scope {
        let (tx, rx) = Channel<Int>(2)
        spawn {
            tx.send(1)
            tx.send(2)
        }
        for v in rx { total += v }
    }
    println(total)
}
```

```output
3
```

* A send, `recv` or `await` that no other runnable task can end is reported as a `deadlock`. <!-- test: differential_deadlock_the_compiled_code_shows_the_recorded_results -->

## `Mutex<T>`

* One value that tasks change one at a time. `Mutex(x)` takes its type from `x`. <!-- test: mutex_once_and_atomics_have_their_methods -->

<!-- test: mutex_once_and_atomics_have_their_methods -->
| Call | Result |
|---|---|
| `Mutex(v)` | a mutex holding a `deep_clone()` of `v` |
| `m.lock(\|v\| { … })` | runs the body with `v` writable once no other task is inside; the value the body returns is the result of `lock` |
| `m.get()` | a `deep_clone()` of the whole value, taken once no other task is inside |
| `m.set(v)` | replaces the value with a `deep_clone()` of `v` |

```mote
fn main() {
    let hits = Mutex(0)
    scope {
        for i in 0..5 {
            spawn { hits.lock(|n| { n += 1 }) }
        }
    }
    println(hits.lock(|n| { return n }))

    var items = [1, 2]
    let shared = Mutex(items)
    items.push(3)
    println(shared.get())
    shared.set([7])
    println(shared.get())
}
```

```output
5
[1, 2]
[7]
```

* The mutex owns its value: the caller's `v` and the mutex share nothing. <!-- test: mutex_once_and_atomics_have_their_methods -->
* The body is a `Send` function: it captures no `var` and only Sendable values, and its result is Sendable. <!-- test: a_send_lambda_captures_no_var, a_send_function_returns_only_a_sendable_value -->
* The body may not park: an `await`, `send`, `recv`, `sleep`, a `lock`, `get` or `set` of any mutex, a `get_or_init`, or a `for` over a `Receiver` or a `Stream` is an error when the body reaches it through the functions it calls. A `spawn` inside it runs elsewhere and is not followed. <!-- test: lock_must_not_park, lock_inside_lock_and_lock_by_a_named_function_are_followed, get_and_set_of_a_mutex_wait_like_lock, lock_may_spawn_and_do_plain_work, a_lock_body_that_parks_is_refused -->
* A task that locks a mutex it is already inside faults with `Mutex is already locked by this task`. <!-- test: differential_sync_the_compiled_code_shows_the_recorded_results -->
* A task that finds the mutex held waits for the holder. <!-- test: differential_sync_the_compiled_code_shows_the_recorded_results -->
* A fault in the body leaves the mutex locked; the next `lock` faults with `Mutex was left locked by a task that stopped`. <!-- test: differential_sync_the_compiled_code_shows_the_recorded_results -->
* A `Mutex` is Sendable for any `T`, and cannot hold a function or a `Stream`. <!-- test: a_mutex_and_a_once_are_sendable_handles, a_mutex_or_once_cannot_hold_a_function_or_a_stream -->

## `Once<T>`

* A value made exactly once, on first use. <!-- test: mutex_once_and_atomics_have_their_methods -->

<!-- test: mutex_once_and_atomics_have_their_methods -->
| Call | Result |
|---|---|
| `Once<T>()` | an empty cell |
| `o.get_or_init(\|\| value)` | the value; the first call runs the body and later calls return what it made |

```mote
fn main() {
    let config = Once<String>()
    println(config.get_or_init(|| "mote"))
    println(config.get_or_init(|| "other"))
}
```

```output
mote
mote
```

* `T` cannot change inside (a `List`, `Map`, `Set`, array or a class with a `var` is an error) and cannot be a function or a `Stream`. <!-- test: a_once_holds_only_what_cannot_change_inside, a_mutex_or_once_cannot_hold_a_function_or_a_stream -->
* The body is a `Send` function, like the body of `lock`; it may park. <!-- test: a_send_function_takes_named_functions_and_lambdas_of_sendable_captures -->
* A fault in the body leaves the cell empty; the next call runs its own body. <!-- test: differential_sync_the_compiled_code_shows_the_recorded_results -->
* Asking the same `Once` from its own body faults with `Once is used inside its own initializer`. <!-- test: differential_sync_the_compiled_code_shows_the_recorded_results -->
* `get_or_init` may wait, so a `lock` body may not call it. <!-- test: lock_must_not_park -->

## Atomics

* `atomic.Int64` and `atomic.Bool` are single values that tasks read and write without a lock. <!-- test: mutex_once_and_atomics_have_their_methods -->

<!-- test: mutex_once_and_atomics_have_their_methods -->
| Type | Made by | Calls |
|---|---|---|
| `AtomicInt` | `AtomicInt(n)` | `load()`; `store(n)`; `swap(n)` returns the old value; `compare_exchange(expected, n)` stores `n` and returns `true` only when the value was `expected`; `fetch_add(n)` and `fetch_sub(n)` return the old value |
| `AtomicBool` | `AtomicBool(b)` | `load()`; `store(b)`; `swap(b)`; `compare_exchange(expected, b)` |

```mote
fn main() {
    let done = AtomicInt(0)
    scope {
        for i in 0..3 {
            spawn { done.fetch_add(1) }
        }
    }
    println(done.load())
    println(done.swap(10))
    println(done.compare_exchange(10, 11))
    println(done.compare_exchange(10, 12))
    println(done.load())
    let flag = AtomicBool(false)
    println(flag.swap(true))
    println(flag.load())
}
```

```output
3
3
true
false
11
false
true
```

* `fetch_add` and `fetch_sub` fault when the result leaves the range of `Int`. <!-- test: differential_sync_the_compiled_code_shows_the_recorded_results -->

## Without waiting, and timeouts

* `rx.try_recv()` gives `Some(v)` or `None` and never waits; `rx.is_closed()` is `true` once closed and drained; `tx.try_send(v)` gives `true` when queued and `false` when full or closed. <!-- test: differential_channels_the_compiled_code_shows_the_recorded_results, differential_channels_the_compiled_code_shows_the_recorded_results -->
* `rx.recv_timeout(ms)` gives `Some(v)`, or `None` when closed or `ms` pass: `is_closed()` tells which. <!-- test: tasks_and_sync_have_their_methods -->

<!-- test: differential_timed_the_compiled_code_shows_the_recorded_results, differential_tasks_the_compiled_code_shows_the_recorded_results, differential_timed_the_compiled_code_shows_the_recorded_results, differential_timed_the_compiled_code_shows_the_recorded_results, differential_channels_the_compiled_code_shows_the_recorded_results, differential_channels_the_compiled_code_shows_the_recorded_results, tasks_and_sync_have_their_methods -->
| Function | Result |
|---|---|
| `sleep(ms)` | parks for `ms` |
| `await_all(tasks)` | each `Result`, in order |
| `with_timeout(ms, f)` | runs `f` in a task: `Some(Result)`, or `None` with the task cancelled |
| `merge(rxs)` | one `Receiver` carrying every message; it closes when all sources do |
| `broadcast(rx, n)` | `n` receivers that each get every message |

* `sleep`, `await_all` and `with_timeout` come from `std.task`; `merge` and `broadcast` from `std.sync`. <!-- test: tasks_and_sync_have_their_methods -->

## Time

* `Instant.now()` of `std.time` reads the scheduler's clock, which is virtual under `--seed`; `elapsed()` is the `Duration` since it, and `since(earlier)` the span between two instants (0 when `earlier` is later). <!-- test: std_rt_time_the_clock_is_virtual_under_a_seed_and_a_sleep_is_exact, std_rt_time_a_sleep_in_a_task_and_the_main_task_share_one_clock -->
* A `Duration` counts nanoseconds; `from_nanos`, `from_micros`, `from_millis` and `from_secs` make one and `as_nanos`, `as_micros`, `as_millis` and `as_secs` read it (rounding down). <!-- test: differential_every_block_of_std_tests_passes -->
* Durations `add`, `sub` (not below 0) and `mul` by an `Int`, compare with `compare` and `==`, and print as `1s`, `250ms`, `7us` or `9ns`. <!-- test: differential_every_block_of_std_tests_passes -->
* `sleep(duration)` of `std.time` parks as `sleep(ms)` does, for the duration rounded up to a millisecond; it has the name of the `std.task` function, so a file that imports both renames one with `as`. <!-- test: std_rt_time_a_sleep_takes_at_least_its_duration_on_the_real_clock -->

## One task owns a resource

* A resource that must not be shared belongs to one task; the others send it requests on a channel, with a reply channel in the request. <!-- block -->

```mote
import Channel from std.sync

struct Add {
    n: Int
    reply: Sender<Int>
}

fn main() {
    let (tx, rx) = Channel<Add>(1)
    spawn {
        for req in rx { req.reply.send(req.n + 1) }
    }
    let (rtx, rrx) = Channel<Int>(1)
    tx.send(Add { n: 41, reply: rtx })
    println(rrx.recv()!)
}
```

```output
42
```
