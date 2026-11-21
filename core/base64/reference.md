# base64: reference

Base64 text. Pure Mote.

`import { encode, decode, encode_string, decode_string, Alphabet } from base64`

| Function | Notes |
|---|---|
| `encode(bytes, alphabet = Alphabet.Standard, padded = true) -> String` | `=` padding unless `padded` is false |
| `encode_string(text, alphabet, padded)` | of the UTF-8 bytes |
| `decode(text, alphabet = Alphabet.Standard) -> Result<[UInt8], Error>` | padding may be left out; padding that is there must be right |
| `decode_string(text, alphabet)` | `Err` when the bytes are not UTF-8 |
| `Alphabet` | `Standard` (`+/`), `UrlSafe` (`-_`) |

```mote
import { Alphabet, decode_string, encode_string } from base64

fn main() -> Result<Null, Error> {
    println(encode_string("mote"))
    println(encode_string("mote?>", Alphabet.UrlSafe, false))
    println(decode_string("bW90ZQ==")?)
    return Ok(null)
}
```

Tests: `tools/test-core.sh base64`.
