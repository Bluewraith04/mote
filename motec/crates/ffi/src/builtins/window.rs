//! GUI natives: windows, a node tree each, and the pictures they paint.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use contracts::{CustomSource, EventPayload, EventSink, Overflow, SourceHandle, SourceRequest};
use gui::{Input, Kind, Painter, StyleDef, Surface, UiEvent, NONE};

use super::formats::{bytes_arg, invalid};
use super::*;

/// The `ErrorKind` code for "unsupported".
const UNSUPPORTED: i32 = 7;

/// A surface and its event sink behind the global lock; taffy lengths hold a tagged `*const ()` that is never a real pointer here (no calc values).
struct Locked {
    surface: Surface,
    sink: Option<EventSink>,
    /// Whether a window on a display shows this surface; those are shown and closed from the home thread.
    on_display: bool,
}

// SAFETY: only reached through the `GUI` mutex, and no taffy calc pointer is ever created.
unsafe impl Send for Locked {}

impl Locked {
    /// Queues events for the program, if it is reading them; a full queue replaces its newest.
    fn deliver(&self, events: impl IntoIterator<Item = UiEvent>) {
        let Some(sink) = &self.sink else { return };
        for e in events {
            sink.push(EventPayload::List(e.to_wire().into_iter().map(EventPayload::Float).collect()));
        }
    }
}

struct Gui {
    windows: HashMap<i64, Locked>,
    next_id: i64,
}

static GUI: Mutex<Option<Gui>> = Mutex::new(None);

fn with_gui<R>(f: impl FnOnce(&mut Gui) -> Result<R, NativeError>) -> Result<R, NativeError> {
    let mut guard = GUI.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(|| Gui { windows: HashMap::new(), next_id: 1 }))
}

/// Runs `f` on window `id`'s surface, if it still exists.
#[cfg(feature = "window")]
pub(super) fn with_surface(id: i64, f: impl FnOnce(&mut Surface)) {
    let mut guard = GUI.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(window) = guard.as_mut().and_then(|g| g.windows.get_mut(&id)) {
        f(&mut window.surface);
    }
}

/// Feeds window `id` one input from the window system and queues the events it raises.
#[cfg(feature = "window")]
pub(super) fn feed(id: i64, input: &Input) {
    let mut guard = GUI.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(window) = guard.as_mut().and_then(|g| g.windows.get_mut(&id)) {
        let events = window.surface.input(input);
        window.deliver(events);
    }
}

#[cfg(feature = "window")]
use super::os_window as os;

#[cfg(not(feature = "window"))]
mod os {
    use gui::Surface;

    pub(super) fn open(_: i64, _: String, _: u32, _: u32) -> Result<(), String> {
        Err("this build has no window system".to_string())
    }

    pub(super) fn close(_: i64) {}

    pub(super) fn chrome(_: i64, _: i64, _: i64) -> Result<i64, String> {
        Ok(0)
    }

    pub(super) fn set_title(_: i64, _: &str) {}

    pub(super) fn show(_: i64, _: &Surface) {}
}

fn on_home_thread(ctx: &NativeCallContext<'_>) -> bool {
    ctx.heap.on_home_thread()
}

fn with_window<R>(ctx: &NativeCallContext<'_>, f: impl FnOnce(&mut Surface) -> Result<R, String>) -> Result<R, NativeError> {
    let id = arg_int(ctx, 0);
    with_gui(|g| {
        let window = g.windows.get_mut(&id).ok_or_else(|| invalid(format!("no window {id}")))?;
        f(&mut window.surface).map_err(invalid)
    })
}

fn node_arg(ctx: &NativeCallContext<'_>, i: usize) -> Result<u32, NativeError> {
    let n = arg_int(ctx, i);
    u32::try_from(n).ok().filter(|n| *n != NONE).ok_or_else(|| invalid(format!("no node {n}")))
}

fn float_arg(ctx: &NativeCallContext<'_>, i: usize) -> f64 {
    ctx.arg(i).and_then(|v| v.as_float().or_else(|| v.as_int().map(|n| n as f64))).unwrap_or(0.0)
}

