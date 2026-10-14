//! The terminal: raw mode, size and an event source over `crossterm`; restored at process exit.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use contracts::{EventPayload, EventSink, PlatformError, PlatformErrorKind, PushResult, SourceHandle};
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};

/// Whether the terminal is open, and whether it was opened with the mouse.
static OPEN: Mutex<Option<bool>> = Mutex::new(None);
static HOOKED: AtomicBool = AtomicBool::new(false);

fn failed(e: std::io::Error) -> PlatformError {
    PlatformError { kind: PlatformErrorKind::Unsupported, message: format!("terminal: {e}") }
}

pub(crate) fn open(mouse: bool) -> Result<(), PlatformError> {
    let mut state = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    if state.is_some() {
        return Ok(());
    }
    enable_raw_mode().map_err(failed)?;
    let mut out = std::io::stdout();
    let entered = crossterm::execute!(out, EnterAlternateScreen).and_then(|_| if mouse { crossterm::execute!(out, EnableMouseCapture) } else { Ok(()) });
    if let Err(e) = entered {
        let _ = disable_raw_mode();
        return Err(failed(e));
    }
    *state = Some(mouse);
    drop(state);
    hook_exit();
    Ok(())
}

pub(crate) fn close() {
    let mut state = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    let Some(mouse) = state.take() else { return };
    let mut out = std::io::stdout();
    if mouse {
        let _ = crossterm::execute!(out, DisableMouseCapture);
    }
    let _ = crossterm::execute!(out, crossterm::cursor::Show, LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

pub(crate) fn size() -> Result<i64, PlatformError> {
    let (cols, rows) = crossterm::terminal::size().map_err(failed)?;
    Ok(i64::from(cols) * 65536 + i64::from(rows))
}

#[cfg(unix)]
fn hook_exit() {
    extern "C" fn restore() {
        close();
    }
    if !HOOKED.swap(true, Ordering::SeqCst) {
        // SAFETY: `restore` is a plain `extern "C"` function with no arguments.
        unsafe { libc::atexit(restore) };
    }
}

#[cfg(not(unix))]
fn hook_exit() {
    HOOKED.store(true, Ordering::SeqCst);
}

/// A thread polls for input in 50 ms steps and pushes each event as a list of integers.
pub(crate) fn watch(sink: EventSink) -> Result<SourceHandle, PlatformError> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::Builder::new()
        .name("mote-term".to_string())
        .spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                match crossterm::event::poll(Duration::from_millis(50)) {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(_) => break,
                }
                let Ok(event) = crossterm::event::read() else { break };
                if let Some(fields) = encode(event) {
                    let list = EventPayload::List(fields.into_iter().map(EventPayload::Int).collect());
                    if sink.push(list) == PushResult::Closed {
                        break;
                    }
                }
            }
        })
        .map_err(failed)?;
    Ok(SourceHandle::new(move || stop.store(true, Ordering::SeqCst)))
}

const SHIFT: i64 = 1;
const CTRL: i64 = 2;
const ALT: i64 = 4;

fn mods(m: KeyModifiers) -> i64 {
    let mut bits = 0;
    if m.contains(KeyModifiers::SHIFT) {
        bits |= SHIFT;
    }
    if m.contains(KeyModifiers::CONTROL) {
        bits |= CTRL;
    }
    if m.contains(KeyModifiers::ALT) {
        bits |= ALT;
    }
    bits
}

/// A key code: a character is its code point and any other key is a negative number.
fn key(code: KeyCode) -> Option<i64> {
    Some(match code {
        KeyCode::Char(c) => c as i64,
        KeyCode::Enter => -1,
        KeyCode::Esc => -2,
        KeyCode::Tab | KeyCode::BackTab => -3,
        KeyCode::Backspace => -4,
        KeyCode::Delete => -5,
        KeyCode::Insert => -6,
        KeyCode::Up => -7,
        KeyCode::Down => -8,
        KeyCode::Left => -9,
        KeyCode::Right => -10,
        KeyCode::Home => -11,
        KeyCode::End => -12,
        KeyCode::PageUp => -13,
        KeyCode::PageDown => -14,
        KeyCode::F(n) => -100 - i64::from(n),
        _ => return None,
    })
}

fn button(b: MouseButton) -> i64 {
    match b {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}

/// An event as a list of integers: a key, a mouse action or a resize.
fn encode(event: Event) -> Option<Vec<i64>> {
    match event {
        Event::Key(k) if k.kind != KeyEventKind::Release => {
            let mut bits = mods(k.modifiers);
            if k.code == KeyCode::BackTab {
                bits |= SHIFT;
            }
            Some(vec![0, key(k.code)?, bits])
        }
        Event::Mouse(m) => {
            let (kind, b) = match m.kind {
                MouseEventKind::Down(b) => (0, button(b)),
                MouseEventKind::Up(b) => (1, button(b)),
                MouseEventKind::Drag(b) => (2, button(b)),
                MouseEventKind::Moved => (3, 0),
                MouseEventKind::ScrollUp => (4, 0),
                MouseEventKind::ScrollDown => (5, 0),
                _ => return None,
            };
            Some(vec![1, kind, b, i64::from(m.column), i64::from(m.row)])
        }
        Event::Resize(cols, rows) => Some(vec![2, i64::from(cols), i64::from(rows)]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyEventState, MouseEvent};

    fn press(code: KeyCode, m: KeyModifiers) -> Event {
        Event::Key(KeyEvent { code, modifiers: m, kind: KeyEventKind::Press, state: KeyEventState::NONE })
    }

    #[test]
    fn keys_encode_with_modifiers() {
        assert_eq!(encode(press(KeyCode::Char('é'), KeyModifiers::NONE)), Some(vec![0, 233, 0]));
        assert_eq!(encode(press(KeyCode::Up, KeyModifiers::CONTROL | KeyModifiers::SHIFT)), Some(vec![0, -7, 3]));
        assert_eq!(encode(press(KeyCode::F(5), KeyModifiers::NONE)), Some(vec![0, -105, 0]));
        assert_eq!(encode(press(KeyCode::BackTab, KeyModifiers::NONE)), Some(vec![0, -3, 1]));
    }

    #[test]
    fn mouse_and_resize_encode() {
        let m = Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Right), column: 4, row: 9, modifiers: KeyModifiers::NONE });
        assert_eq!(encode(m), Some(vec![1, 0, 2, 4, 9]));
        assert_eq!(encode(Event::Resize(80, 24)), Some(vec![2, 80, 24]));
        assert_eq!(encode(Event::FocusGained), None);
    }
}
