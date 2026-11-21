# yaml: reference

YAML text to `Json` values and back. Native: a Rust crate. The value type is `json.Json`.

`import { parse, parse_all, load, to_text, to_text_all, dump } from yaml`

| Function | Notes |
|---|---|
| `parse(text) -> Result<Json, Error>` | the one document, `Null` for a text with none; more than one is an `Err` |
| `parse_all(text) -> Result<List<Json>, Error>` | every document of a stream |
| `load<T>(text) -> Result<T, Error>` | as `json.decode` reads a value |
| `to_text(value: Json) -> Result<String, Error>` | the YAML text |
| `to_text_all(values) -> Result<String, Error>` | documents each after a `---` line |
| `dump<T>(value) -> Result<String, Error>` | as `json.encode` reads a value |

An `Err` says the line and column of the first mistake. The rules for `T` are `json`'s (records by field name, optionals, lists, unit variants by name).

```mote
import { dump, load } from yaml

class Server {
    host: String
    ports: List<Int>
}

fn main() -> Result<Null, Error> {
    let server = load<Server>("host: example.org\nports: [80, 443]\n")?
    println(server.ports.len())
    println(dump(server)?)
    return Ok(null)
}
```

Tests: `tools/test-core.sh yaml`.
