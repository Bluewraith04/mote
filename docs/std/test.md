# std.test

An assertion library and case runner for tests written as code. `import { Suite } from std.test`. For `test { }` blocks see [Tools](../tools.md).

The registry is a value: create a `Suite`, register cases, call `finish()`.

```mote
import { Suite } from std.test

fn main() {
    var t = Suite.new()
    t.test("addition", |var s| { s.assert_eq(1 + 1, 2) })
    t.test("strings", |var s| { s.assert_ne("a", "b") })
    t.finish()
}
```

```output
  ok    addition
  ok    strings
2 passed, 0 failed
```

| Method (each takes `var self`) | What |
|---|---|
| `Suite.new()` | an empty suite |
| `test(name, body)` | runs one case; `body` receives the suite as `var s` |
| `assert(cond, msg)` | records a failure when `cond` is false |
| `assert_eq(a, b)`, `assert_ne(a, b)` | records a failure when the values are unequal, or equal |
| `fail(msg)` | records a failure |
| `finish()` | prints the report; faults if a case failed |

A failed assertion records the failure and the case keeps running; there is no `assert_throws`.
