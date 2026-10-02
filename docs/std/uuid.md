# std.data.uuid

UUIDs of version 4 (random) and 7 (time-ordered), by the Rust `uuid` crate. `import std.data.uuid as uuid`.

```mote
import std.data.uuid as uuid
import { Uuid } from std.data.uuid

let id = Uuid.parse("0189F7C0-1234-7ABC-8DEF-0123456789AB")!
println("id: ${id}")
println(id.version())
println(id.to_bytes().len())
println(uuid.v4().version())
```

```output
id: 0189f7c0-1234-7abc-8def-0123456789ab
7
16
4
```

| Call | Answers |
|---|---|
| `uuid.v4()` | a random id |
| `uuid.v7()` | 48 bits of Unix milliseconds, then random bits; ids from different milliseconds sort in creation order |
| `Uuid.nil()` | all zero |
| `Uuid.parse(text)` | `Result`; the `8-4-4-4-12` form in either case, and the 32-digit, braced and `urn:uuid:` forms |
| `Uuid.from_bytes(b)` | `Result`; exactly 16 bytes |
| `u.to_string()`, `"${u}"` | lowercase `8-4-4-4-12` |
| `u.to_bytes()`, `u.version()` | the 16 bytes; the version number |
| `u.to_json()`, `Uuid.from_json(j)` | a string, so a `Uuid` can be a field of a [`@derive(Json)`](json.md#records) type |

An `Err` has kind `InvalidData`. Random bits come from the operating system's secure source.
