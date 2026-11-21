# fs: reference

Files and directories. Native: Zig, direct system calls. Every call can fail with an `Err` whose message is `what: reason`.

`import { read_string, write_string, File, list, exists } from fs`

## Whole files

| Function | Notes |
|---|---|
| `read_bytes(path) -> Result<[UInt8], Error>` | every byte |
| `read_string(path) -> Result<String, Error>` | `Err` when not UTF-8 |
| `write_bytes(path, bytes)` / `write_string(path, text)` | replaces the file, creates it when missing |
| `append_bytes(path, bytes)` / `append_string(path, text)` | adds to the end, creates it when missing |

## Open files

`File` is `Closeable`; closing twice is an `Err`; a file left open stays open until the program ends.

| Item | Notes |
|---|---|
| `File.open(path)` | read only, must exist |
| `File.create(path)` | write only; creates or empties; mode 0644 |
| `File.create_new(path)` | write only; `Err` if it exists |
| `File.append(path)` | write at the end; creates |
| `.read_into(buffer) -> Result<Int, Error>` | count read, 0 at the end |
| `.read(count)`, `.read_all()`, `.read_string()` | from the position |
| `.write(bytes)`, `.write_string(text)` | writes all of it |
| `.seek(offset, origin) -> Result<Int, Error>` | `Origin.Start`, `Current`, `End`; gives the new position |
| `.truncate(size)` | cut or extend |
| `.metadata()` | of the open file |
| `.close()` | |

## Paths

| Function | Notes |
|---|---|
| `exists(path)`, `is_file(path)`, `is_dir(path)` | `Bool`; follow symbolic links |
| `metadata(path)`, `link_metadata(path)` | `Result<Metadata, Error>`; the second does not follow a link |
| `Metadata { kind, size, modified, mode }` | `kind` is `FileKind.File Directory Symlink Other`; `modified` in nanoseconds since 1970; `mode` the permission bits; `.is_file()` `.is_dir()` `.is_symlink()` |
| `list(path) -> Result<List<String>, Error>` | names, sorted, no `.` and `..` |
| `create_dir(path)` | parent must exist |
| `create_dir_all(path)` | missing parents too; existing is fine |
| `remove_file(path)` | a file or link |
| `remove_dir(path)` | empty directory |
| `remove_all(path)` | everything below; a link is removed, not followed |
| `rename(from, to)` | moves; the target is replaced when it is a file |
| `copy(from, to) -> Result<Int, Error>` | contents; bytes copied |

## Example

```mote
import { File, Origin, list, read_string, remove_all, create_dir_all, write_string } from fs

fn main() -> Result<Null, Error> {
    create_dir_all("/tmp/mote-fs-demo/a")?
    write_string("/tmp/mote-fs-demo/a/note.txt", "hello\n")?
    let file = File.append("/tmp/mote-fs-demo/a/note.txt")?
    file.write_string("more\n")?
    file.close()?
    println(read_string("/tmp/mote-fs-demo/a/note.txt")?)
    println(list("/tmp/mote-fs-demo/a")?)
    remove_all("/tmp/mote-fs-demo")?
    return Ok(null)
}
```

## Limits

Blocking calls run on the calling thread (files, not sockets); there is no async file I/O, no permission setting, no symbolic-link creation, no watching.

Tests: `tools/test-core.sh fs`.
