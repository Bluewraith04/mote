# Changelog

## 0.1.3

The last 0.1.x release: a package registry, compile-time modifiers, a flat standard library, core libraries fetched by name, task, terminal and signal support, and Linux as the one platform.

### Platforms

- Linux on x86_64 and aarch64 is the supported platform. Windows and macOS are not planned, tested or documented.

### Packages and tools

- **A registry is a second source beside git**. `shapes = "^1.2"` resolves through a named registry's sparse index to an immutable, checksummed `.mpk` archive; `mote add shapes` writes `^X.Y.Z` for the highest release and `mote add shapes@1.2` writes `^1.2`. The lock pins the version and the archive's checksum; the index and archives are kept under `~/.mote/registry`, so a package fetched once syncs again offline.
- `mote package --add-registry NAME URL` (`--default`, `--local`) and `--remove-registry NAME` name registries in `~/.mote/mote_config.toml`, or in `[registries]` of `mote.toml`. A URL is `https://`, or `http://` for `localhost`; a token goes in the config file (kept mode 600) or `MOTE_REGISTRY_TOKEN`, and a token in `mote.toml` is an error. No registry is built in.
- `mote publish [--registry NAME]` uploads a package that has `description` and `license` in `[package]`, with its `README.md` and every `native/<triple>/`, after building the unpacked archive. `mote package` packs the README and native libraries too.
- A registry server written in Mote, for the owner to run behind a proxy: one token, a database of versions, two HTML pages, yank from the command line.
- Native libraries have no gates. `--allow-native` and `MOTE_ALLOW_NATIVE` are gone, `native = true` is no longer needed (an older manifest that says it still reads), and `open_package` opens the libraries of any installed package. A bound C function may take up to 12 parameters, up from 6.

### Core libraries

- `pane` (windows), `tile` (terminal screens), `wire` (HTTP and WebSocket servers and clients), `sqlite`, `compress`, `archive`, `log` and `path` live in `core/` and are imported by name: `import wire`, with no entry in `mote.toml`.
- A core library with a native part needs the core component for this version and machine: `mote core fetch` downloads it into the mote home, the first import fetches it when it is missing, and `mote core list` shows where each library is.

### Breaking: a smaller prelude

- `Display`, `Debug`, `Eq`, `Ord` and `Closeable` are no longer in the prelude: `import { Display, Eq } from std.traits`. A use without the import names the line to add.
- `Set`, `Stream` and `Task` need `import { Set } from std.collections`, `import { Stream } from std.stream` and `import { Task } from std.task` (the module imported in any form also works). A use without the import names the line to add.

### Breaking: a flat standard library

- The groups `std.sys`, `std.data`, `std.dev` and `std.net` are gone: every module is `std.<name>`, such as `std.fs`, `std.json`, `std.args`, `std.socket`, `std.tls` and `std.http_server`. An old path is an error that names the new one (`unknown standard-library module 'std.data.json': did you mean 'std.json'?`).

### Breaking: `@derive` is gone

- `@derive(Display, Debug)` becomes a trait list: `import { Display, Debug } from std.traits`, then `struct P: (Display, Debug)`. `@derive(Json)` becomes `: (JsonCodec)` with `import { JsonCodec } from std.json`, and `@derive(Args)` becomes `: (Args)` with `import { Args } from std.args`. A type that already writes one of the trait's methods keeps it, and the trait is not generated.
- `@derive` is a compile error that says what to write instead.
- `@arg` on a type that lists no trait reading it is an error, as before.

### Breaking: `Args` reads the type, not tags on its fields

