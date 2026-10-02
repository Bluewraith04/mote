# std.sys.process

Run a child process to completion and capture its output. There is no shell: `program` is looked up on `PATH`. `import std.sys.process as process`.

| Function | Answers |
|---|---|
| `run(program, args)` | `Result`; inherited environment, empty stdin |
| `run_with(program, args, env, stdin, cwd)` | `env` is `KEY=VALUE` overrides; `stdin` is written then closed; an empty `cwd` inherits |
| `exit(code)` | ends this process |

`Ok` carries an `Output` with `status` (`-1` if a signal ended the child), `stdout` and `stderr` as `Bytes`, `success()`, and `stdout_text()`, `stderr_text()` (`String?`). `Err` means the program could not be started.

```mote
import std.sys.process as process

let out = process.run("echo", ["hi"])!
println(out.stdout_text()!)
```

```output
hi
```
