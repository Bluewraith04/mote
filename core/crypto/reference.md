# crypto: reference

SHA-2 hashes, HMAC and HKDF, hex text. Native: a Rust crate (RustCrypto `sha2`, `hmac`, `hkdf`).

`import { hash, hash_text, hmac, hkdf, equal, Hasher, Algorithm, to_hex, from_hex } from crypto`

| Item | Notes |
|---|---|
| `Algorithm` | `Sha256 Sha384 Sha512`; `.size()` is the digest length in bytes (32, 48, 64) |
| `hash(algorithm, data: [UInt8]) -> [UInt8]` | the digest |
| `hash_text(algorithm, text) -> [UInt8]` | of the UTF-8 bytes |
| `Hasher.new(algorithm)` | a hash in progress; `Closeable` (closing unfinished frees it) |
| `.update(data)` / `.update_text(text)` | add pieces |
| `.finish() -> [UInt8]` | the digest; the hasher is finished after it |
| `hmac(algorithm, key, data) -> [UInt8]` | `algorithm.size()` bytes |
| `hkdf(algorithm, secret, salt, info, length) -> Result<[UInt8], Error>` | RFC 5869; an empty salt is a zero salt; `Err` when `length` is over 255 digests |
| `equal(a, b) -> Bool` | the time depends on the lengths, not on where the bytes differ; use it to compare MACs |
| `to_hex(bytes) -> String` | lowercase |
| `from_hex(text) -> Result<[UInt8], Error>` | either case; `Err` for an odd length or another character |

There are no ciphers, signatures or password hashing here.

```mote
import { Algorithm, hash_text, hmac, to_hex } from crypto

fn main() {
    println(to_hex(hash_text(Algorithm.Sha256, "abc")))
    println(to_hex(hmac(Algorithm.Sha256, "key".bytes(), "message".bytes())))
}
```

The first line is `ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad`.

Tests: `tools/test-core.sh crypto`.