- A field is a flag, an option or a positional by its type and its default: `Bool` is a flag; a field with `= value` (`jobs: Int = 8`, `out: String? = null`, `xs: List<String> = []`) is an option; a field with no default is a positional (`List<T>` takes the rest). A required option can no longer be written (use a positional), and `missing option --x` is gone.
- Help is the `///` comment of a field, a command or the type. Short flags are written before the struct, class or command: `@arg(F = fixed, j = jobs)`.
- An attribute before a field or a field of a variant is a compile error, and the `@arg` keys `help`, `short`, `default` and `positional` are gone; the error says what replaces each.
- A flag with `= true`, a short that names no field, a short used twice, the short `h` and a short on a positional are compile errors.

### Added

- `///` doc comments are kept: the text before a struct, class, enum, variant or field is that item's `doc`, which `std.meta` hands a modifier as `Decl.doc`, `Field.doc` and `Variant.doc`. `//`, `/* */` and `////` are still plain comments.
- A field may have a default: `name: Type = expression` on a struct, class or enum-variant field. A literal may leave the field out and the expression is evaluated for it each time; `Field.default` in `std.meta` is its source text. An attribute value may be a bare name as well as a string.

- `mote expand [file]` prints a program with the methods its listed traits generate written into the types. The methods are written in a pass of their own after parsing, behind one interface and one attribute registry.
- `std.meta` and `@modifier`: the `Decl` a compile-time modifier reads and the `Additions` it answers, as plain Mote types. A `@modifier pub fn` from `Decl` to `Result<Additions, Diagnostic>` is checked where it is written and is an ordinary function, so a `test` block can call it.
- `@generate(path)` on a trait names the Mote modifier that writes its methods; a type that lists the trait (`struct P: (Display)`) and writes none of them gets the generated ones, and `mote expand` shows them. `Display` and `Debug` are Mote modifiers in `std.derive`. Modifiers run at compile time in a sandbox with step, memory and output limits, and their answers are cached under the build cache.
- `JsonCodec` (`std.json`) and `Args` (`std.args`) are traits too: `struct P: (JsonCodec, Args)` writes `to_json`/`from_json` and `parse`/`usage`, and `@derive(Json)`/`@derive(Args)` run the same Mote modifiers (`std.derive.json`, `std.derive.args`), which replace about 700 lines of Rust. `@modifier(reads = "arg")` names the member attributes a modifier consumes; `@arg` without a trait that reads it is an error. Errors from a modifier point at the listed trait and name the field in the message.
- A package can define a trait and its modifier, and another package lists the trait and gets the methods; a new [Modifiers](docs/modifiers.md) chapter writes one. The types in `std.meta` and the four built-in modifiers are `@stable`, and the stable-surface snapshot marks a `@modifier` function as `[modifier]`.
- `Error.context(what)` and `std.error.context(result, what)` put what a caller was doing in front of a failure's message (`loading config: read_file: missing.txt`) and keep its kind. `Error.code` and `with_code`, four more `ErrorKind`s, and a `?` checked against the function's `Result` type.
- `std.task` gains `Semaphore`, `timeout(d, f)` and `par_map(items, n, f)`; `all` cancels the other tasks after the first `Err`. Enums, tuples and `Result` are Sendable when their payloads are.
- `std.term` reads the terminal: raw mode, size, keys, mouse and resize events, and `width` and `glyphs` that count wide characters as two columns. `std.signal.watch` delivers operating system signals as a stream.
- `std.regex` runs on the Rust `regex` crate: code points, flags, linear time and `find_from`.
- `std.fs` gains `read_dir` (name, kind, size and modified time of every entry in one request), `lstat`, `read_link`, `canonicalize`, `symlink`, `set_modified` and `set_mode`; `Stat` and `Entry` carry mode, inode, device and link count.
- Slices: `xs[a..b]` copies a range of a `List`, `String` or `Bytes`, `xs[a..b] = v` assigns into a list in place, and `xs[a..b..s]` takes every `s`th item of a list.
- `std.json` quoting and scanning and `std.string.join` are native, an offloaded call hands off in 15 µs instead of 127 µs, and `File.read_to_end` reads 64 KiB at a time.

### Fixes

