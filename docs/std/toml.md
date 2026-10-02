# std.data.toml

TOML, read into and written from a [`Json`](json.md) value by the Rust `toml` crate. `import std.data.toml as toml`.

```mote
import std.data.toml as toml
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
let config = Config.from_json(toml.parse(text)!)!
println("${config.name} on port ${config.server.port}")
println(toml.to_text(config.to_json())!)
```

```output
demo on port 8080
name = "demo"

[server]
host = "localhost"
port = 8080

```

| Function | Answers |
|---|---|
| `parse(text) -> Result<Json, Error>` | the document as an `Object`, keys in document order |
| `to_text(j) -> Result<String, Error>` | the `Object` as a TOML document |

An `Err` has kind `InvalidData` and starts with the position, as in `line 2, column 1: duplicate key`.

| TOML | `Json` |
|---|---|
| string | `Str` |
| integer | `Int`; past 64 bits is an error |
| float | `Float`; `inf`, `-inf`, `nan` read as `Str` |
| boolean | `Bool` |
| date and time values | `Str` in standard form, since `Json` cannot tell a date from a string |
| array | `Array` |
| table, inline table | `Object` |
| `[[array of tables]]` | `Array` of `Object` |

Duplicate keys, extending a closed value, malformed numbers and dates, and text after a value are errors. The crate follows TOML 1.1.

Writing needs an `Object` at the top. In each table plain keys come first, then sub-tables. A `null` is an error, since TOML has none.