fn float_list_arg(ctx: &NativeCallContext<'_>, idx: usize) -> Result<Vec<f64>, String> {
    let list = ctx.arg(idx).ok_or("missing style list")?;
    let len = slot_count(ctx.heap, list, HEADER_LEN_SLOT)?;
    let backing = ctx.heap.get_slot(list, HEADER_BACKING_SLOT)?;
    (0..len)
        .map(|i| {
            let v = ctx.heap.get_slot(backing, BACKING_DATA_BASE + i)?;
            v.as_float().or_else(|| v.as_int().map(|n| n as f64)).ok_or_else(|| "a style list holds numbers".to_string())
        })
        .collect()
}

pub(super) fn ui_headless(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let (w, h) = (arg_int(ctx, 0), arg_int(ctx, 1));
    let (w, h) = (u32::try_from(w).unwrap_or(0), u32::try_from(h).unwrap_or(0));
    with_gui(|g| {
        let surface = Surface::new(w, h, Painter::new()).map_err(invalid)?;
        let id = g.next_id;
        g.next_id += 1;
        g.windows.insert(id, Locked { surface, sink: None, on_display: false });
        Ok(Value::int(id))
    })
}

/// A window on a display; only the pinned (home thread) task may open one.
pub(super) fn ui_open(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let title = ctx.arg(0).and_then(|v| v.as_heap_string()).unwrap_or_default();
    let (w, h) = (u32::try_from(arg_int(ctx, 1)).unwrap_or(0), u32::try_from(arg_int(ctx, 2)).unwrap_or(0));
    if !on_home_thread(ctx) {
        return Err(invalid("a window opens from a pinned task: call task.pin() first"));
    }
    let id = with_gui(|g| {
        let surface = Surface::new(w, h, Painter::new()).map_err(invalid)?;
        let id = g.next_id;
        g.next_id += 1;
        g.windows.insert(id, Locked { surface, sink: None, on_display: true });
        Ok(id)
    })?;
    match os::open(id, title, w, h) {
        Ok(()) => Ok(Value::int(id)),
        Err(message) => {
            with_gui(|g| Ok(g.windows.remove(&id)))?;
            Err(NativeError::failure(UNSUPPORTED, message))
        }
    }
}

/// A frame operation on a window on a display (see `os::chrome`); a headless window has no frame and answers 0.
pub(super) fn ui_chrome(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let (id, op, arg) = (arg_int(ctx, 0), arg_int(ctx, 1), arg_int(ctx, 2));
    let home = on_home_thread(ctx);
    with_gui(|g| {
        let window = g.windows.get(&id).ok_or_else(|| invalid(format!("no window {id}")))?;
        if !window.on_display {
            return Ok(Value::int(0));
        }
        if !home {
            return Err(invalid("a window on a display changes from the pinned task that opened it"));
        }
        os::chrome(id, op, arg).map(Value::int).map_err(invalid)
    })
}

pub(super) fn ui_set_title(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let id = arg_int(ctx, 0);
    let title = ctx.arg(1).and_then(|v| v.as_heap_string()).unwrap_or_default();
    let home = on_home_thread(ctx);
    with_gui(|g| {
        let window = g.windows.get(&id).ok_or_else(|| invalid(format!("no window {id}")))?;
        if window.on_display {
            if !home {
                return Err(invalid("a window on a display changes from the pinned task that opened it"));
            }
            os::set_title(id, &title);
        }
        Ok(Value::null())
    })
}

pub(super) fn ui_close(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let id = arg_int(ctx, 0);
    let home = on_home_thread(ctx);
    with_gui(|g| {
        if g.windows.get(&id).is_some_and(|w| w.on_display) && !home {
            return Err(invalid("a window on a display closes from the pinned task that opened it"));
        }
        let window = g.windows.remove(&id).ok_or_else(|| invalid(format!("no window {id}")))?;
        if window.on_display {
            os::close(id);
        }
        if let Some(sink) = window.sink {
            sink.end();
        }
        Ok(Value::null())
    })
}

/// Feeds the window one input as the window system would; answers how many events it raised.
pub(super) fn ui_input(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let kind = arg_int(ctx, 1);
    let input = Input::from_wire(kind, float_arg(ctx, 2), float_arg(ctx, 3), float_arg(ctx, 4), float_arg(ctx, 5)).map_err(invalid)?;
    let id = arg_int(ctx, 0);
    with_gui(|g| {
        let window = g.windows.get_mut(&id).ok_or_else(|| invalid(format!("no window {id}")))?;
        let events = window.surface.input(&input);
        let n = events.len();
        window.deliver(events);
        Ok(Value::int(n as i64))
    })
}

