# std.data.crypto

Hashes, HMAC, secure random bytes and password hashing, over the RustCrypto crates. `import std.data.crypto as crypto`. Data is `Bytes`.

| Function | Answers |
|---|---|
| `sha256(data)`, `sha384`, `sha512` | the hash as `Bytes` |
| `hmac_sha256(key, data)`, `hmac_sha384`, `hmac_sha512` | the tag as `Bytes` |
| `constant_time_eq(a, b)` | `Bool`, in the same time wherever the bytes differ |
| `to_hex(data)`, `from_hex(text)` | lowercase hex text; `Result<Bytes, Error>` |
| `random_bytes(n)` | `Result<Bytes, Error>`, 0 to 64 MiB from the operating system |
| `hash_password(password)` | `Result<String, Error>`: an Argon2id hash with a fresh salt |
| `verify_password(password, hash)` | `Result<Bool, Error>`; `false` for a wrong password, `Err` when `hash` is not a password hash |

```mote
import std.data.crypto as crypto

fn main() {
    println(crypto.to_hex(crypto.sha256("abc".bytes())))
    let tag = crypto.hmac_sha256("key".bytes(), "message".bytes())
    println(crypto.constant_time_eq(tag, tag))
}
```

```output
ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
true
```

```mote
import std.data.crypto as crypto

fn main() {
    let hash = crypto.hash_password("correct horse")!
    println(hash.slice(0, 31))
    println(crypto.verify_password("correct horse", hash)!)
    println(crypto.verify_password("wrong", hash)!)
}
```

```output
$argon2id$v=19$m=19456,t=2,p=1$
true
false
```

Compare tags with `constant_time_eq`, not `==`. A password hash carries its parameters and salt, and takes tens of milliseconds. Not included: encryption, signatures, key derivation, MD5, SHA-1, SHA-3, BLAKE3, streaming hashes.
