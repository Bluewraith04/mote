# path: reference

Text operations on `/`-separated paths. Pure Mote; nothing reads the file system.

`import { join, normalize, split, parent, file_name, extension, relative_to, is_absolute } from path`

| Function | Gives | Examples |
|---|---|---|
| `is_absolute(path) -> Bool` | starts with `/` | |
| `join(a, b) -> String` | `b` below `a`; `b` alone if absolute | `join("a", "b")` = `a/b`; `join("a", "/x")` = `/x` |
| `split(path) -> List<String>` | the parts; an absolute path starts with `/` | `split("/a/b")` = `["/", "a", "b"]` |
| `normalize(path) -> String` | no empty parts, no `.`, `..` cancelled where it can; empty result is `.` | `normalize("a/./b/../c")` = `a/c` |
| `parent(path) -> String?` | without the last name; null for `/`, empty and a single relative name | |
| `file_name(path) -> String?` | the last name; null for `/`, empty and a path ending in `..` | |
| `extension(path) -> String?` | text after the last `.` of the file name; null if none or the name starts with the only `.` | `extension("a/b.tar.gz")` = `gz` |
| `relative_to(path, base) -> String?` | `path` below `base` (`.` when equal), else null; never writes `..` | |

A path is text: a name is whatever lies between `/`. There is no `\`, no drive, no home expansion.

Tests: `tools/test-core.sh path`.
