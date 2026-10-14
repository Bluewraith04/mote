# Command-line arguments

Listing `Args` on a struct, class or enum, `struct Options: (Args)` with `import { Args } from std.args`, writes a command-line parser, its error messages and its help text. `env.args()` gives the raw list ([env](env.md)). Each field is read by its type and its default, and a `///` comment on a field, a command or the type is its help.

```mote,skip
import std.env
import { Args } from std.args

/// Copies files
@arg(v = verbose)
class Options: (Args) {
    /// print each file
    verbose: Bool
    /// where to put them
    out: String? = null
    /// how many copies
    count: Int = 3
    /// files to copy
    files: List<String>
}

fn main() {
    let o = Options.parse_or_exit(env.args())
    println("${o.files} -> ${o.out}, ${o.count} copies")
}
```

```text
$ copy -v --out=backup --count 2 a.txt b.txt
["a.txt", "b.txt"] -> backup, 2 copies
```

A type that holds a `List` is a class.

| Method | Answers |
|---|---|
| `T.parse(argv)` | `Result<T, Error>`; `argv[0]` is the program |
| `T.parse_or_exit(argv)` | `--help` prints the help and exits 0; a bad command line prints the error and usage to stderr and exits 2 |
| `T.usage(program)` | the help text |

A flag, else an option if the field has a default, else a positional.

| Field | Reads as | Absent |
|---|---|---|
| `Bool` | the flag `--name` | `false` |
| `Int`, `Float` or `String` with `= value` | `--name value` or `--name=value` | the value |
| `T?` with `= null` | the same | `null` |
| `List<T>` with `= []` | the option repeated | `[]` |
| `T` with no default | a bare argument, in declaration order | an error |
| `T?` with no default | an optional bare argument | `null` |
| `List<T>` with no default | every bare argument left, so it is the last positional | `[]` |
| a named type | a subcommand | `null` for `C?`, an error for `C` |

A field `out_dir` is the option `--out-dir`. `--` ends the options. `-o x`, `-o=x` and `-ox` read the option with the short `o`. A flag takes no value; combined short flags (`-vq`) are an error. The last of a repeated non-list option wins. There is no required option: write it as a positional. A `Bool` with `= true` is an error.

`@arg(x = field, …)` before a struct, class or command gives `field` the short flag `-x`: a key is one ASCII letter and the value a field name. A short on a positional or a subcommand, a letter used twice, the letter `h` and a name that is no field are errors. `help` and `-h` are reserved. An attribute before a field is an error.

An enum that lists `Args` is a set of commands: `Build` is `build` and `RunTests` is `run-tests`. A struct field of that type takes the first bare argument as the command name; what follows belongs to the command. A unit variant is a command with no arguments; a tuple variant, and a subcommand inside a subcommand, are errors.

```mote,skip
enum Command: (Args) {
    /// add up the numbers
    Sum {
        /// the numbers
        numbers: List<Int>
    }
    /// repeat a word
    @arg(n = times)
    Say {
        /// the word
        word: String
        /// how many times
        times: Int = 1
    }
}

/// Small tools
@arg(q = quiet)
class Tool: (Args) {
    /// print only the result
    quiet: Bool
    command: Command
}
```

Errors read like `unknown option --x`, `option --x needs a value`, `missing argument <FILE>`, `invalid value for --count: expected an int, found "x"`, `unexpected argument "x"`, and `unknown command "x" (expected build, run)`.

Not included: fields read from text (a `Path`, an enum of choices), counted flags, `--no-flag`, environment fallbacks, validators, per-subcommand help, shell completions.
