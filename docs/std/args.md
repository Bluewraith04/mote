# Command-line arguments

`@derive(Args)` on a struct, class or enum adds a command-line parser, its error messages and its help text. No import is needed. `env.args()` gives the raw list ([env](env.md)).

```mote,skip
import std.sys.env

@derive(Args)
@arg(help = "Copies files")
class Options {
    @arg(help = "print each file", short = "v")
    verbose: Bool
    @arg(help = "where to put them")
    out: String?
    @arg(default = "3", help = "how many copies")
    count: Int
    @arg(positional, help = "files to copy")
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

| Field | Command line | Absent |
|---|---|---|
| `Bool` | the flag `--name` | `false` |
| `Int`, `Float`, `String` | `--name value` or `--name=value` | an error, unless `default` is given |
| `T?` | the same | `null` |
| `List<T>` | the option repeated | `[]` |
| with `@arg(positional)` | a bare argument, in declaration order | an error for `T`, `null` for `T?`, `[]` for `List<T>` |
| a named type | a subcommand | `null` for `C?`, an error for `C` |

A field `out_dir` is the option `--out-dir`. `--` ends the options. `-o x`, `-o=x` and `-ox` read the option with `short = "o"`. A flag takes no value; combined short flags (`-vq`) are an error. The last of a repeated non-list option wins.

`@arg` keys: `help = "text"` (a field, variant or the type), `short = "o"`, `default = "3"`, and `positional`. `help` and `-h` are reserved.

An enum that derives `Args` is a set of commands: `Build` is `build` and `RunTests` is `run-tests`. A struct field of that type takes the first bare argument as the command name; what follows belongs to the command. A unit variant is a command with no arguments; a tuple variant, and a subcommand inside a subcommand, are errors.

```mote,skip
@derive(Args)
enum Command {
    @arg(help = "add up the numbers")
    Sum {
        @arg(positional, help = "the numbers")
        numbers: List<Int>
    }
    @arg(help = "repeat a word")
    Say {
        @arg(positional, help = "the word")
        word: String
        @arg(short = "n", default = "1", help = "how many times")
        times: Int
    }
}

@derive(Args)
@arg(help = "Small tools")
class Tool {
    @arg(short = "q", help = "print only the result")
    quiet: Bool
    command: Command
}
```

Errors read like `unknown option --x`, `option --x needs a value`, `missing option --x`, `missing argument <FILE>`, `invalid value for --count: expected an int, found "x"`, `unexpected argument "x"`, and `unknown command "x" (expected build, run)`.

Not included: fields read from text (a `Path`, an enum of choices), counted flags, `--no-flag`, environment fallbacks, validators, per-subcommand help, shell completions.
