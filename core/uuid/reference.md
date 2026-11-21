# uuid: reference

UUIDs, versions 4 (random) and 7 (time ordered). Pure Mote over `random`, `base64`.

`import { Uuid } from uuid`

| Item | Notes |
|---|---|
| `Uuid.nil()` | 16 zero bytes; `.is_nil()` |
| `Uuid.v4()` | random, from the system's source; `Result` |
| `Uuid.v4_from(var random)` | from a seeded `Random`, for tests and replays |
| `Uuid.v7(unix_millis)` | 48 bits of Unix time in ms, then random bits from the system; there is no clock here, so the caller passes the time |
| `Uuid.v7_from(unix_millis, var random)` | the same with a seeded generator |
| `Uuid.from_bytes(bytes)` | exactly 16 bytes, taken as they are |
| `Uuid.parse(text)` | `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` or the 32 digits alone, either case |
| `Uuid.from_base64(text)` | 22 URL-safe characters |
| `.bytes()` | the 16 bytes |
| `.version()` | the first hex digit of the third group |
| `.unix_millis() -> Int?` | the time of a v7 value; null for another version |
| `.to_string()` | lowercase with hyphens |
| `.to_simple()` | 32 hex digits |
| `.to_base64()` | 22 URL-safe characters, no padding |

`Uuid` is `Eq`, `Ord` (the order of the bytes, so v7 values sort by time) and `Display`.

```mote
import { Uuid } from uuid

fn main() -> Result<Null, Error> {
    let id = Uuid.v4()?
    println(id)
    println(Uuid.parse(id.to_simple())? == id)
    return Ok(null)
}
```

Tests: `tools/test-core.sh uuid`.
