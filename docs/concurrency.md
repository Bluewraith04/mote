# Concurrency

A task is a block of code that runs alongside the rest of the program. Tasks communicate through channels and shared cells, never through shared mutable variables.

| Piece | Purpose |
|---|---|
| `spawn { … }` | starts a task and gives a `Task<T>` |
| `scope { … }` | waits at its closing brace for the tasks started inside |
| `Channel<T>(n)` | a message queue with a `Sender<T>` and a `Receiver<T>` end |
| `Shared<T>` | one value many tasks read and take turns to write |

Tasks run on a pool of worker threads, one per core by default. A task gives way at loop back-edges, calls, every 1024 instructions, and when it waits on a `join`, a channel, a `Shared` write or a blocking native call. The type checker enforces what may cross between tasks, so a program that compiles has no data races.

## Tasks

`spawn { … }` is an expression of type `Task<T>`, where `T` is what the block returns. `spawn f(args)` is shorthand for a block that calls `f`.

| Call | Result |
|---|---|
| `h.join()` | waits; `Ok(value)`, or `Err(message)` if the task faulted |
| `h.cancel()` | asks the task to stop at its next safepoint, `join`, `send` or `recv` |
| `h.is_ready()` | `true` once the task has finished; never waits |

```mote
fn main() {
    let h1 = spawn {
        var s = 0
        var i = 0
        while i < 100 {
            s = s + i
            i = i + 1
        }
        return s
    }
    let h2 = spawn { return 7 * 6 }
    println(h1.join()! + h2.join()!)
}
```

```output
4992
```

### Scopes

A `scope { … }` block waits at its closing brace for every task spawned inside it. A `spawn` outside a scope is detached: it is dropped if still pending when `main` returns. Membership is by where the `spawn` is written, so a helper called from inside a scope detaches its tasks.

```mote
fn main() {
    var out = 0
    scope {
        let h = spawn { return 20 + 1 }
        out = h.join()! * 2
    }
    println(out)
}
```

```output
42
```

### What a task may capture

A `spawn` block may use only Sendable values.

| Sendable | Not Sendable |
|---|---|
| `Int`, `Float`, `Bool`, `Char`, `String` | a mutable `List`, `Map`, `Set` or `Bytes` |
| structs of sendable fields | a class with a `var` field |
| classes with no `var` field and sendable fields | a lambda that captures a `var` |
| `Sender<T>`, `Receiver<T>`, `Task<T>`, `Shared<T>` | |
| a named function, or a lambda capturing no `var` | |

Capturing anything else is a compile error; put it in a `Shared` cell. A parameter that `spawn` will call is written `f: Send (Int) -> Int`.

### Faults

A fault in a task never crashes its parent directly.

| Situation | Outcome |
|---|---|
| the parent calls `join()` | the `Result` is the parent's to handle |
| nobody joins, inside a scope, in a function returning `Result` | the scope's closing brace returns `Err` |
| nobody joins, anywhere else | the program panics at the closing brace |
| a task in a scope faults | the scope's other tasks are cancelled |

```mote
fn run_it() -> Result<Int, Error> {
    scope {
        spawn {
            let xs = [1, 2]
            xs.get(5)
        }
    }
    return Ok(1)
}

println(run_it().is_err())
```

```output
true
```

Which task runs between two others is the scheduler's choice. `mote test --schedule-seed N` varies where tasks switch, reproducibly, to find code that depends on it.

## Channels

`Channel<T>(n)` makes a channel and returns its two ends as a pair; `n` is the buffer size, and `0` makes each send wait for a receiver. A channel closes when its last `Sender` is gone, closed or owned by a task that ended; a `Receiver` then drains the buffer and gets `None`.

| Call | Result |
|---|---|
| `tx.send(v)` | queues `v`, parking while full; faults on a closed `Sender` |
| `rx.recv()` | `Some(v)`, parking while empty; `None` once closed and drained |
| `tx.close()` | this `Sender` is finished |
| `tx.clone()`, `rx.clone()` | another end of the same channel |
| `for x in rx`, `rx.iter()`, `rx.stream()` | every message until close |

```mote
fn main() {
    var total = 0
    scope {
        let (tx, rx) = Channel<Int>(2)
        spawn {
            tx.send(1)
            tx.send(2)
            tx.send(3)
        }
        for v in rx {
            total = total + v
        }
    }
    println(total)
}
```

```output
6
```

A `Sender` or `Receiver` captured by a `spawn` moves into it; `clone()` it first to keep one. Sending on a channel nobody drains, with no other runnable task, is reported as a `deadlock` error instead of hanging.

## Shared

A `Shared<T>` holds a sealed, read-only version of a `T`. Readers never wait. Writers take turns, and each write commits a new version.

| Call | Result |
|---|---|
| `Shared(v)` | a cell holding a sealed copy of `v` |
| `x.into_shared()` | moves `x` into a cell without copying |
| `s.get()` | the current version, read-only |
| `s.update(\|v\| { … })` | runs on a copy and commits it when the function returns |
| `s.set(v)` | replaces the value |
| `s.wait_until(\|v\| cond)` | parks until a version satisfies `cond`, then returns it |

```mote
class Table {
    var rows: List<Int>
}

fn main() {
    var hits = Shared(0)
    hits.update(|n| { n += 1 })
    println(hits.get())

    var t = Shared(Table { rows: [1, 2, 3] })
    scope {
        spawn { t.update(|v| { v.rows.push(4) }) }
    }
    println(t.get().rows.len())
}
```

```output
1
4
```

Writing a snapshot from `get()` is a compile error; `.clone()` gives a copy that can change. A fault inside `update` commits nothing. Only a `var` binding may call `update` or `set`. Every write copies the version, so a large value written often pays a copy per write.

## One task owns a resource

There is no lock type. A resource that cannot be copied, such as a file or a socket, belongs to one task; other tasks send it requests on a channel, with a reply channel in each request that wants an answer.

```mote
struct Total {
    var sum: Int
}

struct Add {
    n: Int
    reply: Sender<Int>
}

fn main() {
    let (requests, inbox) = Channel<Add>(8)
    spawn {
        var total = Total { sum: 0 }
        var open = true
        while open {
            let next = inbox.recv()
            if next.is_some() {
                let add = next!
                total.sum += add.n
                add.reply.send(total.sum)
            } else {
                open = false
            }
        }
    }
    let (reply, answers) = Channel<Int>(1)
    requests.send(Add { n: 4, reply: reply })
    println(answers.recv()!)
    requests.send(Add { n: 5, reply: reply })
    println(answers.recv()!)
    requests.close()
}
```

```output
4
9
```
