# log: reference

Leveled logging with key-value fields, written by a sink. There is no clock, so a record has no time, and no global logger: make one with `Logger.new`. Pure Mote.

`import { Logger, Level, Record } from log`

| Item | Notes |
|---|---|
| `Level` | `Debug Info Warn Error`; `.number()` 0 to 3; prints its name |
| `Logger.new(name, sink: (Record) -> Null)` | passes records at `Info` or above to the sink |
| `Logger.to_stdout(name)` / `to_stderr(name)` | the sink prints `record.format()` |
| `.name()`, `.level()`, `.set_level(level)` | `set_level` needs a `var` logger |
| `.enabled(level) -> Bool` | |
| `.log(level, message, fields = [])` | passes the record when `level` is at least the logger's |
| `.debug(msg, fields = [])` `.info` `.warn` `.error` | |
| `Record { level, logger, message, fields }` | `fields: List<(String, String)>` |
| `record.format() -> String` | `LEVEL logger: message key=value`, without `logger: ` for an unnamed logger |

A sink may be a lambda that captures a `var` (tests collect lines that way); such a logger cannot be captured by a spawned task (a lambda that captures a `var` is not `Send`).

```mote
import { Logger, Level } from log

fn main() {
    var logger = Logger.to_stdout("app")
    logger.info("started", [("port", "8080")])
    logger.debug("hidden")
    logger.set_level(Level.Debug)
    logger.debug("shown")
}
```

Tests: `tools/test-core.sh log`.
