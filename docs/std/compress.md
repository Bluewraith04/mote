# std.data.compress

gzip, zlib and raw deflate, by the Rust `flate2` crate. `import std.data.compress as compress`.

```mote
import std.data.compress as compress

let text = "the quick brown fox ".repeat(100)
let packed = compress.gzip(text.bytes())
println("smaller: ${packed.len() < text.len()}")
println(compress.gunzip(packed)!.decode()! == text)
```

```output
smaller: true
true
```

| Function | Answers |
|---|---|
| `gzip(b)`, `zlib(b)`, `deflate(b)` | compressed `Bytes` at level 6; `deflate` has no header |
| `gzip_level(b, level)`, `zlib_level`, `deflate_level` | level 0 (store) to 9 (smallest); out-of-range levels clamp |
| `gunzip(b)`, `unzlib(b)`, `inflate(b)` | `Result<Bytes, Error>` |

Everything is whole `Bytes`; nothing streams. Data in the wrong format, or cut short, is an `Err` of kind `InvalidData`, and so is output past 1 GiB.
