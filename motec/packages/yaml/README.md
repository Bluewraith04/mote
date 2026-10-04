# yaml

YAML, read into and written from a `Json` value by the Rust `serde_yaml_ng` crate. The package carries the crate as a library in `native/<triple>/`, so add it with native access granted, then `import yaml as yaml`.

```toml
[dependencies]
yaml = { git = "github:user/yaml", tag = "v0.1.0", native = true }
```

Libraries are built for `x86_64-unknown-linux-gnu` here. To build one for another machine: `cargo build --release --manifest-path rust/Cargo.toml`, then copy `rust/target/release/libmote_yaml.so` (`.dylib`, `mote_yaml.dll`) into `native/<triple>/`. The functions are `mote_yaml_parse` and `mote_yaml_render`, in the bytes-in, bytes-out shape of `std.dev.libtools`; a negative answer is the negated length of an error message written to the output buffer.

```mote,skip
import yaml as yaml

@derive(Json)
struct Server {
    host: String
    port: Int
}

let text = "host: localhost # the dev box\nport: 8080\n"
let server = Server.from_json(yaml.parse(text).unwrap()).unwrap()
println("${server.host}:${server.port}")
print(yaml.to_text(server.to_json()).unwrap())
```

```text
localhost:8080
host: localhost
port: 8080
```

A YAML file reads into the same value as JSON and TOML, so a type that derives `Json` reads from it with `T.from_json(yaml.parse(text)?)` and writes back with `yaml.to_text(x.to_json())`.

## Functions

| Function | Answers |
|---|---|
| `parse(text) -> Result<Json, Error>` | the one document in `text`; empty text is `Null`; more than one is an `Err` |
| `parse_all(text) -> Result<List<Json>, Error>` | one `Json` for each `---` document, in order |
| `to_text(j: Json) -> Result<String, Error>` | the value as a YAML document |

An `Err` has kind `InvalidData` and the crate's message, with the line and column.

## What a value reads as

| YAML | `Json` |
|---|---|
| mapping | `Object`, keys in document order |
| sequence | `Array` |
| string, integer, float, `true`/`false`, `null` or `~` | `Str`, `Int`, `Float`, `Bool`, `Null` |
| `.inf`, `-.inf`, `.nan` | `Str` of `"inf"`, `"-inf"`, `"nan"`, since JSON has no such number |
| anchor, alias, `<<` merge | expanded in place |
| tag such as `!custom` | dropped; the value stays |
| scalar key such as `1` or `true` | the key's text: `"1"`, `"true"` |

A key that is a list or mapping, and an integer past 64 bits, are errors. A `Json` cannot tell a date from a string, so a timestamp reads as text.

## Writing

The writer prints block style and quotes a string that would read back as something else, so `"123"` is written `'123'`.
