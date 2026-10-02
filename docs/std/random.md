# std.random

A seedable generator; not cryptographically secure (use [`crypto.random_bytes`](crypto.md) for that). `import std.random as random`.

`Rng` owns its state, so a seeded `Rng` is deterministic and there is no hidden global generator.

| Constructor | Answers |
|---|---|
| `Rng.seeded(seed)` | a deterministic generator |
| `Rng.from_entropy()` | a generator seeded from the system |

| Method (each takes `var self`) | Answers |
|---|---|
| `next_bits()` | a non-negative 63-bit integer |
| `next_int(lo, hi)` | uniform in `[lo, hi)`; `lo` if the range is empty |
| `next_float()` | uniform in `[0, 1)` |
| `bool()` | a fair coin |
| `choice(items)` | one element, or `None` for an empty list |
| `shuffle(var items)` | in place, Fisher–Yates |

`random.random()` is one entropy-seeded draw in `[0, 1)`.

```mote
import { Rng } from std.random

var rng = Rng.seeded(42)
println(rng.next_int(1, 7))
```

```output
1
```
