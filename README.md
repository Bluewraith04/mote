# Mote

Mote is a statically typed language that compiles to native code through Cranelift. It has classes and structs, enums with payloads, generics and traits, nullable types, `Result` for errors, and tasks (`spawn`, `scope`, channels) that run on a work-stealing scheduler with a non-moving collector.

```mote
fn main() {
    println("Hello, world!")
}
```

`mote run` compiles a package in memory and runs it. `mote build` links an executable, and `mote test` runs the `test` blocks of a package.

## Status

Version 0.2.0 is a rewrite of the 0.1 toolchain and is not compatible with it. It runs on Linux on x86_64 only.

## Requirements

* Rust (the stable toolchain; `rust-toolchain.toml` names it) to build `mote`.
* Zig, to build the external parts of the core libraries. Development used `0.18.0-dev.253+16d31ceed`.
* A C compiler (`cc`), `ld`, `objcopy` and `ar`, which `mote build` and the external library builds call.

## Install

```bash
./install.sh
```

`install.sh` builds and installs the `mote` binary, compiles `mote_std` shared libraries (or run `./compile-std.sh [name]`), and copies `mote_std/` to `~/.mote/mote_std/`.

```bash
mote init hello
cd hello
mote run
```

## Layout

| Folder | Holds |
|---|---|
| `syntax`, `semantics`, `ir`, `codegen` | the compiler: parser, checker, intermediate representation, Cranelift code generation |
| `abi`, `heap`, `scheduler`, `runtime` | value layout, the collector, tasks, and the functions compiled code calls |
| `packages`, `mote` | manifests and the module graph, and the `mote` command |
| `mote_std/std` | the standard library, written in Mote |
| `mote_std` | the core libraries alongside `std`: `archive`, `args`, `base64`, `compress`, `crypto`, `date`, `doc`, `env`, `fs`, `json`, `log`, `markdown`, `net`, `pane`, `path`, `process`, `random`, `regex`, `signal`, `sqlite`, `term`, `tile`, `toml`, `uuid`, `yaml` |
| `docs/reference` | the language reference |
| `editors` | a tree-sitter grammar and a Zed extension |
| `integration` | tests across the crates |

`mote_std/NAME/reference.md` describes one library. `tools/check.sh` runs every check of the repository.

## Editors

`editors/tree-sitter-mote` is the grammar and `editors/zed` the Zed extension; `editors/README.md` says how to use them.

## Releases

A release carries `mote-source-VERSION.tar.gz`, `mote_std` with its built shared libraries, and its `.sha256`. `mote run --std DIR --core DIR` uses a custom copy on disk instead of `~/.mote/mote_std/`.

## Licence

MIT. See `LICENSE`.
