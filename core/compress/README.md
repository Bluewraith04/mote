# compress

gzip, zlib and raw deflate, by the Rust `flate2` crate. The package carries the crate as a library in `native/<triple>/`.

| Step | Line |
|---|---|
| add it | nothing: it ships with Mote, and its library is copied beside a built program |
| use it | `import compress` |
| build the library | `tools/build-core-natives.sh compress` in the Mote repository, which puts it in `native/<triple>/` for this machine |

The functions are `mote_compress` and `mote_decompress`, in the bytes-in, bytes-out shape of `std.libtools`.

The compressing functions answer `Bytes`, not a `Result`: they fault when the library cannot be opened, which means the library was not built for this machine.

```mote
import compress

fn main() {
    let text = "the quick brown fox ".repeat(100)
    let packed = compress.gzip(text.bytes())
    println("smaller: ${packed.len() < text.len()}")
    println(compress.gunzip(packed).unwrap().decode().unwrap() == text)
}
```

```output
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

Data in the wrong format, or cut short, is an `Err` of kind `InvalidData`, as in `unexpected end of file` for `gunzip` of text that is not gzip. So is output past 1 GiB, which stops a small hostile input from filling memory.

## Tests

`mote test` in this directory runs `tests/`.
