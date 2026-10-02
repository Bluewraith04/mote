# std.data.archive

tar and zip archives, by the Rust `tar` and `zip` crates. `import std.data.archive as archive`.

```mote
import std.data.archive as archive
import std.data.compress as compress

let entries = [
    archive.dir("docs"),
    archive.file("docs/a.txt", "hello".bytes()),
    archive.file("b.txt", "world".bytes()),
]
let tgz = compress.gzip(archive.pack_tar(entries)!)

for e in archive.unpack_tar(compress.gunzip(tgz)!)! {
    println("${e.name} ${e.is_dir()} ${e.data.len()}")
}
```

```output
docs/ true 0
docs/a.txt false 5
b.txt false 5
```

An archive is read or written whole as `Bytes`, as a list of `Entry` with a `name` and `data`; a directory is a name ending in `/` with empty data.

| Function | Answers |
|---|---|
| `file(name, data)`, `dir(name)` | an `Entry` |
| `pack_tar(entries)`, `pack_zip(entries)` | `Result<Bytes, Error>` |
| `unpack_tar(b)`, `unpack_zip(b)` | `Result<List<Entry>, Error>`; tar links and devices are skipped |
| `read_dir(path)` | every file and directory under `path`, sorted, named relative to it |
| `extract(entries, dir)` | writes the entries under `dir` |

Times are zero and modes fixed, so the same entries pack to the same bytes. A name that is empty, absolute, contains `\`, starts with a drive letter, or has a `.` or `..` part is refused everywhere, so an archive cannot write outside its directory. More than 1 GiB unpacked is an error.
