# Getting started

A Mote program is a `.mote` file. The compiler turns it into bytecode and the `mote` command runs it.

```mote
fn main() {
    println("Hello, Mote")
}
```

```output
Hello, Mote
```

`main` is called for you. Statements at the top level of the file run first, so a short script needs no `main`. A run prints only what the program prints; the value `main` returns is not the exit code.

## Commands

Each command takes a file path, or none to use the package in the current directory.

| Command | What it does |
|---|---|
| `mote run main.mote` | compiles and runs |
| `mote check main.mote` | type-checks without running |
| `mote check --memory main.mote` | also reports where each struct lives ([Memory](memory.md)) |
| `mote compile [-o out]` | writes bytecode, by default `dist/<name>.mbc`; `mote file.mbc` runs it |
| `mote build [-o out]` | writes a standalone executable, by default `dist/<name>` |
| `mote test` | runs `test { }` blocks ([Tools](tools.md)) |
| `mote init myapp [--lib]` | creates a package, or a library with `--lib` |
| `mote add foo`, `mote remove foo` | adds or removes a dependency |
| `mote install [--locked]` | fetches dependencies |
| `mote package` | writes the package's `.mpk` archive to `dist/` |
| `mote version`, `mote help` | prints the version, or every command and flag |

Flags for `mote run`:

| Flag | Sets |
|---|---|
| `--max-heap 2GiB` | the heap limit |
| `--workers 4` | scheduler threads; default one per core |
| `--gc nogc` | turns the collector off; `--gc mark-sweep` is the default |
| `--release` | a release build, which leaves out the source text used in error messages |
| `--allow-native` | lets the program load C libraries ([libtools](std/libtools.md)) |
| `--mem-stats` | prints what the run used |

Arguments after the file name reach the program through `std.sys.env.args()`. The exit code is `0`, or `1` after a runtime error; `std.sys.env.exit(code)` picks another.

## Packages

A package is a directory with a `mote.toml`:

```toml
[package]
name    = "myapp"
version = "0.1.0"
entry   = "src/main.mote"

[dependencies]
foo = "^1.0.0"
bar = { path = "../bar" }
```

A dependency is a version requirement taken from a registry, or a `path`. `mote.lock` records a checksum for each; commit it and use `mote install --locked` in automated builds.
