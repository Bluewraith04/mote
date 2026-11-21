# tile: reference

A terminal UI as a function of state. You write `view(state) -> Element`; handlers change the state; tile lays the view out on a grid of cells, writes only the cells that differ, and runs the loop over keys, the mouse, a timer and resizes. Pure Mote over `term` (needs `term`'s native, nothing of its own).

| Import | Gives |
|---|---|
| `import tile` | `tile.vstack(...)`, `tile.App(...)` |
| `import { Color } from term` | the colours for `fg` and `bg` |

## Run it yourself

```bash
mote run
```

(An installed `mote` carries the natives; from the repository's sources use `--core` as in `pane`'s page.) Run it in a package whose `src/main.mote` is below, from a real terminal (it takes the alternate screen, hides the cursor and, by default, reports the mouse; `q` quits and puts everything back). Verified: it compiles, and on a pseudo-terminal it draws the bordered counter with the reversed ` - ` and ` + ` buttons. Clicks and keys are verified through a pseudo-terminal in `tools/test-core.sh tile -- builders`; trying them by hand is the first thing to do.

```mote
import tile
import { Color } from term

class Counter {
    var count: Int
}

fn bump(var s: Counter) { s.count += 1 }

fn view(s: Counter) -> tile.Element {
    return tile.vstack([
        tile.text("count ${s.count}").bold().fg(Color.Green),
        tile.hstack([
            tile.button("-").on_click(|var s: Counter| { s.count -= 1 }),
            tile.button("+").on_click(bump)
        ]).gap(1),
        tile.spacer(),
        tile.text("j/k change, q quits").dim()
    ]).pad(1).border("counter")
}

fn main() -> Result<Null, Error> {
    let last = tile.App(Counter { count: 0 }, view).on_key("j", bump).on_key("k", |var s| { s.count -= 1 }).quit_on("q").run()?
    println("final count ${last.count}")
    return Ok(null)
}
```

What to look at by hand: a left click on ` + ` or ` - ` changes the count, one on the border or the empty rows does nothing; `j`, `k` change it; resizing the terminal redraws; `q` leaves with the screen as it was and prints the count; a terminal without mouse support just never sends clicks.

## Elements

An `Element` is a class; every modifier answers a changed copy.

| Constructor | Shows |
|---|---|
| `text(label)` | lines of text; `\n` starts a line, a line too long is cut |
| `button(label)` | ` label ` (a space each side) drawn reversed |
| `vstack(kids)` / `hstack(kids)` | column / row |
| `spacer()` | empty space that grows (one share) |
| `fill(glyph)` | the glyph repeated over the whole room |

| Modifier | Sets | Notes |
|---|---|---|
| `pad(cells)` | empty cells around the content | |
| `gap(cells)` | empty cells between children | |
| `width(cells)` | fixed width | applies along a row |
| `height(cells)` | fixed height | applies along a column |
| `grow()` / `grow_by(weight)` | take shares of what the siblings leave | the remainder goes to the first shares |
| `border(title)` | a line around the element, the title in its top edge | adds 1 cell on every side; outside the padding |
| `fg(color)` / `bg(color)` | colours | `Color.Red`, `Indexed(n)`, `Rgb(r, g, b)`, `Default`, ... |
| `bold()` `dim()` `italic()` `underline()` `reverse()` | attributes | |
| `id(name)` | a tag | its cells are in `lay_out(...).areas` |
| `on_click(f)` | handler | function over `var` state or `\|var s: Counter\| {...}` |

* Style passes down: the colour of an element is used by the text below unless that text sets its own; the flags (`bold`, ...) add up. A `bg` colours the cells of the text it reaches, not the empty cells of a container.
* A child along an axis takes: a fixed size if `width`/`height` is set for that axis; else its share if it grows; else what its content needs. There is no centring or alignment (not planned).
* An element with a handler and no `id` is tagged `~0`, `~1`, ... in order.

## App

| Item | Notes |
|---|---|
| `App(state, view)` | `view: (S) -> Element`; the mouse is reported |
| `.on_key(key, f)` | `key` is the text a `KeyEvent` prints: `q`, `Enter`, `Tab`, `Escape`, `Up`, `Down`, `F5`, `Ctrl+c`, `Alt+x`, `Shift+Tab`... (a character key is the character) |
| `.quit_on(key)` | ends the run on the key |
| `.quit_when(done)` | `done: (S) -> Bool`, checked after every event |
| `.every(ms, f)` | `f: (var S) -> Null` at the interval |
| `.mouse(on)` | whether the terminal reports the mouse (clicks need it) |
| `.run()` | on standard input and output; gives `Result<S, Error>`, the last state |
| `.run_on(terminal)` | on a given `Terminal`; a `Pty` terminal tests an app |
| `lay_out(element, rows, cols)` | `-> Painted { grid, areas }` without a terminal |

A left press runs the handler of the last tagged area, in paint order, that holds the cell (the element under it, or the nearest around it). `Ctrl+c` does not interrupt the program: raw mode stops the signal, so bind it (`quit_on("Ctrl+c")`) or leave through another key. The terminal is put back (raw mode, alternate screen, cursor, mouse) on every exit, errors included. Not a terminal on input (a pipe): `run` gives `Err`.

## Lower level: views, grid, run

| Item | Notes |
|---|---|
| `View.text(s)`, `styled(s, style)`, `fill(glyph, style)`, `column(slots)`, `row(slots)`, `border(title, inner)`, `pad(cells, inner)`, `empty()`, `.tagged(name)` | the tree the builders make |
| `Slot { length, view }`, `fixed(cells, v)`, `fit(v)`, `flex(weight, v)`; `Length`: `Fixed(n)`, `Fit`, `Flex(n)` | a child and its room |
| `render(view, rows, cols) -> Grid`, `render_areas(...) -> Painted` | `Area { tag, row, col, rows, cols }` |
| `Grid`: `rows`, `cols`, `cell(row, col) -> Cell`, `lines() -> List<String>`, `diff(before: Grid?) -> String` | `diff` is the text that turns a terminal showing `before` into this grid: all of it for the first frame or a new size, else the differing runs |
| `Cell { glyph, style }`; `Style` is `term.Style` | a wide glyph fills a cell and an empty second cell |
| `text_width(s)` | columns: wide glyphs 2, combining marks 0 |
| `run(terminal, state, view, update, options)` | the loop; `update: (S, Input) -> Next<S>`; `Input`: `Key(KeyEvent)`, `Mouse(MouseEvent)`, `Tick`, `Resize(rows, cols)`; `Next`: `Continue(S)`, `Quit(S)` |
| `Options { tick_ms: 0, mouse: false, rows: 24, cols: 80 }` | the size is the fallback when the terminal says none |

The loop asks the size on every turn (a change is `Input.Resize` and a full redraw), draws, waits up to 50 ms or the next tick for an event, then calls `update`.

## Limits

No wrapping, scrolling, focus, text fields, alignment or container backgrounds; widths come from a short table of wide and zero-width ranges. Records of these are in `tasks.md` "Not planned".

## Tests

`tools/test-core.sh tile` (`-- builders` for the builders); `cargo test -p mote-integration --test core_tools -- core_tile`.
