# log

Leveled, structured logging on one process-wide logger. A Mote package with no native code of its own: add it with `mote add github:user/log` (or a `path` dependency), then `import log`.

```mote,skip
import log

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

## Writing a record

| Call | Level |
|---|---|
| `log.debug(msg, fields)` | `Debug` |
| `log.info(msg, fields)` | `Info` |
| `log.warn(msg, fields)` | `Warn` |
| `log.error(msg, fields)` | `Error` |

`fields` is optional. It is a `Map<String, Field>`, where `Field` is `String | Int | Float | Bool | Json`, written as a literal at the call. A map built earlier must be declared `Map<String, log.Field>`; a `Map<String, Int>` is not accepted. A list or a map is logged as `Json`, and so is a struct that derives `Json`: `{"user": user.to_json()}`. See [Records](json.md#records).

## Levels

`Level` is `Debug`, `Info`, `Warn`, `Error` and `Off`, in that order. A record is written when its level is at or above the threshold, which starts at `Info`. `Off` is a threshold only: it silences everything.

## Settings

The settings are shared by every task. A library should log and leave them to the program.

| Function | Sets | Default |
|---|---|---|
| `log.set_level(Level)` | the threshold | `Info` |
| `log.set_format(Format)` | `Format.Text` or `Format.Json` | `Text` |
| `log.set_output(Output)` | `Output.Stderr` or `Output.File(path)` | `Stderr` |
| `log.set_fields(map)` | fields added to every record, in place of any set before | none |

A call's own field replaces a base field with the same key.

The environment variable `MOTE_LOG` names a level (`debug`, `info`, `warn`, `error` or `off`, in any case). When it does, that level is the threshold and `set_level` has no effect. It is read once, when the program starts; any other value is ignored.

## Formats

| Format | A record is |
|---|---|
| Text | `time LEVEL message key=value key=value` on one line |
| Json | `{"time":"…","level":"info","msg":"…","key":value}` on one line |

The time is UTC, to the millisecond. In text, a `String` value is quoted, as JSON text, when it is empty or holds a space, `=`, `"` or a newline; a `Json` value is written as compact JSON. In JSON, the fields sit beside `time`, `level` and `msg`, and a field with one of those names gets a trailing `_`. A `Float` that is `NaN` or infinite is written as a string.

## Files and faults

`Output.File(path)` appends to the file, opening it for each record. A record that cannot be written is lost; logging never faults. Each record is written whole, so lines from different tasks do not interleave.

There is no function sink yet: a shared cell cannot hold a function. The destination is one of the two outputs above.
