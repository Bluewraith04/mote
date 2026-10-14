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
| `mote build [-o out]` | writes a standalone executable, by default `dist/<name>`; about 1.3 MB, 2.5 MB when the program uses TLS, 8 MB when it uses the GUI (libraries such as `sqlite` and `http` are copied beside it as `dist/<name>.lib`) |
| `mote test` | runs `test { }` blocks ([Tools](tools.md)) |
| `mote init myapp [--lib]` | creates a package, or a library with `--lib` |
| `mote add <git url>`, `mote remove foo` | adds or removes a dependency |
| `mote sync [--locked]` | fetches dependencies |
| `mote install [<dir> \| <git url>[@ref]]` | builds a program into `~/.mote/bin`, with its libraries |
| `mote package` | writes the package's `.mpk` archive to `dist/` |
| `mote publish` | uploads the package to a registry |
| `mote version`, `mote help` | prints the version, or every command and flag |

Flags for `mote run`:

| Flag | Sets |
|---|---|
| `--max-heap 2GiB` | the heap limit |
| `--workers 4` | scheduler threads; default one per core |
| `--gc nogc` | turns the collector off; `--gc mark-sweep` is the default |
| `--release` | a release build, which leaves out the source text used in error messages |
| `--mem-stats` | prints what the run used |

Arguments after the file name reach the program through `std.env.args()`. The exit code is `0`, or `1` after a runtime error; `std.env.exit(code)` picks another.

## Packages

A package is a directory with a `mote.toml`:

```toml
[package]
name    = "myapp"
version = "0.1.0"
entry   = "src/main.mote"
mote    = "0.1.2"

[dependencies]
ui  = { git = "github:someone/mote-ui", tag = "v1.2.0" }
bar = { path = "../bar" }
```

`mote` is the oldest Mote that builds the package; an older toolchain stops with a message that names it. A dependency is a git repository at a `tag`, `branch` or `rev`, or a `path`. A repository is an `https://`, `ssh://`, `git://` or `file://` URL, a `user@host:path` address, or `github:user/repo`; Mote reads it with the `git` command, so `git` must be installed and its credentials apply. `mote add <repository>[@ref]` adds one (the highest release tag without `@ref`), and `mote sync` fetches them. `mote.lock` records the commit and a checksum of the files for each; commit it and use `mote sync --locked` in automated builds. A release is a pushed tag. Fetched packages are cached under the mote home, `~/.mote` or `$MOTE_HOME`, so a package fetched once syncs again offline; `mote install` puts programs in its `bin/`.

A registry is a second source. `shapes = "^1.2"` takes the newest 1.x from a registry, and `mote add shapes` writes `shapes = "^X.Y.Z"` for the highest release (`mote add shapes@1.2` writes `^1.2`, `--registry NAME` picks one). No registry is built in: `mote package --add-registry NAME URL` names one in `~/.mote/mote_config.toml` (`--default` makes it the default, `--local` writes `mote.toml` instead, `--remove-registry NAME` forgets it). A URL is `https://`, or `http://` for `localhost` only. `mote publish` uploads the package with the token in `MOTE_REGISTRY_TOKEN` or under `[registries.NAME]` in the config file; a token in `mote.toml` is an error. A published package needs `description` and `license` in `[package]`, carries its `README.md` and every `native/<triple>/`, builds on its own when unpacked, can depend only on registry packages, and is never replaced; its owner can yank it, which hides it from new resolution and leaves it to a lock that has it. The lock pins the version and the archive's checksum, and the index and archives are cached under `~/.mote/registry` for offline syncs.

A package may carry compiled libraries in `native/<triple>/`, such as `native/x86_64-unknown-linux-gnu/libmote_toml.so`. Nothing grants them: `toml = { git = "github:someone/mote-toml", tag = "v0.1.0" }` is enough. `mote sync` unpacks only this machine's directory and the lock checksums each triple. The package opens its library with `open_package("toml", "mote_toml")` from `std.libtools`. `mote build` copies the libraries to `dist/<name>.lib/<package>/`; ship that directory beside the executable.

Libraries written only over the standard library are packages, kept outside the toolchain's repository and each with a README. Packages written so far include `pane` (windows as a function of your state, over `std.gui`), `log` (structured logging) and `path` (text operations on `/`-separated paths). `toml`, `yaml`, `compress`, `archive`, `sqlite` and `http` carry Rust libraries.
