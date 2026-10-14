# tile

A terminal UI described as a function of your state. A Mote package, written in Mote over `std.term` with no native code of its own.

| Step | Command or line |
|---|---|
| add it | nothing: it ships with Mote |
| use it | `import tile` |
| try the demo | `$ cd apps/tile_demo && mote run` |

You write a `view` that turns a state value into a tree of elements. Handlers on the elements change the state, and the screen is drawn again from the new tree; only the cells that differ are written to the terminal. It is built on `std.term`, which stays available for anything the elements do not cover.

This page covers the app and its state, the elements and their modifiers, handlers, keys, timers and mouse, colors, testing, and the limits.

## State and view

The state is a class. The view reads it and answers one element; handlers take the state as a `var` parameter and change it. `tile.App.new(state, view)` is the app, and `run` takes the terminal over until the app quits.

```mote,skip
import tile
import std.term as term

class Counter {
    var count: Int
}

fn view(s: Counter) -> tile.Element<Counter> {
    let body: tile.Element<Counter> = tile.vstack([
        tile.text("Count: ${s.count}").bold(),
        tile.text("Up adds one, Esc quits").dim(),
    ])
    return tile.frame("Counter", body).pad(1)
}

fn main() {
    let app = tile.App.new(Counter { count: 0 }, view)
        .on_key(term.Key.Up, tile.no_mods(), |var s| { s.count += 1 })
        .quit_on(term.Key.Escape)
    match app.run() {
        Err(e) => { println("not a terminal: ${e.message}") }
        _ => { }
    }
}
```

`run` answers `Err` when stdin or stdout is not a terminal. The terminal is put back (cursor, screen, mouse) when the app quits, fails, or the process exits.

## Elements

Every constructor answers an `Element<S>`, where `S` is the state class. Give a `let` an explicit `tile.Element<S>` type when it holds one.

| Constructor | Shows |
|---|---|
| `text(s)` | text; a newline starts another line |
| `rich(runs)` | text in several looks, from `tile.run(text)` runs: `tile.rich([tile.run("a "), tile.run("b").bold()])` |
| `vstack(kids)` | children one under the other |
| `hstack(kids)` | children side by side |
| `spacer()` | empty space that takes what the parent has left, to push siblings apart |
| `frame(title, kid)` | a border and a title around one child |
| `list(items, selected, top)` | one row per item (more with `wrap()`), item `selected` highlighted (-1 for none), starting at item `top`; when `selected` changes the list scrolls itself to bring it into view; change `top` to move the view by hand, and it stays there until `selected` changes again |
| `input(value, on_change)` | a field showing `value`, wrapped at spaces onto the next row when it is longer than the field is wide; `on_change` is `\|var s, text\|` |
| `scroll(offset, kids)` | children one under the other (`gap(n)` rows between), starting `offset` rows down, clipped to the element; a child with a `height` is neither measured nor painted unless it shows, so a long document costs what the screen shows |
| `height(element, width)` | not an element: the rows `element` takes at `width` columns (-1 for no limit), to give a scroll's children their `height` and to know how far a document goes |

Modifiers answer a changed copy, so they chain: `tile.text("Done").bold().fg(0x9ece6a)`.

| Modifier | Sets |
|---|---|
| `fg(color)`, `bg(color)` | colors, `0xRRGGBB`; a child without its own takes the background beneath it |
| `bold()`, `dim()`, `underline()`, `reverse()` | text style |
| `pad(n)`, `gap(n)` | cells inside, and between the children of a stack |
| `width(n)`, `height(n)` | a fixed size in cells |
| `grow()`, `shares(n)` | a share of what the parent has left; `grow()` is `shares(1)` |
| `wrap()` | breaks text at spaces to fit its width; a `list` breaks its items the same way, with the later rows of an item indented, and highlights every row of the selected one |
| `numbered()` | puts `1.`, `2.` and so on before the items of a `list`; a wrapped item's later rows line up under its text |
| `center()`, `right()` | aligns text in its width |

## Handlers

| Modifier | Runs when |
|---|---|
| `on_click(f)` | the element, or something inside it, is clicked; `f` is `\|var s\|` |
| `on_pick(f)` | a row of a `list` is clicked; `f` is `\|var s, index\|` |
| `on_change(f)` | the text of the focused `input` changes; `f` is `\|var s, text\|` |
| `on_submit(f)` | Enter is pressed in the focused `input`; `f` is `\|var s\|` |
| `focus()` | (not a handler) makes an `input` the one that takes typed keys and shows the cursor at its end |

