# std.dev.log

Leveled, structured logging on one process-wide logger. `import std.dev.log as log`.

```mote,skip
import std.dev.log as log

log.info("started")
log.info("login", {"user": "ann smith", "tries": 3})
log.debug("not shown at the default level")
log.warn("slow", {"ms": 1200})
```

```text
2026-10-02T12:34:56.789Z INFO started
2026-10-02T12:34:56.790Z INFO login user="ann smith" tries=3
2026-10-02T12:34:56.790Z WARN slow ms=1200
```

`log.debug`, `info`, `warn` and `error` take a message and an optional field map. A field is a `String`, `Int`, `Float`, `Bool` or `Json`; a map built earlier must be declared `Map<String, log.Field>`. A list, a map, or a struct that derives `Json` is logged as `Json`: `{"user": user.to_json()}`.

`Level` is `Debug`, `Info`, `Warn`, `Error` and `Off` in that order; a record is written when its level reaches the threshold. The settings are shared by every task, so a library should log and leave them to the program.

| Function | Sets | Default |
|---|---|---|
| `log.set_level(Level)` | the threshold | `Info` |
| `log.set_format(Format)` | `Format.Text` or `Format.Json` | `Text` |
| `log.set_output(Output)` | `Output.Stderr` or `Output.File(path)` | `Stderr` |
| `log.set_fields(map)` | fields added to every record | none |

The environment variable `MOTE_LOG` (`debug`, `info`, `warn`, `error`, `off`) overrides `set_level`; it is read once at start-up.

A text record is `time LEVEL message key=value …` on one line, with a string value quoted when it is empty or holds a space, `=`, `"` or a newline. A JSON record is `{"time":"…","level":"info","msg":"…","key":value}`. The time is UTC to the millisecond. A record that cannot be written is lost; logging never faults. Lines from different tasks do not interleave.
