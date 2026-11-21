# env: reference

Environment variables and the working directory. Native: Zig.

`import { get, get_or, has, set, remove, vars, current_dir, set_current_dir } from env`

| Function | Gives | Notes |
|---|---|---|
| `get(name) -> String?` | the value | null when unset or not UTF-8 |
| `get_or(name, fallback) -> String` | the value or the fallback | |
| `has(name) -> Bool` | set, even to "" | |
| `set(name, value) -> Result<Null, Error>` | | for this process and the programs it starts; `Err` for an empty name or one with `=` |
| `remove(name) -> Result<Null, Error>` | | an unset variable is not an error |
| `vars() -> List<(String, String)>` | every variable, in the order the process holds them | non-UTF-8 ones left out |
| `current_dir() -> Result<String, Error>` | the working directory | |
| `set_current_dir(path) -> Result<Null, Error>` | | process wide |

Program arguments are `std.io.args()`, not here.

Tests: `tools/test-core.sh env`.
