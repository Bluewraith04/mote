# archive

tar and zip archives, by the Rust `tar` and `zip` crates. The package carries the crates as a library in `native/<triple>/`, so add it with native access granted, then `import archive as archive`.

```toml
[dependencies]
archive = { git = "github:user/archive", tag = "v0.1.0", native = true }
```

Libraries are built for `x86_64-unknown-linux-gnu` here. To build one for another machine: `cargo build --release --manifest-path rust/Cargo.toml`, then copy `rust/target/release/libmote_archive.so` (`.dylib`, `mote_archive.dll`) into `native/<triple>/`. The functions are `mote_tar_pack`, `mote_tar_unpack`, `mote_zip_pack` and `mote_zip_unpack`, in the bytes-in, bytes-out shape of `std.dev.libtools`; entries cross as frames, `<name length> <data length>\n`, the name, then the data.

```mote,skip
import archive as archive
import compress as compress

let entries = [
    archive.dir("docs"),
    archive.file("docs/a.txt", "hello".bytes()),
    archive.file("b.txt", "world".bytes()),
]
let tgz = compress.gzip(archive.pack_tar(entries).unwrap())

for e in archive.unpack_tar(compress.gunzip(tgz).unwrap()).unwrap() {
    println("${e.name} ${e.is_dir()} ${e.data.len()}")
}
```

```text
docs/ true 0
docs/a.txt false 5
b.txt false 5
```

An archive is read or written whole as `Bytes`, as a list of `Entry`. An `Entry` has a `name` and its `data`; a directory is a name ending in `/` with empty data.

## Functions

| Function | Answers |
|---|---|
| `file(name, data)`, `dir(name)` | an `Entry`; `dir` adds the trailing `/` |
| `e.is_dir()` | whether the name ends in `/` |
| `pack_tar(entries)`, `pack_zip(entries)` | `Result<Bytes, Error>`; zip is deflated at level 6 |
| `unpack_tar(b)`, `unpack_zip(b)` | `Result<List<Entry>, Error>`; tar links and devices are skipped |
| `read_dir(path)` | every file and directory under `path`, named relative to it, in sorted order with a directory before its contents |
| `extract(entries, dir)` | writes the entries under `dir`, creating it and any directories |

Times are zero and modes are `0644` for a file and `0755` for a directory, so the same entries pack to the same bytes. A downloaded archive unpacks with `extract(unpack_zip(bytes)?, "out")`, and a directory packs with `pack_tar(read_dir("src")?)`.

## What is refused

| Error | When |
|---|---|
| `unsafe entry name x` | a name that is empty, starts with `/`, contains `\`, starts with a drive letter like `C:`, or has a `.` or `..` part; packing, unpacking and `extract` all refuse it, so an archive cannot write outside its directory |
| bad archive | data that is not that format, or is cut short |
| too large | more than 1 GiB unpacked |
