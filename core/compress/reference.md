# compress: reference

gzip, zlib and raw deflate. Native: a Rust crate (`flate2`).

`import { compress, decompress, Encoder, Decoder, Format } from compress`

| Item | Notes |
|---|---|
| `Format` | `Gzip Zlib Deflate` (the container around the deflate data) |
| `compress(format, data: [UInt8], level = 6) -> Result<[UInt8], Error>` | one stream; level 0 (stored) to 9 (smallest) |
| `decompress(format, data, limit = -1) -> Result<[UInt8], Error>` | `Err` for corrupt or cut data, or output over `limit` bytes (-1: none) |
| `Encoder.new(format, level = 6) -> Result<Encoder, Error>` | `Err` for another level; `Closeable` |
| `.push(data) -> Result<[UInt8], Error>` | compresses a piece; returns the output ready so far (may be empty) |
| `.finish() -> Result<[UInt8], Error>` | ends the stream, returns the rest |
| `Decoder.new(format, limit = -1)` | `Closeable` |
| `.push(data)` | `Err` for corrupt data, data after the end, or output over the limit |
| `.finish()` | `Err` when the stream was not whole |

Set a `limit` when the input is not yours (a small input can expand a great deal).

```mote
import { Format, compress, decompress } from compress

fn main() -> Result<Null, Error> {
    let packed = compress(Format.Gzip, "hello hello hello hello".bytes())?
    println(packed.len())
    println(String.from_bytes(decompress(Format.Gzip, packed)?)!)
    return Ok(null)
}
```

Tests: `tools/test-core.sh compress`.
