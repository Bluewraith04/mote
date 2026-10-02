# std.task

Join-combinators over live `Task<T>` handles. `import std.task`.

| Function | Answers |
|---|---|
| `all(tasks: List<Task<T>>) -> Result<List<T>, String>` | every result, in list order; stops at the first `Err` and leaves later tasks running |
| `any(tasks: List<Task<T>>) -> Result<T, String>` | whichever task resolves first; cancels all the others |

Both take a list, since there is no variadic call of handles. Neither needs a `scope { }`.

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
