# std.sys.gui

A window of boxes, text, images and buttons, laid out with flexbox and grid and drawn on the CPU. `import std.sys.gui as gui`.

`gui.open(title, width, height)` opens a window on the display; only a pinned task (`std.task.pin()`) may open, present and close it, and it is an `Err` with no display. `gui.headless(width, height)` makes a window with no screen, whose pixels read back with `pixel` and `png`.

```mote,skip
import std.sys.gui as gui

fn main() {
    let win = gui.headless(300, 100)!
    let row = win.style(gui.style().width(gui.Size.Pct(100.0)).height(gui.Size.Pct(100.0)).gap(10.0))!
    let red = win.style(gui.style().grow(1.0).background(0xff0000ff))!
    win.set_style(win.root(), row)!
    let a = win.add(win.root(), gui.Kind.Box)!
    win.set_style(a, red)!
    win.present()!
    println(win.rect(a)!)
}
```

A window on a display shows what `present` last painted and repaints by itself after each batch of events, so hover styles need no `present`. A pinned task waits in `for e in win.events()`; a user closing the window raises `Close`, and the program closes it. A task that never parks stalls the window, and sizes are physical pixels. Under WSL the window opens through X11, so Windows draws its frame; install `libxkbcommon-x11-0` for it. `motec/examples/gui_demo.mote` is a complete program: a click counter, a text field, a scrolling list and a clock a second task ticks with `post`.

`win.root()` is the node every other hangs from. `win.add(parent, kind)` adds a last child; `Kind` is `Box`, `Text`, `Image`, `Button`, `Input` or `Scroll`. `win.style(style)` stores a style and answers a `StyleId` for `set_style`. `present` lays out and paints what changed.

| Window method | Does |
|---|---|
| `add(parent, kind)`, `remove(node)` | grows and prunes the tree |
| `move_before(node, before)` | reorders siblings: before `before`, or last when `None` |
| `set_frame(on)`, `set_title(title)` | the system frame and title of a window on a display |
| `drag()`, `drag_resize(edge)`, `minimize()`, `toggle_maximize()`, `maximized()` | move, resize and state of the window, for a title bar a program draws |
| `set_text(node, text)`, `text(node)` | a node's text |
| `set_style(node, style_id)` | gives a stored style to a node |
| `set_image(node, png)` | the picture of an `Image` node |
| `present()` | lays out and paints; answers the rectangles repainted |
| `rect(node)`, `hit(x, y)` | a node's box; the front-most node under a point |
| `scroll(node, dx, dy)` | scrolls a node's content |
| `resize(w, h)`, `close()` | changes the size; closes the window |
| `pixel(x, y)`, `png()` | a pixel as `0xRRGGBBAA` (-1 outside); the picture |

`gui.style()` is a row-flex box with no size or color. Each `Style` method answers a changed copy: `display`, `position`, `direction`, `wrap`, `align_items`, `align_self`, `align_content`, `justify`, `gap`, `gap_xy`, `grow`, `shrink`, `basis`, `width`, `height`, `min_width`, `min_height`, `max_width`, `max_height`, `margin`, `margin_sides`, `padding`, `padding_sides`, `border`, `inset`, `overflow`, `columns`, `rows`, `grid_column`, `grid_row`, `background`, `radius`, `opacity`, `color`, `font_size`, `font_weight`, `line_height`, `text_align`, `hover`, `pressed`.

`win.events()` is a `Stream<UiEvent>` that ends when the window closes; a window has one reader. `win.input(i)` feeds the window one `Input` as the window system would, so a program can be tested without a screen; `win.post(n)` queues `User(n)` from any task.

| `UiEvent` | Raised when |
|---|---|
| `Enter(node)`, `Leave(node)`, `Click(node)` | the pointer enters or leaves a button or input; goes down and up on one |
| `PointerDown(node, x, y)`, `PointerUp(...)`, `PointerMove(...)` | the pointer acts over the front-most node under it |
| `Wheel(node, dx, dy)` | a wheel turn scrolled nothing |
| `Key(key, mods, down)` | a key; `Key` is `Char(code_point)`, `Enter`, `Escape`, `Backspace`, `Delete`, `Tab`, an arrow, `Home` or `End` |
| `Changed(node)`, `Submit(node)` | an input's text changed; Enter was pressed in an input |
| `Resize(w, h)`, `Close`, `User(n)` | the window changed size or was asked to close; another task posted |

`Input` has the same shape (`PointerMove(x, y)`, `PointerDown`, `PointerUp`, `PointerLeave`, `Wheel(x, y, dx, dy)`, `Key`, `Text(s)`, `Resize`, `Close`). `gui.mods(shift, ctrl, alt, meta)` and `gui.no_mods()` make a `Mods`.

A button's `hover` and `pressed` styles change paint fields only. A `Scroll` node scrolls with the wheel. An `Input` node edits one line: a press focuses it and places the caret, dragging selects, the arrows, Home, End, Backspace and Delete edit, Control with `a` selects all, and `Text` inserts. `win.selection(node)` answers `[caret, anchor]`; `win.focus()` and `win.focus_on(node)` read and set the focus. Not included: the system clipboard, input method composition and a right-to-left caret.

A `Size` is `Auto`, `Px(f)` or `Pct(f)`; a `Track` is `Px(f)`, `Fr(f)`, `Auto`, `MinContent` or `MaxContent`. Colors are `0xRRGGBBAA` Ints. A window is 1 to 16384 pixels on a side. Text uses system fonts, with a bundled DejaVu Sans as the fallback.
