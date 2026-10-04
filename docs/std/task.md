# std.task

Join-combinators over live `Task<T>` handles, and pinning a task to the main thread. `import std.task`.

| Function | Answers |
|---|---|
| `all(tasks: List<Task<T>>) -> Result<List<T>, String>` | every result, in list order; stops at the first `Err` and leaves later tasks running |
| `any(tasks: List<Task<T>>) -> Result<T, String>` | whichever task resolves first; cancels all the others |
| `pin()` | moves the calling task to the program's main thread, where it stays until `unpin()` |
| `unpin()` | returns the calling task to the worker pool |
| `is_pinned() -> Bool` | whether the calling task runs on the main thread |

Both take a list, since there is no variadic call of handles. Neither needs a `scope { }`.

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
