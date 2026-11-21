# toml: reference

TOML 1.0 text to values and back. Native: a Rust crate (`toml`). Uses `json` for the reflection rules.

`import { Toml, parse, encode, decode, ToToml, FromToml } from toml`

`Toml` is `Table(List<(String, Toml)>)`, `Array(List<Toml>)`, `Text`, `Integer`, `Real`, `Boolean`, `Datetime(String)` (the date or time as text, such as `1979-05-27T07:32:00Z`).

| Item | Notes |
|---|---|
| `parse(text) -> Result<Toml, Error>` | the `Err` says line and column of the first mistake |
| `.as_bool() .as_int() .as_float() .as_text() .as_datetime()` | optionals |
| `.at(index)`, `.get(key)` | element of an array, value of a table key |
| `.to_text() -> Result<String, Error>` | a table's text; `Err` for a value that is not a table |
| `.pretty()` | arrays of several items on lines of their own |
| `encode<T>(value) -> Result<Toml, Error>` | as `json.encode`; an optional field that is null is left out; a null anywhere else is an `Err` |
| `decode<T>(value) -> Result<T, Error>` | as `json.decode`; a date-time is read as text |
| `ToToml`, `FromToml` | traits for `to_toml()` / `from_toml(value)` |

```mote
import { decode, parse } from toml

struct Config {
    name: String
    port: Int
}

fn main() -> Result<Null, Error> {
    let config = decode<Config>(parse("name = \"mote\"\nport = 80\n")?)?
    println("${config.name}:${config.port}")
    return Ok(null)
}
```

Tests: `tools/test-core.sh toml`.
