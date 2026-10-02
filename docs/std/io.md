# std.sys.io

Standard streams and one-shot file operations. `import std.sys.io as io`.

| Function | Answers |
|---|---|
| `stdout()`, `stderr()` | an `OutStream` with `write(s)`, `write_line(s)`, `flush()`; none can fail, and `write_line` is one write so lines do not interleave |
| `stdin()` | an `InStream`; `read_line()` answers `Ok(Some(line))`, `Ok(None)` at end of input, or `Err` |
| `read_file(path)`, `write_file(path, s)` | `Result` of the whole file as text; replaces the file |
| `read_file_bytes(path)`, `write_file_bytes(path, b)` | the same with `Bytes` |

```mote,skip
let input = io.stdin()
match input.read_line() {
    Ok(line) => match line {
        Some(text) => println("got ${text}")
        None => println("end of input")
    }
    Err(e) => println(e.message)
}
```

For handles, seeking and appending see [fs](fs.md).
