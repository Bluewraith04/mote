# std.time

Durations, monotonic instants and an injectable clock. `import std.time as time`. For calendar time see [date](date.md).

| Type | Members |
|---|---|
| `Duration` | `from_nanos`, `from_millis`, `from_secs`; `as_nanos`, `as_millis`, `as_secs` (truncating); `plus`, `minus`, `compare` |
| `Instant` | a monotonic point: `duration_since(earlier)`, `compare(other)` |
| `Clock` | `Clock.system()`, or `Clock.fake(start_nanos)` which moves only on `advance(d)`; `now()`, `since(earlier)` |

| Function | Answers |
|---|---|
| `now() -> Instant` | the system monotonic clock |
| `unix_millis() -> Int` | wall-clock milliseconds since the Unix epoch |
| `sleep(d)` | parks the calling task; other tasks keep running |
| `ticker(period) -> Receiver<Int>` | receives 1, 2, 3, … once per `period` |

```mote
import { Clock, Duration, sleep } from std.time

let clock = Clock.system()
let start = clock.now()
sleep(Duration.from_millis(10))
println(clock.since(start).as_millis() >= 10)
```

```output
true
```
