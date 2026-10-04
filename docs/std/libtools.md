# std.dev.libtools

Call functions in a C shared library at run time. `import { open, bind, bind_blocking } from std.dev.libtools`.

```mote,skip
import { open, bind } from std.dev.libtools

fn main() {
    let libm = open("libm.so.6")?
    let pow = bind<(Float, Float) -> Float>(libm, "pow")?
    println(pow(2.0, 10.0))
}
```

This prints `1024.0`. The run must allow native libraries: `mote run --allow-native`, or `MOTE_ALLOW_NATIVE=1`. Without it `open` answers `Err` with kind `PermissionDenied`.

| Function | Answers |
|---|---|
| `open(path)` | `Result<Library, Error>` |
| `open_package(package, name)` | `Result<Library, Error>`; loads `lib<name>.so` (`.dylib`, `.dll`) that a package granted `native = true` carries, with no flag |
| `bind<F>(lib, name)` | `Result<F, Error>`; a call runs on the scheduler's worker, about 1 µs |
| `bind_blocking<F>(lib, name)` | the same, but a call runs on a helper thread while the task waits, about 60 µs |
| `lib.close()` | drops the handle; bound functions keep working |

`F` is a function type and is the C signature, written out: `bind<(Float) -> Float>(lib, "sin")`. A function takes at most six parameters, and the bound function is Sendable.

| Mote type | C type |
|---|---|
| `Int` | `int64_t` |
| `Float` | `double` |
| `Bool` | `int` |
| `String` | `const char *`; as a result, copied out of the returned `char *` |
| `Bytes` | `char *`, which C may write up to `len` |
| `Null` | `void`, as a result only |

A function shaped `fn(in, in_len, out, out_cap) -> Int` needs no pointer type: the caller allocates `out` as `Bytes(n)`, C writes into it and returns the length written, a length above `out_cap` (call again with a bigger buffer) or a negative error code with the message in `out`. The buffer stays valid for the whole call, `bind_blocking` included.

Not supported: narrow or sized C types (write a small C wrapper over `int64_t` and `double`), pointer results, structs by value, variadic functions, callbacks into Mote, and Windows. The call is not checked: a wrong signature is undefined behaviour. A `bind` call that blocks holds its worker, so use `bind_blocking` for anything that may block. Cancelling a task takes effect once the C call returns.
