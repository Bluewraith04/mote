# std.sys.env

Program arguments and environment variables; the environment is read-only. `import std.sys.env as env`.

| Function | Answers |
|---|---|
| `args() -> List<String>` | `[script, arg1, arg2, …]` |
| `get_var(name) -> String?` | the variable's value |
| `vars() -> Map<String, String>` | every variable |
| `exit(code)` | flushes stdout and stderr, then ends the process with `code` |

```mote
import std.sys.env as env

println(env.args().len())
match env.get_var("NO_SUCH_VARIABLE") {
    Some(value) => println(value)
    None => println("not set")
}
```

```output
1
not set
```

The value `main` returns is not the exit code; call `exit`.
