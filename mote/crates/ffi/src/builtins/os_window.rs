//! Windows on a display: winit owns the window and its events, softbuffer shows what the painter drew.
//!
//! Everything here runs on the home thread, which waits in winit's loop through [`contracts::HomeLoop`] whenever it has no task to run.

use std::cell::RefCell;
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use contracts::HomeLoop;
use gui::{Input, Surface, keys};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::platform::pump_events::{EventLoopExtPumpEvents, PumpStatus};
use winit::window::{Fullscreen, ResizeDirection, Window, WindowId};

/// Pixels a wheel line moves.
const LINE_PIXELS: f32 = 40.0;

/// How long creating a window may take before it is given up on.
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);

struct OsWindow {
    window: Arc<Window>,
    surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
    gui_id: i64,
    cursor: (f32, f32),
    mods: u32,
    shown: (u32, u32),
}

struct Request {
    gui_id: i64,
    title: String,
    width: u32,
    height: u32,
}

#[derive(Default)]
struct App {
    context: Option<softbuffer::Context<Arc<Window>>>,
    windows: HashMap<WindowId, OsWindow>,
    pending: Vec<Request>,
    created: HashMap<i64, Result<(), String>>,
}

struct Shell {
    event_loop: EventLoop<()>,
    app: App,
}

thread_local! {
    static SHELL: RefCell<Option<Shell>> = const { RefCell::new(None) };
}

/// What the scheduler's home worker waits in.
struct Hook {
    proxy: EventLoopProxy<()>,
    woken: AtomicBool,
    windows: AtomicUsize,
}

impl HomeLoop for Hook {
    fn wait(&self, timeout: Option<Duration>) {
        let timeout = if self.woken.swap(false, Ordering::SeqCst) { Some(Duration::ZERO) } else { timeout };
        pump(timeout);
        self.woken.store(false, Ordering::SeqCst);
    }

    fn wake(&self) {
        self.woken.store(true, Ordering::SeqCst);
        let _ = self.proxy.send_event(());
    }

    fn is_open(&self) -> bool {
        self.windows.load(Ordering::SeqCst) > 0
    }
}

static HOOK: std::sync::OnceLock<Arc<Hook>> = std::sync::OnceLock::new();

fn pump(timeout: Option<Duration>) {
    let ended = SHELL.with(|cell| match cell.borrow_mut().as_mut() {
        Some(Shell { event_loop, app }) => matches!(event_loop.pump_app_events(timeout, app), PumpStatus::Exit(_)),
        None => false,
    });
    if ended {
        display_lost();
    }
}

/// Drops the window system after its connection ended and tells every window to close.
fn display_lost() {
    let Some(shell) = SHELL.with(|cell| cell.borrow_mut().take()) else { return };
    let Shell { event_loop, app } = shell;
    let ids: Vec<i64> = app.windows.values().map(|os| os.gui_id).collect();
    if let Some(hook) = HOOK.get() {
        hook.windows.store(0, Ordering::SeqCst);
    }
    for id in ids {
        super::window::feed(id, &Input::Close);
    }
    // Dropping them would talk to the dead connection.
    std::mem::forget(app);
    std::mem::forget(event_loop);
}

/// Whether this process runs under WSL, whose Wayland compositor has no window frame and crashes on winit's own.
#[cfg(all(unix, not(any(target_os = "macos", target_os = "android", target_os = "ios"))))]
fn under_wsl() -> bool {
    std::env::var_os("WSL_DISTRO_NAME").is_some() || std::env::var_os("WSL_INTEROP").is_some()
}

/// Builds the event loop; under WSL it goes through X11, whose windows Windows frames.
#[cfg(all(unix, not(any(target_os = "macos", target_os = "android", target_os = "ios"))))]
fn build_event_loop() -> Result<EventLoop<()>, String> {
    use winit::platform::x11::EventLoopBuilderExtX11;
    let wsl = under_wsl();
    let made = std::panic::catch_unwind(|| {
        let mut builder = EventLoop::builder();
        if wsl {
            builder.with_x11();
        }
        builder.build()
    });
    let hint = if wsl { "; under WSL install libxkbcommon-x11-0" } else { "" };
    match made {
        Ok(Ok(event_loop)) => Ok(event_loop),
        Ok(Err(e)) => Err(format!("no display to open a window on: {}{hint}", short(&e.to_string()))),
        Err(_) => Err(format!("no display to open a window on: a system library is missing{hint}")),
    }
}

#[cfg(not(all(unix, not(any(target_os = "macos", target_os = "android", target_os = "ios")))))]
fn build_event_loop() -> Result<EventLoop<()>, String> {
    EventLoop::new().map_err(|e| format!("no display to open a window on: {}", short(&e.to_string())))
}

/// The last clause of a windowing error, which drops the source path the library puts first.
fn short(message: &str) -> &str {
    message.rsplit(": ").next().unwrap_or(message)
}

