# random: reference

A seeded generator, and the system's random bytes. Native: Zig (`getrandom`).

`import { Random, system_bytes, system_int } from random`

| Item | Notes |
|---|---|
| `system_bytes(count) -> Result<[UInt8], Error>` | from the system; for keys and seeds |
| `system_int() -> Result<Int, Error>` | |
| `Random.seeded(seed: Int)` | the same seed gives the same numbers |
| `Random.system() -> Result<Random, Error>` | seeded from the system |
| `.next_bits() -> UInt64` | the next 64 bits |
| `.below(limit) -> Int` | 0 up to but not including `limit` (> 0), every value equally likely |
| `.between(low, high) -> Int` | `low` up to but not including `high` |
| `.float() -> Float` | 0.0 up to but not including 1.0 |
| `.bool() -> Bool` | |
| `.bytes(count) -> [UInt8]` | |
| `.shuffled(items) -> List<T>` | a copy in random order |
| `.choose(items) -> T?` | null for an empty list |

Methods take `var self`, so a generator is a `var`. `Random` is not for secrets: use `system_bytes`.

```mote
import { Random } from random

fn main() {
    var rng = Random.seeded(42)
    println(rng.below(100))
    println(rng.shuffled([1, 2, 3, 4]))
}
```

Tests: `tools/test-core.sh random`.