/// Types text into the focused input; answers how many events it raised.
pub(super) fn ui_type(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let text = ctx.arg(1).and_then(|v| v.as_heap_string()).unwrap_or_default();
    let id = arg_int(ctx, 0);
    with_gui(|g| {
        let window = g.windows.get_mut(&id).ok_or_else(|| invalid(format!("no window {id}")))?;
        let events = window.surface.input(&Input::Text(text));
        let n = events.len();
        window.deliver(events);
        Ok(Value::int(n as i64))
    })
}

pub(super) fn ui_focus(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    with_window(ctx, |s| Ok(Value::int(s.tree.focus().map_or(-1, i64::from))))
}

pub(super) fn ui_focus_on(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let node = node_arg(ctx, 1)?;
    with_window(ctx, |s| {
        s.tree.kind(node).ok_or_else(|| format!("no node {node}"))?;
        s.tree.focus_on(Some(node));
        Ok(Value::null())
    })
}

pub(super) fn ui_selection(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let node = node_arg(ctx, 1)?;
    let (caret, anchor) = with_window(ctx, |s| s.tree.selection(node).ok_or_else(|| format!("node {node} is not an input")))?;
    Ok(alloc_list(ctx, &[Value::int(caret as i64), Value::int(anchor as i64)])?)
}

/// Queues a `User` event from any task.
pub(super) fn ui_post(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let (id, tag) = (arg_int(ctx, 0), arg_int(ctx, 1));
    with_gui(|g| {
        let window = g.windows.get_mut(&id).ok_or_else(|| invalid(format!("no window {id}")))?;
        window.deliver([UiEvent::User(tag)]);
        Ok(Value::null())
    })
}

fn decode_event(cx: &mut dyn NativeCtx, payload: EventPayload) -> Result<Value, String> {
    let EventPayload::List(items) = payload else { return Err("a window event is a list".to_string()) };
    let list = cx.alloc_header(LIST_TYPE_ID, 2)?;
    let backing = cx.alloc_backing(items.len())?;
    for (i, item) in items.iter().enumerate() {
        let EventPayload::Float(f) = item else { return Err("a window event holds numbers".to_string()) };
        cx.set_slot(backing, BACKING_DATA_BASE + i, Value::float(*f))?;
    }
    cx.set_slot(list, HEADER_LEN_SLOT, Value::uint(items.len() as u64))?;
    cx.set_slot(list, HEADER_BACKING_SLOT, backing)?;
    Ok(list)
}

/// The window's events as a channel of `[code, node, a, b, c]` lists; one reader per window.
pub(super) fn ui_events(ctx: &mut NativeCallContext<'_>) -> Result<SourceSpec, String> {
    let id = arg_int(ctx, 0);
    let start = Arc::new(move |sink: EventSink| -> Result<SourceHandle, String> {
        let mut guard = GUI.lock().unwrap_or_else(|e| e.into_inner());
        let window = guard.as_mut().and_then(|g| g.windows.get_mut(&id)).ok_or_else(|| format!("no window {id}"))?;
        if window.sink.is_some() {
            return Err("the window's events are already being read".to_string());
        }
        window.sink = Some(sink);
        Ok(SourceHandle::new(move || {
            let mut guard = GUI.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(window) = guard.as_mut().and_then(|g| g.windows.get_mut(&id)) {
                window.sink = None;
            }
        }))
    });
    Ok(SourceSpec { request: SourceRequest::Custom(CustomSource(start)), capacity: 256, overflow: Overflow::Coalesce, decode: decode_event })
}

pub(super) fn ui_default_style(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let wire: Vec<Value> = StyleDef::default().to_wire().into_iter().map(Value::float).collect();
    alloc_list(ctx, &wire)
}

pub(super) fn ui_style(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let wire = float_list_arg(ctx, 1)?;
    let def = StyleDef::from_wire(&wire).map_err(|e| invalid(format!("style: {e}")))?;
    with_window(ctx, |s| {
        for (what, id) in [("hover", def.hover), ("pressed", def.pressed)] {
            if id != gui::NO_STYLE && id as usize >= s.tree.style_count() {
                return Err(format!("style: the {what} style does not exist"));
            }
        }
        Ok(Value::int(i64::from(s.tree.intern_style(def))))
    })
}

