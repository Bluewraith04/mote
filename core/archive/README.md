# archive

tar and zip archives, by the Rust `tar` and `zip` crates. The package carries the crates as a library in `native/<triple>/`.

| Step | Line |
|---|---|
| add it | nothing: it ships with Mote, and its library is copied beside a built program |
| use it | `import archive` |
| build the library | `tools/build-core-natives.sh archive` in the Mote repository, which puts it in `native/<triple>/` for this machine |

The functions are `mote_tar_pack`, `mote_tar_unpack`, `mote_zip_pack` and `mote_zip_unpack`, in the bytes-in, bytes-out shape of `std.libtools`; entries cross as frames, `<name length> <data length>\n`, the name, then the data.

This example also uses the `compress` package, for the `.tar.gz`:

```mote
import archive
import compress

fn main() {
    let entries = [
        archive.dir("docs"),
        archive.file("docs/a.txt", "hello".bytes()),
        archive.file("b.txt", "world".bytes()),
    ]
    let tgz = compress.gzip(archive.pack_tar(entries).unwrap())

    for e in archive.unpack_tar(compress.gunzip(tgz).unwrap()).unwrap() {
        println("${e.name} ${e.is_dir()} ${e.data.len()}")
    }
}
```

```output
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
| bad archive | data that is not that format, or is cut short, as in `invalid Zip archive: Could not find EOCD` |
| too large | more than 1 GiB unpacked |

## Tests

`mote test` in this directory runs `tests/`.
