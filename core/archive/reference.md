# archive: reference

tar, tar.gz and zip, in memory and to and from a directory. Native: a Rust crate (`tar`, `zip`). Uses `compress`, `fs`.

`import { read, write, pack, extract, Entry, EntryKind, Format } from archive`

| Item | Notes |
|---|---|
| `Format` | `Tar TarGz Zip` |
| `EntryKind` | `File Directory Symlink Hardlink` |
| `Entry` | `path, kind, mode, mtime, target, data` (all `pub`); `mtime` in seconds since 1970 |
| `Entry.file(path, data)` | mode 0644, time 0 |
| `Entry.directory(path)` | mode 0755, time 0 |
| `Entry.symlink(path, target)` | |
| `.with_mode(mode)` / `.with_mtime(seconds)` | changed copy |
| `.text() -> String?` | contents as text, null if not UTF-8 |
| `read(format, bytes, limit = -1) -> Result<List<Entry>, Error>` | in order; `limit` bounds the bytes of contents (of the unpacked tar for `TarGz`); `Err` for a bad archive or one over the limit |
| `write(format, entries) -> Result<[UInt8], Error>` | `Err` for a link in a zip other than a symbolic one, or an invalid path |
| `pack(format, directory) -> Result<[UInt8], Error>` | everything under it, paths relative to it, sorted; links and special files left out |
| `extract(format, bytes, directory, limit = -1) -> Result<Int, Error>` | writes files and directories under `directory`; returns how many files; links skipped; permission bits and times not applied; an absolute path or one with `..` is an `Err` |

```mote
import { Entry, EntryKind, Format, read, write } from archive

fn main() -> Result<Null, Error> {
    let bytes = write(Format.Zip, [Entry.directory("docs"), Entry.file("docs/a.txt", "hello".bytes())])?
    for entry in read(Format.Zip, bytes)? {
        println("${entry.path} directory=${entry.kind == EntryKind.Directory}")
    }
    return Ok(null)
}
```

Always pass a `limit` to `extract` and `read` for archives you did not make.

Tests: `tools/test-core.sh archive`.
