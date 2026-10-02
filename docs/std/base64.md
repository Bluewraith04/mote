# std.data.base64

Base64 and base64url (RFC 4648), by the Rust `base64` crate. `import std.data.base64 as base64`.

```mote
import std.data.base64 as base64

let packed = base64.encode("foobar".bytes())
println(packed)
println(base64.decode(packed)!.decode()!)
println(base64.encode_url(Bytes(2)))
```

```output
Zm9vYmFy
foobar
AAA
```

| Function | Answers |
|---|---|
| `encode(b)` | the standard alphabet, padded with `=` |
| `decode(s)` | `Result<Bytes, Error>` |
| `encode_url(b)` | the URL-safe alphabet, no padding |
| `decode_url(s)` | `Result<Bytes, Error>` |

Decoding answers an `Err` of kind `InvalidData` for a character outside the alphabet and for bad padding: the standard form needs its `=` and the URL form takes none. Whitespace is not skipped.
