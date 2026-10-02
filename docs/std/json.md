# std.data.json

A JSON value tree, a strict parser and a printer. `import std.data.json as json`.

```mote,skip
pub enum Json {
    Null
    Bool(Bool)
    Int(Int)
    Float(Float)
    Str(String)
    Array(List<Json>)
    Object(List<Member>)
}
```

An `Object` keeps its members in source order and may repeat a key; build a `Member` with `json.member(key, value)`. `1` is an `Int` and `1.0` a `Float`.

| Function | Answers |
|---|---|
| `parse(text)` | `Result` of a `Json`; strict RFC 8259, depth limited to 256, errors name line and column |
| `to_text(j)`, `pretty(j, indent)` | `Result` of compact or indented text; `Err` for a `NaN` or infinite `Float` |
| `is_null`, `as_bool`, `as_int`, `as_float`, `as_str`, `as_array`, `as_object` | the payload as a `T?` |
| `get(j, key)`, `at(j, index)` | the last member named `key`, or the element, as a `T?` |

```mote
import std.data.json as json

match json.parse("{\"name\": \"mote\", \"tags\": [1, 2]}") {
    Ok(doc) => println(json.as_str(json.get(doc, "name")!)!)
    Err(e) => println(e.message)
}
```

```output
mote
```

## Records

`@derive(Json)` on a struct, class or enum adds `fn to_json(self) -> Json` and a static `fn from_json(j: Json) -> Result<T, Error>`.

```mote
import std.data.json as json

@derive(Json)
struct Address {
    city: String
    zip: Int
}

@derive(Json)
class Person {
    name: String
    age: Int?
    homes: List<Address>
}

let ann = Person { name: "Ann", age: null, homes: [Address { city: "Oslo", zip: 150 }] }
let text = json.to_text(ann.to_json())!
println(text)

match Person.from_json(json.parse(text)!) {
    Ok(p) => println(p.homes.get(0).city)
    Err(e) => println(e.message)
}

let bad = "{\"name\": \"Bo\", \"age\": 7, \"homes\": [{\"city\": \"Rome\", \"zip\": \"x\"}]}"
match Person.from_json(json.parse(bad)!) {
    Ok(p) => println("read")
    Err(e) => println(e.message)
}
```

```output
{"name":"Ann","age":null,"homes":[{"city":"Oslo","zip":150}]}
Oslo
homes[0].zip: expected an int, found a string
```

A struct or class is an object with one key per field. Reading ignores unknown keys; a missing key is an error except for a `T?` field.

| Field type | Written as |
|---|---|
| `Int`, `Float`, `Bool`, `String` | the matching `Json` case; a `Float` also reads an `Int` |
| `T?` | `null` or `T`'s form |
| `List<T>`, `Set<T>` | `Array` |
| `Map<String, T>` | `Object` |
| `(A, B)` | `Array` of that length |
| `A \| B` | by the member the value has; reading tries each in turn |
| `Json` | itself |
| another type | its own `to_json` and `from_json` |

Any other field type (`Char`, `Bytes`, a function, a `Map` keyed by anything but `String`) is a compile error, and so is deriving on a generic type.

An enum is externally tagged: `Red` is `"Red"`, `Circle(2)` is `{"Circle": 2}`, `Pair(7, "x")` is `{"Pair": [7, "x"]}`, and `Rect { w: 2, h: 3 }` is `{"Rect": {"w": 2, "h": 3}}`. A failed read has kind `InvalidData` and names the path from the root, such as `homes[1].zip`.
