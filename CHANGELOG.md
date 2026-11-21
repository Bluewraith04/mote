# Changelog

## 0.2.0

A rewrite of the toolchain on Cranelift. Source written for 0.1 does not build unchanged.

* The compiler is split into crates: parser, checker, intermediate representation and code generation. `mote run` compiles in memory; `mote build` links an executable with the runtime.
* The runtime has a non-moving collector and a work-stealing scheduler for tasks.
* `std` and 25 core libraries are Mote packages. The core libraries are `archive`, `args`, `base64`, `compress`, `crypto`, `date`, `doc`, `env`, `fs`, `json`, `log`, `markdown`, `net`, `pane`, `path`, `process`, `random`, `regex`, `signal`, `sqlite`, `term`, `tile`, `toml`, `uuid` and `yaml`.
* The natives of the core libraries are built when `mote` is installed and embedded in it; no flag or script is needed to use them.
* `net` has sockets, TLS, an HTTP client and server and WebSocket. `Options` has defaults, and headers are `(name, value)` pairs.
* `pane` and `tile` build interfaces from elements and modifiers, for a window and for a terminal.
* A tree-sitter grammar and a Zed extension are in `editors/`.
* Releases carry a source asset with the built natives and a checksum.

Linux on x86_64 only.

## 0.1.3

The last release of the first toolchain, kept on its own tag.
