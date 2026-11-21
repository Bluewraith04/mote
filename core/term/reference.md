# term: reference

The terminal under `tile`: size, raw mode, escape output, styled text, and the keys and mouse it sends. Native: Zig (`ioctl`, `poll`, `read`, `write` on descriptors).

| Import | Gives |
|---|---|
| `import { Terminal, Pty, Size, Event, Key, KeyEvent, MouseEvent, MouseKind, decode_input } from term` | input and the terminal |
| `import { Color, Style, Span, paint, fg, bg, bold, dim, italic, underline, reverse, reset_style } from term` | colours and attributes |
| `import { clear_screen, clear_line, move_to, hide_cursor, show_cursor, enter_alternate_screen, leave_alternate_screen, enable_mouse, disable_mouse } from term` | escape text |

## Run it yourself

```bash
mote run
```

(An installed `mote` carries the natives; from the repository's sources use `--core` as in `pane`'s page.) Run it in a package whose `src/main.mote` is below, from a real terminal: it prints styled text, then shows each key or mouse event until `q`.

```mote
import { Color, Terminal, paint } from term

fn main() -> Result<Null, Error> {
    var terminal = Terminal.stdio()
    terminal.raw()?
    terminal.mouse(true)?
    terminal.write("${paint("press keys, click, scroll; q quits").fg(Color.Cyan).bold()}\r\n")?
    while true {
        let event = terminal.read_event(1000)?
        if event == null { continue }
        terminal.write("${event}\r\n")?
        if event.to_string() == "q" { break }
    }
    terminal.mouse(false)?
    terminal.close()?
    return Ok(null)
}
```

Raw mode turns off echo, line editing and signals, so `\r\n` is needed and `Ctrl+c` arrives as the event `Ctrl+c`. `close()` gives the settings back. If the program dies before `close()`, run `stty sane` or `reset`. Verified through a pseudo-terminal (`a`, `Up`, a click and `q` were read); trying it in your own terminal is still worth doing.

## Terminal

| Item | Signature | Notes |
|---|---|---|
| `Terminal.stdio()` | | descriptors 0 and 1 |
| `Terminal.on(input_fd, output_fd)` | | any two descriptors |
| `.is_tty()` | `-> Bool` | the input is a terminal |
| `.size()` | `-> Result<Size, Error>` | `Size { rows, cols }`; `Err` when the output is not a terminal |
| `.raw()` | `-> Result<Null, Error>` | stops echo, line editing and signals; a second call does nothing; `Err` (`Unsupported`, "not a terminal") on a pipe |
| `.close()` | `-> Result<Null, Error>` | restores the mode raw replaced; no-op if never raw; leaves the descriptors open |
| `.read_event(timeout_ms)` | `-> Result<Event?, Error>` | null after the time (negative waits for ever); `Err` at the end of input; a lone Escape is told from a sequence by waiting 30 ms |
| `.write(text)` | `-> Result<Null, Error>` | text with escapes |
| `.print(span)` | | a `Span` and then the reset |
| `.clear()` | | clear screen, cursor to top left |
| `.goto(row, col)` | | counted from 0 |
| `.cursor(visible)` | | |
| `.alternate_screen(on)` | | leaves the user's screen as it was |
| `.mouse(on)` | | report presses, releases, drags and the wheel (SGR form) |

`Terminal` is `Closeable`. Bytes typed before `raw()` are held and released after it.

## Events

`Event` is `Key(KeyEvent)` or `Mouse(MouseEvent)`; all print through `to_string()`.

| Type | Fields / text |
|---|---|
| `KeyEvent { key, ctrl, alt, shift }` | text `Ctrl+Alt+Shift+key`: `a`, `Ctrl+c`, `Alt+Up`, `Shift+Tab`, `F5` |
| `Key` | `Char(String)`, `Enter`, `Tab`, `BackTab`, `Backspace`, `Escape`, `Up`, `Down`, `Left`, `Right`, `Home`, `End`, `PageUp`, `PageDown`, `Insert`, `Delete`, `Function(n)` |
| `MouseEvent { kind, button, row, col, ctrl, alt, shift }` | text `Press(0) at 4,7` (row first, from 0); `button` 0 left, 1 middle, 2 right |
| `MouseKind` | `Press Release Drag Move ScrollUp ScrollDown` |

Decoded: plain bytes, UTF-8, control bytes (`Ctrl+letter`), `CSI` and `SS3` sequences with the xterm modifier parameter, an escape before a key as `Alt`, SGR mouse reports. `decode_input(bytes) -> (Event?, Int)` is the first event and the bytes it took: 0 when the sequence stops short, no event for bytes that mean nothing.

## Styled text

| Item | Notes |
|---|---|
| `paint(text) -> Span` | chain `.fg(color)` `.bg(color)` `.bold()` `.dim()` `.italic()` `.underline()` `.reverse()`; in a string (`"${span}"`) it is the codes that set the style, the text, and the reset |
| `Style { fg, bg, bold, dim, italic, underline, reverse }` | the same as data (defaults: `Color.Default`, false); `.codes()` is the setting text, `.same(other)` compares by look |
| `Color` | `Default Black Red Green Yellow Blue Magenta Cyan White`, `Indexed(0-255)`, `Rgb(r, g, b)` |
| `fg(color)`, `bg(color)`, `bold()`, `dim()`, `italic()`, `underline()`, `reverse()`, `reset_style()` | each is the text of one escape |
| `clear_screen()`, `clear_line()`, `move_to(row, col)`, `hide_cursor()`, `show_cursor()`, `enter_alternate_screen()`, `leave_alternate_screen()`, `enable_mouse()`, `disable_mouse()` | the same escapes as strings |

## Pty

A pseudo-terminal, so a terminal program (or a test) needs no terminal.

| Item | Notes |
|---|---|
| `Pty.open()` | `Result<Pty, Error>` |
| `.terminal()` | the program's side: a `Terminal` on the slave |
| `.send(text)` | types at the program |
| `.read(timeout_ms)` | what the program wrote, waiting up to the time for the first bytes; "" when nothing came |
| `.resize(rows, cols)` | gives the terminal side a size |
| `.close()` | closes both ends; a second close is a no-op |

```mote
let pty = Pty.open()!
pty.resize(24, 80)!
pty.send("q")!
var terminal = pty.terminal()
terminal.raw()!
println(terminal.read_event(1000)!!)
terminal.close()!
pty.close()!
```

This is the shape of `tools/test-core.sh term` and of tile's tests.

## Tests

`tools/test-core.sh term`; `cargo test -p mote-integration --test core_tools -- core_term`.
