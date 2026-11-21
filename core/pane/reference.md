# pane: reference

A window as a function of state. You write `view(state) -> Element`; handlers change the state; pane lays the view out (clay), shows the draw list (raylib, on a thread of its own) and runs the loop. Native: C (vendored clay and raylib 6.2, X11 through GLFW). Needs `doc` (view tree, native) and a display for a real window; tests use a headless window.

| Import | Gives |
|---|---|
| `import pane` | everything below as `pane.vstack(...)`, `pane.App(...)` |
| `import { KEY_ESCAPE, Window, Event } from pane` | names without the prefix |

## Run it yourself

```bash
mote run
```

Run it in a package whose `src/main.mote` is below. An installed `mote` carries the natives and unpacks `pane` and `doc` into `~/.mote/core/` the first time; nothing else to do. When editing the library in the repository, use `mote run --core <repo>/core` after `tools/build-core-natives.sh pane doc`. Needs `DISPLAY` (X11; WSLg's `:0` works). The window draws DejaVu Sans Mono (embedded; Latin, Latin-1, punctuation, arrows, box drawing and shapes; any other character is blank) at the sizes you ask for, one glyph per 0.6-of-size cell. To use another monospace font, replace `core/pane/native/vendor/font/DejaVuSansMono.ttf`, set `LINE_PER_EM` in `pane_window.c` to its (ascent + descent) / em, and rebuild.

```mote
import pane
import { KEY_ESCAPE } from pane

class Counter {
    var count: Int
}

fn view(s: Counter) -> pane.Element {
    return pane.vstack([
        pane.h1("${s.count}"),
        pane.hstack([
            pane.button("-").on_click(|var s: Counter| { s.count -= 1 }),
            pane.button("+").on_click(|var s: Counter| { s.count += 1 })
        ]),
        pane.text("Escape quits").color(pane.MUTED)
    ]).pad(24).gap(16).center().grow()
}

fn main() -> Result<Null, Error> {
    let last = pane.App("Counter", Counter { count: 0 }, view).size(360, 240).quit_on(KEY_ESCAPE).run()?
    println("final count ${last.count}")
    return Ok(null)
}
```

This program was compiled and its window opened under WSLg (it ran until killed with no error). Clicks and keys are verified headless (`tools/test-core.sh pane`), not by hand: a real click is the first thing to try.

What to look at by hand: the three buttons' areas follow the pointer (a click on the padding of a button counts, one on the gap between buttons does not); the window can be resized and the layout follows; Escape or the close button ends `run()` and prints the count; nothing changes on hover (no hover state yet).

## Elements

An `Element` is a class; every modifier answers a changed copy, so chains never change a shared element. `view` may build elements any way it likes (loops, `if`, helper functions).

| Constructor | Shows |
|---|---|
| `text(label)` | a line of text, 16 px, black |
| `h1(label)` `h2` `h3` `h4` | text 28, 24, 20, 18 px |
| `button(label)` | a box: padding 16 across and 8 down, `ACCENT` background, white text, radius 6, centred |
| `vstack(kids)` | `column`, children one under the other, gap 8 |
| `hstack(kids)` | `row`, gap 8, children centred down |
| `box(kids)` | a plain `Box` that lays its children out in a row, no gap |
| `spacer()` | empty space that grows along the parent's axis (and across it at the top) |

| Modifier | Sets | Notes |
|---|---|---|
| `pad(px)` / `pad_xy(across, down)` | space inside | |
| `margin(px)` | space outside | wraps the element in a padded box; a click in it misses the element |
| `gap(px)` | space between children | |
| `width(px)` / `height(px)` | fixed size | wins over `grow` on that axis |
| `grow()` | take the room the parent leaves | along the parent's axis (row: width, column: height); both axes at the top |
| `bg(color)` | background | `0xRRGGBBAA` |
| `color(color)` | text colour of the element and under it | inherited by text |
| `size(px)` | text height of the element and under it | inherited |
| `radius(px)` | corner radius | |
| `border(px, color)` | border | |
| `center()` | children centred across and down (a text: `align=center`) | |
| `middle()` | children centred down only | |
| `id(name)` | name the element | its area is `layout.area(name)`; two elements with one id are an `Err` |
| `on_click(f)` | handler | see below |

Colours and spacing helpers: `ACCENT 0x2563ebff`, `INK`, `MUTED`, `SURFACE`, `LINE`, `WHITE`, `GOOD`, `BAD`; `space(n)` is `n` steps of 4 px; `type_size(n)` is 16 px times 1.25 per step (`type_size(2)` = 25).

Missing on purpose (in `tasks.md` "Not planned"): `input`, `scroll`, hover and pressed colours, `disabled`, bold, `nav`, `tabs`, `select`, `checkbox`, `image`, `slider`.

## Handlers and the state

* `on_click(f)` takes a function over `var` state, `fn bump(var s: Counter)`, or a lambda that says the state type, `|var s: Counter| { s.count += 1 }`. The type has to be written in a lambda because the compiler cannot infer it through the chain.
* A click runs the handler of the last element in document order whose area holds the point and has a handler: the element under the pointer, or the nearest one around it. An element with a handler and no `id` gets `~0`, `~1`, ... in document order.
* After each event the view is built again from the state and shown. The state may be a class (shared) or a struct (copied in and written back); both are tested.
* A handler cannot fail or end the run; use `quit_on(key)`, `quit_when(done)` or a window close.

## App

| Item | Notes |
|---|---|
| `App(title, state, view)` | `view: (S) -> Element`; window 800 x 600 |
| `.size(width, height)` | pixels |
| `.on_key(code, f)` | `f: (var S) -> Null`; `code` is a raylib key code |
| `.quit_on(code)` | ends the run when the key goes down |
| `.quit_when(done)` | `done: (S) -> Bool`, checked after every event |
| `.run()` | opens a window, runs until it closes, closes it; gives `Result<S, Error>`, the last state |
| `.run_in(var window)` | runs in a window that is already open; a headless window ends the run when its events are used up |
| `lay_out(element, width, height)` | `-> Result<Layout, Error>`, no window |

Events it handles: `Click`, `Key`, `Close`. `Text`, `Wheel` and `Resized` are delivered by the window but ignored by `App` (a resize changes the size the next view is laid out in). Only one real window can be open at a time. `run()` is an `Err` that starts with `pane:` when the window cannot open.

Key constants: `KEY_SPACE 32`, `KEY_ESCAPE 256`, `KEY_ENTER 257`, `KEY_TAB 258`, `KEY_BACKSPACE 259`, `KEY_RIGHT 262`, `KEY_LEFT 263`, `KEY_DOWN 264`, `KEY_UP 265`. Other keys: the raylib code (`KEY_A` = 65 ... letters are upper-case ASCII, digits their ASCII).

## Testing without a screen

In a `tests/` file next to the `Counter` and `view` above (also `import { assert_eq } from std.test`); it follows `core/pane/tests/builders.mote`, which runs in `tools/test-core.sh pane -- builders`:

```mote
import { Window, Event } from pane

test "a click adds one" {
    var window = Window.headless(400, 300)
    let first = pane.lay_out(view(Counter { count: 0 }), 400, 300)!
    let plus = first.areas.get(1)
    window.inject(Event.Click(null, plus.x + plus.w / 2, plus.y + plus.h / 2, 0))
    let last = pane.App("Counter", Counter { count: 0 }, view).run_in(window)!
    assert_eq(last.count, 1)
}
```

`Window.headless(w, h)` keeps every layout shown in `window.frames` (a `List<Layout>`; `frames.get(0).draws` are the commands); `window.inject(event)` queues an event; the run ends when the queue is empty. `Layout.areas` has one `Area { id, x, y, w, h }` per element with an id (auto ids included), in document order.

## Layout, draw list, window (the layer under the builders)

`layout(node, width, height) -> Result<Layout, Error>` lays out a `doc` tree. The builders produce that tree; you can write one yourself.

| `doc` element | Attributes | |
|---|---|---|
| `row`, `column`, `box` (a row), `spacer`, `text` | `width`, `height` (`grow`, `fit`, pixels, or a percent up to 100%), `padding` (1, 2 or 4 numbers), `gap`, `align-x` (`left center right`), `align-y` (`top middle bottom`), `background`, `radius`, `border` (`width colour`), `clip` (`true`), `id`, `color`, `size`, `wrap` (`words`, `none`), `align`, `line-height` | a colour is `#rgb`, `#rrggbb`, `#rrggbbaa` or `black white red green blue gray transparent` |

* The root grows to the window; a `spacer` grows. A text is `size` px high and each glyph is 0.6 of that wide, so layouts are identical on every machine. An unknown element, a bad value or a duplicate id is an `Err` that names the element and the attribute.
* `Layout`: `width`, `height`, `draws: List<Draw>`, `areas: List<Area>`, `area(id) -> Area?`, `at(x, y) -> String?` (the id of the last area that holds the point).
* `Draw { kind, x, y, w, h, color, radius, border, text, size }`; `DrawKind` is `Rect Text Border Clip Unclip`, in paint order.
* `Window`: `open(w, h, title)`, `headless(w, h)`, `inject(event) -> Bool`, `present(layout)`, `next() -> Event?` (waits in a real window; null when closed or, headless, empty), `run(state, view, update)`, `run_routes(state, view, routes)`, `close()`, `width`, `height`, `last`, `frames`.
* `Event`: `Click(id: String?, x, y, button)`, `Key(code)`, `Text(codepoint)`, `Wheel(x, y, dx, dy)` (hundredths of a notch), `Resized(w, h)`, `Close`. `Next<S>` is `Continue(S)` or `Quit(S)`.

## Pipelines (declarative handlers)

`Step`: `Set(field, text)`, `Show(screen)` (sets the field `screen`), `Send(sender, text)`, `Await(receiver, field)`, `Quit`. `run_steps(state, steps) -> Result<Next<S>, Error>` runs them on a copy. A field takes text as `Bool`, `Int`, `Float`, `String` or the name of a unit variant of an enum. `Routes.new()` with `on_click(id, steps)`, `on_key(code, steps)`, `on_close(steps)`; `window.run_routes(state, view, routes)`. An `Await` blocks the loop until the reply; the window keeps drawing its last list.

## Limits

One font and no bold; no hover, text input or scrolling; one window; X11 only (no Wayland-native window); the draw list buffers grow as needed (first call sized for 256 commands). Layout and drawing are tested; the pixels on screen are only checked by eye.

## Tests

`tools/test-core.sh pane` (`-- builders`, `-- pipelines`, `-- window`, `-- layout`); `cargo test -p mote-integration --test core_tools -- core_pane`.
