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
| Lexer, parser, type checker, code generator | `motec/crates/compiler`, `motec/crates/modules` |
| Instruction set and value layout | `motec/crates/isa` |
| Register VM, task scheduler | `motec/crates/runtime` |
| Mark-sweep garbage collector | `motec/crates/gc` |
| Native calls, I/O, networking, clocks | `motec/crates/ffi`, `motec/crates/platform` |
| Packages, lockfile, registry client, bundler | `motec/crates/pkg` |
| The `mote` command | `motec/crates/cli` |
| Standard library, written in Mote | `motec/crates/modules/std` |
| Tree-sitter grammar and Zed extension | `editors/` |

The language has structs, classes, enums, generics with trait bounds, optionals and `Result`, closures, generators, modules, and tasks with channels. Types are checked at compile time; `Any` exists as an explicit escape hatch. Memory is managed by the compiler and a tracing collector.

The standard library covers collections, strings, regex, dates, JSON/TOML/YAML, compression, archives, hashing, SQLite, files, sockets, TLS, an HTTP client and server, logging and argument parsing. Several of these wrap Rust crates.

## Building

Rust (2024 edition) is required.

```sh
cd motec
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
