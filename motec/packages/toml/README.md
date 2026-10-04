# toml

TOML, read into and written from a `Json` value by the Rust `toml` crate. The package carries the crate as a library in `native/<triple>/`, so add it with native access granted, then `import toml as toml`.

```toml
[dependencies]
toml = { git = "github:user/toml", tag = "v0.1.0", native = true }
```

Libraries are built for `x86_64-unknown-linux-gnu` here. To build one for another machine: `cargo build --release --manifest-path rust/Cargo.toml`, then copy `rust/target/release/libmote_toml.so` (`.dylib`, `mote_toml.dll`) into `native/<triple>/`. The functions are `mote_toml_parse` and `mote_toml_render`, in the bytes-in, bytes-out shape of `std.dev.libtools`; a negative answer is the negated length of an error message written to the output buffer.

```mote,skip
import toml as toml
import std.data.json as json

@derive(Json)
struct Server {
    host: String
    port: Int
}

@derive(Json)
struct Config {
    name: String
    server: Server
}

let text = "name = \"demo\"\n\n[server]\nhost = \"localhost\"\nport = 8_080\n"
let config = Config.from_json(toml.parse(text).unwrap()).unwrap()
println("${config.name} on port ${config.server.port}")
println(toml.to_text(config.to_json()).unwrap())
```

```text
demo on port 8080
name = "demo"

[server]
host = "localhost"
port = 8080

```

A TOML file reads into the same value as JSON, so a type that derives `Json` reads from a config file with `T.from_json(toml.parse(text)?)` and writes back with `toml.to_text(x.to_json())`.

## Functions

| Function | Answers |
|---|---|
| `parse(text) -> Result<Json, Error>` | the document as a `Json` `Object`, keys in document order |
| `to_text(j: Json) -> Result<String, Error>` | the `Object` as a TOML document |

An `Err` has kind `InvalidData` and a message that starts with the position, as in `line 2, column 1: duplicate key`.

## What a value reads as

| TOML | `Json` |
|---|---|
| string: basic, literal, multi-line | `Str` |
| integer: decimal, `0x`, `0o`, `0b`, `_` between digits | `Int`; past the 64-bit range is an error |
| float | `Float` |
| `inf`, `-inf`, `nan` | `Str` of `"inf"`, `"-inf"`, `"nan"`, since JSON has no such number |
| `true`, `false` | `Bool` |
| offset date-time, local date-time, local date, local time | `Str`, the date in standard form (`T` between date and time) |
| array | `Array`, mixed types allowed |
| table, inline table | `Object` |
| `[[array of tables]]` | `Array` of `Object` |

A `Json` cannot tell a date from a string, so a date reads as its text and is written back as a string.

## What is refused

| Error | Example |
|---|---|
| a key defined twice | `a = 1` then `a = 2` |
| extending a closed value | `a = {x = 1}` then `[a.y]` |
| a bad number | `01`, `1__0`, `9223372036854775808` |
| a bad date | `2021-02-30` |
| a bad key | `[]` |
| text after a value | `x = 1 y = 2` |

The crate follows TOML 1.1, so newlines inside an inline table and a time without seconds (`12:30`) are read.

## Writing

The top level must be an `Object`. In each table the plain keys come first, then the sub-tables as `[a.b]` and the arrays whose elements are all objects as `[[a.b]]`. An array that mixes objects with other values writes its objects as inline tables. Keys of letters, digits, `_` and `-` are bare and the rest are quoted.

```mote,skip
import toml as toml
import std.data.json as json

let doc = json.parse("{\"b\": {\"c\": 1}, \"a\": [1, 2], \"f\": [{\"g\": 1}, {\"g\": 2}]}").unwrap()
println(toml.to_text(doc).unwrap())
```

```text
a = [1, 2]

[b]
c = 1

[[f]]
g = 1

[[f]]
g = 2

```

A `null` is an error naming its path, since TOML has none: `TOML has no null (at a.b[0])`. So is a top level that is not an object. A document read with `parse` and written again reads back as the same value, with plain keys moved ahead of tables, and without the comments.