fn key_code(key: &Key) -> Option<u32> {
    Some(match key {
        Key::Named(NamedKey::Enter) => keys::ENTER,
        Key::Named(NamedKey::Escape) => keys::ESCAPE,
        Key::Named(NamedKey::Backspace) => keys::BACKSPACE,
        Key::Named(NamedKey::Delete) => keys::DELETE,
        Key::Named(NamedKey::Tab) => keys::TAB,
        Key::Named(NamedKey::ArrowUp) => keys::UP,
        Key::Named(NamedKey::ArrowDown) => keys::DOWN,
        Key::Named(NamedKey::ArrowLeft) => keys::LEFT,
        Key::Named(NamedKey::ArrowRight) => keys::RIGHT,
        Key::Named(NamedKey::Home) => keys::HOME,
        Key::Named(NamedKey::End) => keys::END,
        Key::Named(NamedKey::Space) => ' ' as u32,
        Key::Character(s) => s.chars().next()?.to_lowercase().next()? as u32,
        _ => return None,
    })
}

/// Copies the painted pixels to the window.
fn blit(os: &mut OsWindow, surface: &Surface) {
    let (w, h) = (surface.width(), surface.height());
    let (Some(nw), Some(nh)) = (NonZeroU32::new(w), NonZeroU32::new(h)) else { return };
    if os.shown != (w, h) {
        if os.surface.resize(nw, nh).is_err() {
            return;
        }
        os.shown = (w, h);
    }
    let Ok(mut buffer) = os.surface.buffer_mut() else { return };
    let (pixels, _) = surface.pixels().as_chunks::<4>();
    for (dst, px) in buffer.iter_mut().zip(pixels) {
        *dst = u32::from(px[0]) << 16 | u32::from(px[1]) << 8 | u32::from(px[2]);
    }
    let _ = buffer.present();
}

impl App {
    fn create_pending(&mut self, el: &ActiveEventLoop) {
        for request in std::mem::take(&mut self.pending) {
            let made = self.create(el, &request);
            self.created.insert(request.gui_id, made);
        }
    }

    fn create(&mut self, el: &ActiveEventLoop, request: &Request) -> Result<(), String> {
        let attributes = Window::default_attributes().with_title(request.title.clone()).with_inner_size(PhysicalSize::new(request.width, request.height));
        let window = Arc::new(el.create_window(attributes).map_err(|e| format!("cannot open a window: {}", short(&e.to_string())))?);
        if self.context.is_none() {
            self.context = Some(softbuffer::Context::new(window.clone()).map_err(|e| format!("cannot open a window: {}", short(&e.to_string())))?);
        }
        let context = self.context.as_ref().ok_or("no display context")?;
        let surface = softbuffer::Surface::new(context, window.clone()).map_err(|e| format!("cannot open a window: {}", short(&e.to_string())))?;
        window.request_redraw();
        self.windows.insert(window.id(), OsWindow { window, surface, gui_id: request.gui_id, cursor: (0.0, 0.0), mods: 0, shown: (0, 0) });
        if let Some(hook) = HOOK.get() {
            hook.windows.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }

    /// Lays out and paints every window whose tree changed, and shows it.
    fn present_all(&mut self) {
        for os in self.windows.values_mut() {
            super::window::with_surface(os.gui_id, |surface| {
                if !surface.present().is_empty() {
                    blit(os, surface);
                }
            });
        }
    }
}

impl ApplicationHandler<()> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        self.create_pending(el);
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        self.create_pending(el);
        self.present_all();
    }

    fn window_event(&mut self, _el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(os) = self.windows.get_mut(&id) else { return };
        let mut inputs: Vec<Input> = Vec::new();
        match event {
            WindowEvent::CloseRequested => inputs.push(Input::Close),
            WindowEvent::Resized(size) => {
                inputs.push(Input::Resize { width: size.width.max(1), height: size.height.max(1) });
                os.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                super::window::with_surface(os.gui_id, |surface| {
                    surface.present();
                    blit(os, surface);
                });
            }
            WindowEvent::CursorMoved { position, .. } => {
                os.cursor = (position.x as f32, position.y as f32);
                inputs.push(Input::PointerMove { x: os.cursor.0, y: os.cursor.1 });
            }
            WindowEvent::CursorLeft { .. } => inputs.push(Input::PointerLeave),
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                let (x, y) = os.cursor;
                inputs.push(if state == ElementState::Pressed { Input::PointerDown { x, y } } else { Input::PointerUp { x, y } });
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (-x * LINE_PIXELS, -y * LINE_PIXELS),
                    MouseScrollDelta::PixelDelta(p) => (-p.x as f32, -p.y as f32),
                };
                inputs.push(Input::Wheel { x: os.cursor.0, y: os.cursor.1, dx, dy });
            }
            WindowEvent::ModifiersChanged(m) => {
                let s = m.state();
                os.mods = [(s.shift_key(), keys::SHIFT), (s.control_key(), keys::CTRL), (s.alt_key(), keys::ALT), (s.super_key(), keys::META)]
                    .into_iter()
                    .filter(|(held, _)| *held)
                    .fold(0, |bits, (_, bit)| bits | bit);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let down = event.state == ElementState::Pressed;
                if let Some(code) = key_code(&event.logical_key) {
                    inputs.push(Input::Key { code, mods: os.mods, down });
                }
                if down && os.mods & (keys::CTRL | keys::META) == 0 && let Some(text) = event.text {
                    inputs.push(Input::Text(text.to_string()));
                }
            }
            _ => {}
        }
        for input in inputs {
            super::window::feed(os.gui_id, &input);
        }
    }

    fn user_event(&mut self, _el: &ActiveEventLoop, _event: ()) {}
}

