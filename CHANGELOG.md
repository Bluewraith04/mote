# Changelog

## 0.2.1

* Generics on concrete methods (`obj.method<T>()`, static methods, and structs/classes).
* Operator overloading with statically analyzed, auto-inlined dunder methods, reverse dunder fallback, compound assignment laddering, and post-monomorphization `__eq__`/`__ne__` dispatch on generic `T: Eq`.
* Sized array (`[T; N]`) element-wise / SIMD operations, random array initializers, and `std.array` helpers.
* Universal builtin types (`List`, `Array`, `Map`, `Set`, `Range`, `Stream`, `Option`, `Result`, `Error`, `ErrorKind`, `Pointer`, `Task`, `Sender`, `Receiver`, `Mutex`, `Once`, `PortEnd`, `AtomicInt`, `AtomicBool`), `Any`, `comptime`, const generics, `@` builtins (`@type_info`, `@type_name`, `@size_of`, `@align_of`, `@is_flat`, `@typeof`), `extend` blocks, and `extern` declarations.
* Consolidated `mote_std/` core libraries alongside `std`, and full `x86_64-windows` (`x86_64-pc-windows-msvc`) toolchain and native support.
* Path-first imports (`import path.{ a, b }`, `import path.*`).
* Direct single-threaded runtime fast path when no `spawn`, channels, ports, locks, sleep, or blocking externs are reachable.
* Off-heap explicit memory helpers in `std.mem` (`Buffer<T>`, `Pointer<T>` allocation/access guarded by `@is_flat`).
* Restructured zero-copy views (`StrView` in `std.string`, `Span<T>`/`MutSpan<T>` in `std.collections`, zero-copy slicing on `String`/`[T]`/`List<T>`, and Owner-to-View coercions).
* Upgraded Zed extension highlighting (`0.2.1`), outline, and interpolation overrides.

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
