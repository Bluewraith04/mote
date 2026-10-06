# Changelog

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