- **parser:** `x = f()?` at the end of a line read the `?` as a ternary when a `:` followed within the next lines, as in `var v: T`; the lookahead now stops at `let`, `var`, `return`, `for` and `while`.
- **runtime:** `cancel()` ended nothing for a task in `sleep`, `accept`, a socket or UDP read, so a `scope` holding an accept loop never closed and `task.any` waited for a sleeping loser; a cancelled task now leaves the wait at once. A task in a file, process or connect request still ends with that request.
- **runtime:** a task spawned into a faulted scope starts cancelled, a cancel is not a scope fault, and a detached task cancelled while it waits ends quietly instead of printing an error.
- **runtime:** a bounded regex repeat such as `[a-z0-9_]{1,39}` took exponential time when the match failed; it compiles to nested optionals and takes linear time.

## 0.1.2

Named arguments, an explicit glob import, rich text and a multi-line editor for windows, a markdown package, a mote home that caches packages, and a round of compiler and runtime fixes.

### Breaking changes

- **A plain import is a namespace.** `import .m` and `import pane` bring `m.f()` and `pane.f()`, no longer every name bare. Write `import { * } from m` for the old behaviour, or import the names you use. `pub import { * } from m` re-exports every public name.
- **pane's `run*` and `mount*` functions are replaced by the `App` builder**: `pane.App(...)` with `Handle` for shared state, a required input setter, component-local state, `every` and `on_key`, and headless test helpers (`click`, `type`, `press`).
- **pane's title bar presets `flat`, `windows` and `mac` are removed.** The bar is built from theme tones; `slate` and `banner` are the new presets, and a `Theme` sets `text_size` and `spacing`.
- **A function with defaults or a variadic parameter used as a value has its full arity** (a variadic parameter is a `List`). It was typed `Any`, so a short call ran with an argument missing; it is now a checker error.

### Language

- Named arguments: `f(a, width = 3)`, after the positional ones and in any order. A named argument may skip the defaulted parameters before it. Functions, methods and constructors take names.
- `import { * } from m`, as above.
- A function stored in a field is called as `obj.f(x)`, and `(obj.f)(x)` works too.
- Lexer and parser errors say what was expected and what was found, name the block's owner and show the file and source line.
- The tree-sitter grammar and the Zed highlights know named arguments and `{ * }`.

### Windows

- `std.sys.gui` rich text: runs with weight, italic, monospace, underline, colour, strike and links on text and button nodes; a `Link` event; a bundled DejaVu Sans Mono.
- `TextArea`: wrapped multi-line editing with caret movement by line, scroll, undo and redo.
- `Window.set_maximized`, `set_fullscreen` and `fullscreen`.
- A scroll frame of a large document drops from 120–230 ms to 20 ms, and a 14 KB text area paints in 45 ms instead of 850 ms.

### pane

- `rich`, `on_link`, `input_multi`, `Element.reveal`, `memo` (an element rebuilt only when its token changes), `App.maximized` and `App.fullscreen`, and the roles `Code`, `Rule` and `Cell`.

### Packages and tools

- `markdown`: a package over `pulldown-cmark` that answers typed blocks and spans, with a `view` for pane (outline, `render_at`, images).
- `[package] mote = "0.1.2"` names the oldest Mote that builds a package. An older toolchain stops with a message that names both versions. `mote init` writes the current one.
- The mote home: `mote sync` fetches what `mote.toml` lists, `mote install` builds a program into `~/.mote/bin` with its native libraries, and fetched packages are cached under `~/.mote/git`, so a package fetched once syncs again offline.
- `mote test` runs a package's `tests/` files. `mote run -- args` passes arguments to a package's program.
- The tests of the language no longer import packages; each package carries its own.

### Performance

- Slicing, measuring, comparing and hashing a string read it in place instead of copying it. Parsing 230 KB of JSON drops from 5.2 s to 0.66 s.

