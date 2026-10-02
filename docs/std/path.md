# std.sys.path

Text operations on `/`-separated paths. Nothing touches the disk. `import std.sys.path as path`.

| Function | Answers |
|---|---|
| `is_absolute(p)` | whether `p` starts with `/` |
| `components(p)` | the non-empty parts; a leading `/` is the first |
| `join(a, b)` | `b` if absolute or `a` is empty; else `a/b` |
| `file_name(p)`, `parent(p)` | the last component; everything before it; `None` for `""` and `"/"` |
| `extension(p)`, `stem(p)` | the text after the last `.`; the name without it (`.bashrc` has no extension) |
| `with_extension(p, ext)` | `p` with its extension replaced; an empty `ext` removes it |
| `normalize(p)` | drops `.` and empty parts and resolves `..` lexically |

```mote
import std.sys.path as path

println(path.join("a/b", "c.txt"))
println(path.extension("a/b/c.tar.gz")!)
println(path.normalize("a/./b/../c"))
```

```output
a/b/c.txt
gz
a/c
```
