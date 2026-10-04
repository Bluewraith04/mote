# Changelog

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