Focus, the selected row, a list's starting item (when you move it yourself) and a scroll offset live in your state; the view passes them in. Typed keys go to the one input marked `focus()` in the view, and Backspace edits its end.

## App

| Method | Does |
|---|---|
| `on_key(key, mods, f)` | runs `f` when `key` is pressed with exactly those modifiers |
| `quit_on(key)` | ends the app on `key` with no modifiers |
| `quit_when(done)` | ends the app once `done(state)` is true, checked after every event |
| `every(ms, f)` | runs `f` on the state every `ms` milliseconds |
| `on_resize(f)` | runs `f(state, cols, rows)` before the first frame and at every resize, so the state knows the size of the screen |
| `mouse()` | reports clicks to `on_click` and `on_pick` handlers, and wheel steps to `on_wheel` |
| `on_wheel(f)` | runs `f(state, step)` for each step of the mouse wheel, -1 up and 1 down; needs `mouse()` |
| `run()` | takes the terminal over until the app quits |
| `headless(cols, rows)` | answers a `Running` on an in-memory screen, for tests |

`tile.no_mods()` and `tile.mods(shift, ctrl, alt)` build the `mods` argument. Ctrl+C is a key like any other in raw mode; bind it with `on_key` or `quit_on`. Colors are truecolor only; with `NO_COLOR` set none are written and the styles still are.

## Testing

`headless` runs the app on a screen in memory. The `Running` it answers takes the events a person or the clock would send.

| Method | Does |
|---|---|
| `press(key, mods)`, `type_text(text)` | a key, or text one character at a time |
| `click(col, row)`, `wheel(step)` | a click at a cell, or a wheel step (-1 up, 1 down) |
| `resize(cols, rows)` | a new size |
| `tick()`, `fire(index)` | every timer once, or timer `index` once |
| `screen()`, `shows(text)`, `cell(col, row)` | the rows as text, whether some row holds `text`, one cell with its colors and bits |
| `state()`, `finished()` | the state, and whether the app has ended |
| `take_output()` | the bytes that would have gone to the terminal |

The example above, driven headless and printed:

```mote,skip
var app = tile.App.new(Counter { count: 0 }, view)
    .on_key(term.Key.Up, tile.no_mods(), |var s| { s.count += 1 })
    .quit_on(term.Key.Escape)
    .headless(30, 6)
app.press(term.Key.Up, tile.no_mods())
app.press(term.Key.Up, tile.no_mods())
for line in app.screen() { println("[${line}]") }
```

```output
[┌─ Counter ──────────────────┐]
[│                            │]
[│ Count: 2                   │]
[│ Up adds one, Esc quits     │]
[│                            │]
[└────────────────────────────┘]
```

## Wide characters

Widths come from `std.term.width` and `std.term.glyphs`. A CJK character or an emoji takes two cells, an accent takes none, and every part of tile counts cells: wrapping, `height`, frames, lists and the cursor of an `input`. Backspace removes a whole character. A wide character cut by the edge of its element is drawn as a blank.

| Where | What a program sees |
|---|---|
| `Screen.get(col, row).ch` | the character in its first cell and `""` in its second |
| `screen.glyphs(text)` | the columns of a text, `""` after a wide character, so its length is the width |
| `screen()` and `shows` in a test | the rows as text, the second cell adding nothing |

## Limits

| Limit | Why |
|---|---|
| a terminal that draws a character differently than `std.term.width` says | a wide character is two cells, an accent joins its letter, an emoji sequence (a family, a flag) is two cells; a terminal that draws a sequence as several emoji misaligns that row; right-to-left and shaped scripts are not placed |
| an input has no cursor movement | typing and Backspace act at the end |
| no per-element key handlers | keys go to `on_key` and the focused input |
| a 200 by 50 frame takes about 32 ms empty and 60 to 105 ms full of text | the view, the paint and the diff all run in Mote, a cell at a time |
| a `list` is not windowed | it builds a string for every item each frame, so a program with many items passes only the rows that show and keeps `top` at 0 |

The tests are in `tests/` and run with `mote test`.
