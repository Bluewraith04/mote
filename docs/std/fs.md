# std.fs

File handles, directories and metadata. `import std.fs as fs`. Every operation answers a `Result` whose `Err` is an `Error`.

A `File` is closed by the collector once nothing refers to it, but only when it next runs, so close what you open. `with f = fs.open(path)! { … }` closes it on every exit from the block.

| Function | Answers |
|---|---|
| `open(path)`, `create(path)`, `append(path)` | a `File` for reading; for writing, truncating; for writing at the end |
| `list_dir(path)` | the entry names, sorted |
| `read_dir(path)` | the entries, sorted, each with its name, kind, size, modified time, mode, inode, device and link count as `lstat` gives them |
| `create_dir(path)`, `create_dir_all(path)` | makes a directory (and parents) |
| `remove_file`, `remove_dir`, `remove_dir_all` | removes a file, an empty directory, or a tree |
| `rename(src, dst)` | moves; replaces a file at `dst` |
| `stat(path)` | a `Stat` with `kind`, `size`, `modified` (Unix ms), `mode` (permission bits), `inode`, `device`, `links` (hard links), `is_file()`, `is_dir()`; follows symlinks |
| `lstat(path)` | the same without following a symlink: a link answers kind 3 and `is_symlink()` |
| `read_link(path)` | the text a symlink holds |
| `symlink(target, link)` | makes `link` point at `target`; an existing `link` is an error |
| `set_modified(path, millis)`, `set_mode(path, mode)` | sets the modified time (Unix ms) or the permission bits; follows a symlink |
| `canonicalize(path)` | the path with `.`, `..` and every symlink resolved; it must exist |
| `exists(path) -> Bool` | whether the path exists |
| `read_text`, `read_bytes`, `write_text`, `write_bytes` | one-shot whole-file calls |

| `File` method | Answers |
|---|---|
| `read(max)`, `read_to_end()` | `Bytes`; empty at end of file |
| `read_line()` | `Ok(Some(line))` without its line ending, `Ok(None)` at the end |
| `lines()` | a `Stream<Result>` of the remaining lines |
| `write(b)`, `write_text(s)` | writes |
| `seek(offset)`, `seek_by(offset)`, `seek_end(offset)` | moves; answers the new position |
| `close()` | closes the handle |

```mote
import std.fs as fs

fn main() {
    with f = fs.create("out.txt")! {
        f.write_text("hello\n")
    }

    fs.write_text("out.txt", "hello\n")!
    println(fs.read_text("out.txt")!)
}
```

```output
hello
```
