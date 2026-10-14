# Mote documentation

Mote is a statically typed language that compiles to bytecode and runs on its own VM. These pages describe what it does today.

| Page | Covers |
|---|---|
| [Getting started](getting-started.md) | the `mote` command, packages |
| [Basics](basics.md) | values, types, control flow |
| [Functions](functions.md) | parameters, lambdas, generators |
| [Structs, classes and enums](types.md) | user-defined types and methods |
| [Absence and failure](errors.md) | optionals, `Result`, `?` and `!` |
| [Generics and traits](generics.md) | type parameters, bounds, generated methods |
| [Modifiers](modifiers.md) | a trait that writes its own methods, in a package |
| [Collections and strings](collections.md) | `List`, `Map`, `Set`, `Bytes`, `String` |
| [Modules and packages](modules.md) | imports, `pub`, the standard-library layout |
| [Concurrency](concurrency.md) | tasks, channels, `Shared` |
| [Memory](memory.md) | where values live, limits |
| [Tools](tools.md) | `mote test`, `mote check --memory` |
| [Reference](reference.md) | lexical rules, operators, attributes, known gaps |

## Standard library

Names in the [prelude](std/prelude.md) need no import.

| Kind | Modules |
|---|---|
| Core | [prelude](std/prelude.md), [error](std/error.md), [collections](std/collections.md), [iter](std/iter.md), [stream](std/stream.md), [string](std/string.md), [math](std/math.md), [regex](std/regex.md), [time](std/time.md), [date](std/date.md), [random](std/random.md), [task](std/task.md), [test](std/test.md), [meta](std/meta.md) |
| Data | [json](std/json.md), [base64](std/base64.md), [uuid](std/uuid.md), [crypto](std/crypto.md) |
| Machine | [io](std/io.md), [fs](std/fs.md), [env](std/env.md), [process](std/process.md), [gui](std/gui.md) |
| Network | [socket](std/net.md), [tls](std/tls.md), [http_server](std/http_server.md) |
| Program | [args](std/args.md), [libtools](std/libtools.md) |
| `std.experimental` | [types](std/experimental-types.md) |

Every `mote` example in these pages is run by the test suite, and its printed output is checked.
