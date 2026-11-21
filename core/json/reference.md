# json: reference

JSON values, text, and encoding and decoding of Mote values by reflection. Pure Mote.

`import { Json, parse, encode, decode, ToJson, FromJson } from json`

## Json

`Json` is `Null`, `Boolean(Bool)`, `Integer(Int)`, `Real(Float)`, `Text(String)`, `Array(List<Json>)`, `Object(List<(String, Json)>)` (members keep their order).

| Method | Gives |
|---|---|
| `.is_null()`, `.as_bool()`, `.as_int()`, `.as_float()` (from either number), `.as_text()` | the value or null (`Bool?`, ...) |
| `.at(index) -> Json?` | element of an array |
| `.get(key) -> Json?` | first member of an object with the key |
| `.to_string()` | the compact text |
| `.pretty()` | one member or element per line, two-space indent |

`parse(text) -> Result<Json, Error>`: the `Err` names the place of the first mistake.

## By reflection

| Function | Rule |
|---|---|
| `encode<T>(value) -> Result<Json, Error>` | a struct or class is an object of its fields; a tuple, list, array or set an array; a map with `String` keys an object; an optional its value or null; a unit variant its name; any other variant an object with its name |
| `decode<T>(json) -> Result<T, Error>` | the reverse; a record takes its fields from the members by name; an optional field may be left out; the `Err` names the place of the first mismatch |
| `ToJson`, `FromJson` | traits that list `to_json()` / `from_json(json)`; the default bodies call `encode` / `decode` |

```mote
import { decode, encode, parse } from json

class User {
    name: String
    age: Int
    tags: List<String>
}

fn main() -> Result<Null, Error> {
    let user = decode<User>(parse("{\"name\": \"ada\", \"age\": 36, \"tags\": [\"x\"]}")?)?
    println(user.name)
    let json = encode(user)?
    println(json.pretty())
    return Ok(null)
}
```

`yaml` and `toml` use the same reflection (`yaml.load<T>`, `toml.decode<T>`), and `net.http` has `request.body_json()` and `Response.json(status, value)`.

Tests: `tools/test-core.sh json`.