/// Opens a window of `width` by `height` pixels on the home thread and answers once it exists.
pub(super) fn open(gui_id: i64, title: String, width: u32, height: u32) -> Result<(), String> {
    SHELL.with(|cell| -> Result<(), String> {
        let mut shell = cell.borrow_mut();
        if shell.is_none() {
            let event_loop = build_event_loop()?;
            let hook = Arc::new(Hook { proxy: event_loop.create_proxy(), woken: AtomicBool::new(false), windows: AtomicUsize::new(0) });
            let _ = HOOK.set(hook.clone());
            contracts::install_home_loop(hook);
            *shell = Some(Shell { event_loop, app: App::default() });
        }
        shell.as_mut().ok_or("no window system")?.app.pending.push(Request { gui_id, title, width, height });
        Ok(())
    })?;
    let started = std::time::Instant::now();
    loop {
        pump(Some(Duration::from_millis(5)));
        let done = SHELL.with(|cell| cell.borrow_mut().as_mut().and_then(|s| s.app.created.remove(&gui_id)));
        match done {
            Some(result) => return result,
            None if started.elapsed() > OPEN_TIMEOUT => return Err("the window system did not open the window".to_string()),
            None => {}
        }
    }
}

fn resize_direction(code: i64) -> Result<ResizeDirection, String> {
    Ok(match code {
        0 => ResizeDirection::North,
        1 => ResizeDirection::South,
        2 => ResizeDirection::East,
        3 => ResizeDirection::West,
        4 => ResizeDirection::NorthEast,
        5 => ResizeDirection::NorthWest,
        6 => ResizeDirection::SouthEast,
        7 => ResizeDirection::SouthWest,
        _ => return Err(format!("no window edge {code}")),
    })
}

/// Runs a frame operation on the display window: 0 drag, 1 resize from edge `arg`, 2 minimize, 3 toggle maximize, 4 ask whether maximized, 5 show the system frame when `arg` is not 0, 6 maximize when `arg` is not 0 and restore otherwise, 7 fullscreen on or off, 8 ask whether fullscreen.
pub(super) fn chrome(gui_id: i64, op: i64, arg: i64) -> Result<i64, String> {
    SHELL.with(|cell| {
        let shell = cell.borrow();
        let os = shell.as_ref().and_then(|s| s.app.windows.values().find(|os| os.gui_id == gui_id)).ok_or("the window is not open")?;
        let window = &os.window;
        match op {
            0 => window.drag_window().map_err(|e| format!("cannot drag the window: {}", short(&e.to_string())))?,
            1 => window.drag_resize_window(resize_direction(arg)?).map_err(|e| format!("cannot resize the window: {}", short(&e.to_string())))?,
            2 => window.set_minimized(true),
            3 => window.set_maximized(!window.is_maximized()),
            4 => return Ok(i64::from(window.is_maximized())),
            5 => window.set_decorations(arg != 0),
            6 => window.set_maximized(arg != 0),
            7 => window.set_fullscreen((arg != 0).then_some(Fullscreen::Borderless(None))),
            8 => return Ok(i64::from(window.fullscreen().is_some())),
            _ => return Err(format!("no window operation {op}")),
        }
        Ok(0)
    })
}

/// Changes the title of the display window.
pub(super) fn set_title(gui_id: i64, title: &str) {
    SHELL.with(|cell| {
        if let Some(os) = cell.borrow().as_ref().and_then(|s| s.app.windows.values().find(|os| os.gui_id == gui_id)) {
            os.window.set_title(title);
        }
    });
}

/// Closes the window on the display.
pub(super) fn close(gui_id: i64) {
    SHELL.with(|cell| {
        if let Some(shell) = cell.borrow_mut().as_mut() {
            let before = shell.app.windows.len();
            shell.app.windows.retain(|_, os| os.gui_id != gui_id);
            if let (Some(hook), true) = (HOOK.get(), shell.app.windows.len() < before) {
                hook.windows.fetch_sub(1, Ordering::SeqCst);
            }
        }
    });
}

/// Shows the painted pixels of window `gui_id` on its display window.
pub(super) fn show(gui_id: i64, surface: &Surface) {
    SHELL.with(|cell| {
        if let Some(shell) = cell.borrow_mut().as_mut()
            && let Some(os) = shell.app.windows.values_mut().find(|os| os.gui_id == gui_id)
        {
            blit(os, surface);
        }
    });
}
