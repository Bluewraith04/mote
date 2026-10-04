# path

Pure text operations on `/`-separated paths. Nothing here touches the disk or follows symlinks. A Mote package with no native code of its own: add it with `mote add github:user/path` (or a `path` dependency), then `import path`.

| Function | Answers |
|---|---|
| `is_absolute(p) -> Bool` | whether `p` starts with `/` |
| `components(p) -> List<String>` | the non-empty parts; a leading `/` is the first part |
| `join(a, b) -> String` | `b` if it is absolute or `a` is empty; else `a` and `b` with one `/` between |
| `file_name(p) -> String?` | the last component; `None` for `""` and `"/"` |
| `parent(p) -> String?` | everything before the last component; `None` for `""` and `"/"`, `Some("")` for a bare name |
| `extension(p) -> String?` | the text after the last `.` of the file name; `None` without one (`.bashrc` has none) |
| `stem(p) -> String?` | the file name without its extension |
| `with_extension(p, ext) -> String` | `p` with its extension replaced; an empty `ext` removes it |
| `normalize(p) -> String` | drops `.` and empty parts and resolves `..` lexically |

```mote
import path

println(path.join("a/b", "c.txt"))
println(path.extension("a/b/c.tar.gz").unwrap())
println(path.normalize("a/./b/../c"))
```

```output
a/b/c.txt
gz
a/c
```