### Fixes

- **compiler:** a lambda inside a nested generic call, such as `xs.push(el(|v, t| …))`, takes its parameter types from the collection instead of failing in code generation.
- **compiler:** a method call on a lambda parameter whose type nothing fixes is an error that says to write the type, not an internal one.
- **compiler:** calling a function stored in a field failed with "no method".
- **compiler:** a function with defaults or a variadic parameter used as a value was typed `Any`.
- **runtime:** closing a listener, or a UDP socket, wakes the tasks waiting on it; a pending `accept` returns an error instead of waiting forever.
- **runtime:** a child task finishing under a running parent no longer leaves a stale wake that wakes the parent's next, unrelated wait.
- **runtime:** a cancelled task passes on its wake at `send`, `recv` and shared begin and drops its waiter entries; a scope exit is woken only by its last child.
- **runtime:** an unobserved child fault at a scope's close built an error of the wrong type, which crashed a function returning `Result<_, Error>`; it now builds an `Error`, and a function returning an optional panics. A scope in a function returning a bare `Result` builds an `Err` of a `String`.

## 0.1.1

A window library, a UI package on top of it, packages from any git repository, and much smaller programs.

### Windows

- `std.sys.gui`: a retained tree of boxes, text, images, buttons and inputs, laid out with flexbox and grid (taffy) and painted on the CPU (tiny-skia, cosmic-text, a bundled DejaVu Sans). Hover and pressed styles, clicks, wheel scrolling, a single-line text editor, and an event stream.
- Headless windows (`gui.headless`) whose pixels read back with `pixel` and `png`, so every behaviour is tested with no screen.
- Windows on a display through winit and softbuffer. Under WSL they open through X11 so Windows draws the frame; a lost display closes the windows and a missing system library is an error that names it.
- Window frame calls: `set_frame`, `drag`, `drag_resize`, `minimize`, `toggle_maximize`, `maximized`, `set_title`, and `move_before` to reorder siblings.
- `std.task.pin()` moves a task to the main thread, which a window needs.

### pane

A package that describes a window as a function of your state. It is written in Mote over `std.sys.gui`.

- Elements (`vstack`, `hstack`, `box`, `scroll`, `text`, `h1` to `h6`, `button`, `input`), modifiers, and handlers as lambdas over one state class. A redraw touches only the nodes that differ.
- Components imported by name: `nav`, `tabs`, `pages`, `checkbox`, `select`, `image`.
- Themes (`light`, `dark`, `contrast`) set once for a window, keys for reordering lists, and state shared with other tasks through a `Shared` cell.
- A title bar pane draws, in four presets: `system`, `flat`, `windows`, `mac`.

### Packages

- A dependency is any git repository (https, ssh, a local path, `github:user/repo`) or a path. `mote add <repository>[@ref]` records it, and `mote.lock` pins the commit and a checksum of its `src/`. The registry and `mote publish` are gone.
- A package may carry native libraries in `native/<triple>/`, granted per dependency with `native = true`. `libtools.open_package` opens them, and `mote build` copies them beside the program.
- `import package.module` reads another file of a package.
- Moved out of the standard library into packages that wrap Rust crates: TOML, YAML, compression, archives, SQLite and the HTTP client, along with `log` and `path`. Importing an old path says which package to add.

### Size

- `mote build` attaches a program to the smallest runtime that covers the natives it uses: 1.3 MB for a program with no heavy natives (it was 18.7 MB), 2.5 MB with TLS, 8.0 MB with a window.

### Fixes and changes

- A glob import no longer clashes with the implicit prelude, and a name imported through a facade and its own module is one name.
- A local captured by a lambda wins over a function of the same name.
- `mote run` no longer prints a collector summary; `--mem-stats` reports it.
- Unused catch-all match arms use `_`.

## 0.1.0

The first public release: the compiler, the register VM, the collector, tasks and channels, and the standard library.
