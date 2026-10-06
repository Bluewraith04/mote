# Mote

Mote is a small statically typed language with its own bytecode compiler and virtual machine, written in Rust. It is a hobby project: a mildly successful attempt to understand how programming languages work, by building one end to end.

```mote
struct Job {
    name: String
    weight: Int
}

fn heaviest(jobs: List<Job>) -> Job {
    var best = jobs.get(0)
    for j in jobs {
        if j.weight > best.weight {
            best = j
        }
    }
    return best
}

fn main() {
    let jobs = [Job { name: "a", weight: 3 }, Job { name: "b", weight: 8 }]
    let top = heaviest(jobs)
    println("heaviest: ${top.name}")

    let h = spawn { return top.weight * 2 }
    println(h.join()!)
}
```

## What is in it

| Part | Where |
|---|---|
| Lexer, parser, type checker, code generator | `mote/crates/compiler`, `mote/crates/modules` |
| Instruction set and value layout | `mote/crates/isa` |
| Register VM, task scheduler | `mote/crates/runtime` |
| Mark-sweep garbage collector | `mote/crates/gc` |
| Native calls, I/O, networking, clocks | `mote/crates/ffi`, `mote/crates/platform` |
| Packages from git or a path, lockfile, bundler | `mote/crates/pkg` |
| The `mote` command | `mote/crates/cli` |
| Standard library, written in Mote | `mote/crates/modules/std` |
| Window and drawing library | `mote/crates/gui` |
| Tree-sitter grammar and Zed extension | `editors/` |

The language has structs, classes, enums, generics with trait bounds, optionals and `Result`, closures, generators, modules, and tasks with channels. Types are checked at compile time; `Any` exists as an explicit escape hatch. Memory is managed by the compiler and a tracing collector.

The standard library covers collections, strings, regex, dates, JSON, hashing, files, sockets, TLS, an HTTP server, argument parsing and a window with flexbox layout (`std.sys.gui`). TOML, YAML, compression, archives, SQLite and an HTTP client are packages that wrap Rust crates and carry their own native library; `pane` is a package for describing a window as a function of your state. See [CHANGELOG.md](CHANGELOG.md) for what changed in each release.

## Building

Rust (2024 edition) is required.

```sh
cd mote
cargo build --release
cargo test --workspace
cargo install --path crates/cli
```

```sh
mote run hello.mote
mote check hello.mote
mote test
```

## Documentation

[`docs/`](docs/README.md) has a language manual, a short reference, and one page per standard-library module. Every example in it is run by the test suite.

## Limits

- It is a learning project, not a production language. Expect rough edges and no stability promise.
- There is no debugger and no language server.
- Development and testing happen on Linux; other platforms are untested.
- `if` and `match` are statements, not expressions.
- The collector stops the world and has no generations.

## Licence

MIT. See [LICENSE](LICENSE).
