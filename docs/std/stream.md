# std.stream

Lazy `Stream` sources and adapters. Consume one with `for` or `.next()`. A fallible source is a stream of `Result`: a failure arrives as one final `Err`. `import std.stream as stream`.

| Function | Yields |
|---|---|
| `chars(s)` | one `String` per code point |
| `lines(s)` | the lines of `s` |
| `stdin_lines()` | each line of standard input as `Ok(line)` |
| `file_lines(path)` | each line of the file as `Ok(line)`; an unreadable file is one `Err` |
| `map(s, f)`, `filter(s, pred)`, `take(s, n)` | the lazy adapters |

```mote,skip
for r in stream.file_lines("data.txt") {
    match r {
        Ok(line) => println(line)
        Err(e) => println(e.message)
    }
}
```

The adapters pull one element at a time, so they work over an infinite source as long as something downstream, like `take`, stops pulling. Write a source with a [generator](../functions.md#generators). [`std.iter`](iter.md) has the eager equivalents.
