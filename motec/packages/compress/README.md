# compress

gzip, zlib and raw deflate, by the Rust `flate2` crate. The package carries the crate as a library in `native/<triple>/`, so add it with native access granted, then `import compress as compress`.

```toml
[dependencies]
compress = { git = "github:user/compress", tag = "v0.1.0", native = true }
```

Libraries are built for `x86_64-unknown-linux-gnu` here. To build one for another machine: `cargo build --release --manifest-path rust/Cargo.toml`, then copy `rust/target/release/libmote_compress.so` (`.dylib`, `mote_compress.dll`) into `native/<triple>/`. The functions are `mote_compress` and `mote_decompress`, in the bytes-in, bytes-out shape of `std.dev.libtools`; a negative answer is the negated length of an error message written to the output buffer.

The compressing functions answer `Bytes`, not a `Result`: they fault when the library cannot be opened, which means the package is missing `native = true`.

```mote,skip
import compress as compress

let text = "the quick brown fox ".repeat(100)
let packed = compress.gzip(text.bytes())
println("smaller: ${packed.len() < text.len()}")
println(compress.gunzip(packed).unwrap().decode().unwrap() == text)
```

```text
smaller: true
true
```

## Functions

| Function | Answers |
|---|---|
| `gzip(b)`, `zlib(b)`, `deflate(b)` | the compressed `Bytes` at level 6; `deflate` is raw, with no header |
| `gzip_level(b, level)`, `zlib_level(b, level)`, `deflate_level(b, level)` | the same at level 0 (store) to 9 (smallest); a level outside that range is clamped |
| `gunzip(b)`, `unzlib(b)`, `inflate(b)` | `Result<Bytes, Error>` with the original bytes |

Data and compressed form are whole `Bytes`; nothing streams. A `.tar.gz` is `gunzip` and then `unpack_tar`.

## What is refused

Data in the wrong format, or cut short, is an `Err` of kind `InvalidData`. So is output past 1 GiB, which stops a small hostile input from filling memory.
