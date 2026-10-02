# Mote documentation

Mote is a statically typed language that compiles to bytecode and runs on its own VM. These pages describe what it does today.

| Page | Covers |
|---|---|
| [Getting started](getting-started.md) | the `mote` command, packages |
| [Basics](basics.md) | values, types, control flow |
| [Functions](functions.md) | parameters, lambdas, generators |
| [Structs, classes and enums](types.md) | user-defined types and methods |
| [Absence and failure](errors.md) | optionals, `Result`, `?` and `!` |
| [Generics and traits](generics.md) | type parameters, bounds, `@derive` |
| [Collections and strings](collections.md) | `List`, `Map`, `Set`, `Bytes`, `String` |
| [Modules and packages](modules.md) | imports, `pub`, the standard-library layout |
| [Concurrency](concurrency.md) | tasks, channels, `Shared` |
| [Memory](memory.md) | where values live, limits |
| [Tools](tools.md) | `mote test`, `mote check --memory` |
| [Reference](reference.md) | lexical rules, operators, attributes, known gaps |

## Standard library

Names in the [prelude](std/prelude.md) need no import.

| Group | Modules |
|---|---|
| Core | [prelude](std/prelude.md), [error](std/error.md), [collections](std/collections.md), [iter](std/iter.md), [stream](std/stream.md), [string](std/string.md), [math](std/math.md), [regex](std/regex.md), [time](std/time.md), [date](std/date.md), [random](std/random.md), [task](std/task.md), [test](std/test.md) |
| `std.data` | [json](std/json.md), [toml](std/toml.md), [yaml](std/yaml.md), [base64](std/base64.md), [uuid](std/uuid.md), [compress](std/compress.md), [archive](std/archive.md), [crypto](std/crypto.md), [sql](std/sql.md) |
| `std.sys` | [io](std/io.md), [fs](std/fs.md), [path](std/path.md), [env](std/env.md), [process](std/process.md), [net](std/net.md), [http](std/http.md), [http_server](std/http_server.md), [tls](std/tls.md) |
| `std.dev` | [log](std/log.md), [args](std/args.md), [libtools](std/libtools.md) |
| `std.experimental` | [types](std/experimental-types.md) |

Every `mote` example in these pages is run by the test suite, and its printed output is checked.
