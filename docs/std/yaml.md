# std.data.yaml

YAML, read into and written from a [`Json`](json.md) value by the Rust `serde_yaml_ng` crate. `import std.data.yaml as yaml`.

```mote
import std.data.yaml as yaml

@derive(Json)
struct Server {
    host: String
    port: Int
}

let text = "host: localhost # the dev box\nport: 8080\n"
let server = Server.from_json(yaml.parse(text)!)!
println("${server.host}:${server.port}")
print(yaml.to_text(server.to_json())!)
```

```output
localhost:8080
host: localhost
port: 8080
```

| Function | Answers |
|---|---|
| `parse(text) -> Result<Json, Error>` | the one document in `text`; empty text is `Null`; more than one is an `Err` |
| `parse_all(text) -> Result<List<Json>, Error>` | one `Json` per `---` document |
| `to_text(j) -> Result<String, Error>` | the value as a YAML document |

An `Err` has kind `InvalidData` and carries the line and column.

Mappings read as `Object` in document order. Anchors, aliases and `<<` merges are expanded, tags are dropped, and a scalar key reads as its text. `.inf`, `-.inf` and `.nan` read as `Str`. A key that is a list or mapping, and an integer past 64 bits, are errors. The writer prints block style and quotes a string that would read back as something else.