pub(super) fn ui_add(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let parent = node_arg(ctx, 1)?;
    let code = arg_int(ctx, 2);
    let kind = Kind::from_code(code).ok_or_else(|| invalid(format!("no node kind {code}")))?;
    with_window(ctx, |s| s.tree.add(parent, kind).map(|n| Value::int(i64::from(n))))
}

pub(super) fn ui_remove(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let node = node_arg(ctx, 1)?;
    with_window(ctx, |s| s.tree.remove(node).map(|()| Value::null()))
}

pub(super) fn ui_move_before(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let node = node_arg(ctx, 1)?;
    let before = if arg_int(ctx, 2) < 0 { None } else { Some(node_arg(ctx, 2)?) };
    with_window(ctx, |s| s.tree.move_before(node, before).map(|()| Value::null()))
}

pub(super) fn ui_set_text(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let node = node_arg(ctx, 1)?;
    let text = ctx.arg(2).and_then(|v| v.as_heap_string()).unwrap_or_default();
    with_window(ctx, |s| s.tree.set_text(node, &text).map(|()| Value::null()))
}

pub(super) fn ui_text(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let node = node_arg(ctx, 1)?;
    let text = with_window(ctx, |s| s.tree.text(node).map(str::to_string))?;
    Ok(ctx.heap.alloc_string(text.as_bytes())?)
}

pub(super) fn ui_set_style(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let node = node_arg(ctx, 1)?;
    let style = u32::try_from(arg_int(ctx, 2)).map_err(|_| invalid("no such style"))?;
    with_window(ctx, |s| s.tree.set_style(node, style).map(|()| Value::null()))
}

pub(super) fn ui_set_image(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let node = node_arg(ctx, 1)?;
    let png = bytes_arg(ctx, 2)?;
    with_window(ctx, |s| {
        let Surface { tree, painter, .. } = s;
        painter.load_image(tree, node, &png).map(|()| Value::null())
    })
}

pub(super) fn ui_present(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let id = arg_int(ctx, 0);
    let home = on_home_thread(ctx);
    with_gui(|g| {
        let window = g.windows.get_mut(&id).ok_or_else(|| invalid(format!("no window {id}")))?;
        if window.on_display && !home {
            return Err(invalid("a window on a display is presented from the pinned task that opened it"));
        }
        let painted = window.surface.present().len();
        if window.on_display {
            os::show(id, &window.surface);
        }
        Ok(Value::int(painted as i64))
    })
}

pub(super) fn ui_rect(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let node = node_arg(ctx, 1)?;
    let r = with_window(ctx, |s| {
        s.tree.kind(node).ok_or_else(|| format!("no node {node}"))?;
        Ok(s.tree.rect(node))
    })?;
    let list: Vec<Value> = [r.x, r.y, r.w, r.h].into_iter().map(|v| Value::float(f64::from(v))).collect();
    Ok(alloc_list(ctx, &list)?)
}

pub(super) fn ui_hit(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let (x, y) = (float_arg(ctx, 1) as f32, float_arg(ctx, 2) as f32);
    with_window(ctx, |s| Ok(Value::int(s.tree.hit(x, y).map_or(-1, i64::from))))
}

pub(super) fn ui_scroll(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let node = node_arg(ctx, 1)?;
    let (dx, dy) = (float_arg(ctx, 2) as f32, float_arg(ctx, 3) as f32);
    with_window(ctx, |s| {
        s.tree.kind(node).ok_or_else(|| format!("no node {node}"))?;
        Ok(if s.tree.scroll_by(node, dx, dy) { Value::true_() } else { Value::false_() })
    })
}

pub(super) fn ui_resize(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let (w, h) = (u32::try_from(arg_int(ctx, 1)).unwrap_or(0), u32::try_from(arg_int(ctx, 2)).unwrap_or(0));
    with_window(ctx, |s| s.resize(w, h).map(|()| Value::null()))
}

pub(super) fn ui_pixel(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let (x, y) = (u32::try_from(arg_int(ctx, 1)), u32::try_from(arg_int(ctx, 2)));
    with_window(ctx, |s| Ok(Value::int(match (x, y) {
        (Ok(x), Ok(y)) => s.pixel(x, y).map_or(-1, i64::from),
        _ => -1,
    })))
}

pub(super) fn ui_png(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let png = with_window(ctx, |s| s.png())?;
    Ok(alloc_bytes(ctx, &png)?)
}
