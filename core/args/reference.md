# args: reference

Command-line parsing of a record type by reflection: flags from field names, help text from `///` docs. Pure Mote.

`import { parse_args, from_env, usage_text, help_requested, Args, Flag, flag, parse, usage } from args`

## The shape

A class (or struct) lists `Args`; each field is a flag named after it (`max_depth` is `--max-depth`).

| Field type | Flag |
|---|---|
| `Bool` | a switch: `--verbose` (also `--verbose=false`); absent is false |
| `Int` `Float` `String`, an enum of unit variants | takes a value: `--port 80` or `--port=80` or `-p 80`; the enum by kebab-case name (`SlowAndSteady` is `slow-and-steady`) |
| `T?` | may be left out |
| `List<T>` | repeats: `--include a --include b` |

The doc comment of a field is its help; the doc comment of the type is the usage text's description.

`fn flags() -> List<Flag>` (optional, in the type) adds what the fields do not say:

| `flag(field)` then | |
|---|---|
| `.short("p")` | `-p`; short switches group (`-vq`), a short value may be `-p80` or `-p=80` |
| `.help("text")` | replaces the field's doc |
| `.default_to("8080")` | the value, as text, when the flag is not given |
| `.positional()` | read from the words that are not flags, in field order; a `List` field takes the rest |

## Functions

| Function | Notes |
|---|---|
| `from_env<T: Args>() -> Result<T, Error>` | reads `std.io.args()` with `T.flags()` |
| `parse_args<T: Args>(words) -> Result<T, Error>` | from given words |
| `usage_text<T: Args>(program) -> Result<String, Error>` | synopsis, description, arguments, options with help and defaults, and `-h, --help` |
| `help_requested(words) -> Bool` | `--help` or `-h` before a `--` |
| `parse<T>(words, flags = [])`, `usage<T>(program, flags = [])` | the same for a type that does not list `Args` |

Rules: `--` ends the flags; a word like `-5` or `-1.5` is a value, not a flag; a flag given twice (not a list) is an `Err`. An `Err` names the flag and what was wrong (`unknown flag --x`, `--port needs a value`, `--port: expected an integer, not abc`, `missing --host`, `unexpected argument x`). A field type a flag cannot hold (a nested record, a map) is an `Err` at parse time.

```mote
import { Args, Flag, flag, from_env, help_requested, usage_text } from args
import { args } from std.io

/// Serves files from a directory.
class Serve: (Args) {
    /// The port to listen on.
    port: Int
    host: String
    verbose: Bool
    root: String

    fn flags() -> List<Flag> {
        return [
            flag("port").short("p").default_to("8080"),
            flag("host").default_to("localhost").help("The address to bind"),
            flag("verbose").short("v"),
            flag("root").positional()
        ]
    }
}

fn main() -> Result<Null, Error> {
    if help_requested(args().to_list()) {
        print(usage_text<Serve>("serve")?)
        return Ok(null)
    }
    let serve = from_env<Serve>()?
    println("${serve.host}:${serve.port} ${serve.root} ${serve.verbose}")
    return Ok(null)
}
```

Run it with `mote run -- ./site -p 9000 -v` (arguments after the package path as the CLI passes them; try `--help`).

Tests: `tools/test-core.sh args`.
