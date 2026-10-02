# std.string

Helpers over `String`; the primitive operations are methods ([prelude](prelude.md)). `import std.string as str`.

| Function | Answers |
|---|---|
| `join(parts, sep)` | the parts with `sep` between them |
| `lines(s)` | `s` split on `\n`; a trailing newline gives a trailing empty piece |
| `is_blank(s)` | whether `s` is empty or all ASCII whitespace |
| `pad_start(s, width, pad)` | `s` with `pad` prepended until it is `width` bytes long |
| `parse_int(s)` | `Result` of the `Int`: an optional `-` and digits only |
| `parse_float(s)` | `Result` of the finite `Float` |

`parse_int` and `parse_float` answer `Err` for overflow, spaces, a leading `+`, and `inf` or `nan`.

```mote
import std.string as str

println(str.join(["a", "b", "c"], ", "))
println(str.pad_start("7", 3, "0"))
println(str.parse_int("42")!)
```

```output
a, b, c
007
42
```
