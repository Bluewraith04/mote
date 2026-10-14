# std.task

Join-combinators over live `Task<T>` handles, and pinning a task to the main thread. `import std.task`.

| Function | Answers |
|---|---|
| `all(tasks: List<Task<T>>) -> Result<List<T>, String>` | every result, in list order; the first `Err` cancels the later tasks |
| `any(tasks: List<Task<T>>) -> Result<T, String>` | whichever task resolves first; cancels all the others |
| `timeout(d: Duration, f: Send () -> T) -> Result<T, Error>` | `f`'s value, or `Err` of kind `TimedOut` when `d` passes first; `f` is cancelled |
| `par_map(items: List<T>, n: Int, f: Send (T) -> U) -> Result<List<U>, String>` | `f` over `items`, at most `n` calls at once, results in the order of `items`; a fault cancels the rest |
| `pin()` | moves the calling task to the program's main thread, where it stays until `unpin()` |
| `unpin()` | returns the calling task to the worker pool |
| `is_pinned() -> Bool` | whether the calling task runs on the main thread |

Both take a list, since there is no variadic call of handles. Neither needs a `scope { }`.

## Semaphore

`Semaphore.new(n)` holds `n` permits that tasks take to limit how many run a section at once. It is Sendable, so every task holds the same one.

| Method | Does |
|---|---|
| `acquire()` | takes a permit, waiting while none is free |
| `release()` | gives back a permit this task took; a fault when it holds none |
| `with_permit(f)` | runs `f` holding a permit and answers its value |
| `available()` | the permits free right now |

A permit belongs to the task that took it. When that task returns, faults or is cancelled, the permits it still holds go back.

```mote
import { Semaphore } from std.task

fn main() {
    let sem = Semaphore.new(2)
    let doubled = sem.with_permit(|| { return 21 * 2 })
    println(doubled)
    println(sem.available())
    sem.acquire()
    println(sem.available())
    sem.release()
    println(sem.available())
}
```

```output
42
2
1
2
```

## Pinning

Tasks run on any worker thread. A task that must call something tied to one thread, such as a window system, pins itself to the thread that started the program. Other tasks stay on the pool and reach it over channels; a pinned task that computes for a long time delays only other pinned tasks. A task's children are not pinned.

```mote
import std.task

fn main() {
    println(task.is_pinned())
    task.pin()
    println(task.is_pinned())
    task.unpin()
    println(task.is_pinned())
}
```

```output
false
true
false
```

```mote,skip
import std.task

fn main() {
    scope {
        let a = spawn { slow_fetch() }
        let b = spawn { fast_fetch() }
        match task.any([a, b]) {
            Ok(v) => println(v)
            Err(e) => println(e)
        }
    }
}
```
