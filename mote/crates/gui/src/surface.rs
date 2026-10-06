//! A tree with its painter and pixel buffer: everything a window shows, without the window.

use tiny_skia::Pixmap;

use crate::geom::Rect;
use crate::keys;
use crate::paint::Painter;
use crate::tree::{Input, Kind, Tree, UiEvent};

pub struct Surface {
    pub tree: Tree,
    pub painter: Painter,
    pixmap: Pixmap,
    /// The node and link id the pointer went down on, until it comes up.
    press_link: Option<(u32, u32)>,
}

/// The largest side a surface may have.
pub const MAX_SIDE: u32 = 16_384;

impl Surface {
    /// A surface of `width` by `height` pixels, each between 1 and [`MAX_SIDE`].
    pub fn new(width: u32, height: u32, painter: Painter) -> Result<Surface, String> {
        let pixmap = Self::pixmap(width, height)?;
        Ok(Surface { tree: Tree::new(width as f32, height as f32), painter, pixmap, press_link: None })
    }

    fn pixmap(width: u32, height: u32) -> Result<Pixmap, String> {
        if !(1..=MAX_SIDE).contains(&width) || !(1..=MAX_SIDE).contains(&height) {
            return Err(format!("a window is 1 to {MAX_SIDE} pixels on a side, not {width} by {height}"));
        }
        Pixmap::new(width, height).ok_or_else(|| format!("no memory for a {width} by {height} window"))
    }

    pub fn width(&self) -> u32 {
        self.pixmap.width()
    }

    pub fn height(&self) -> u32 {
        self.pixmap.height()
    }

    /// Changes the size; everything repaints at the next [`present`](Self::present).
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        if (width, height) == (self.width(), self.height()) {
            return Ok(());
        }
        self.pixmap = Self::pixmap(width, height)?;
        self.tree.set_viewport(width as f32, height as f32);
        Ok(())
    }

    /// Applies one input and answers the events it raises; a resize that cannot be honoured is dropped.
    pub fn input(&mut self, input: &Input) -> Vec<UiEvent> {
        match *input {
            Input::Resize { width, height } => match self.resize(width, height) {
                Ok(()) => vec![UiEvent::Resize(width, height)],
                Err(_) => Vec::new(),
            },
            Input::Close => vec![UiEvent::Close],
            Input::Wheel { x, y, dy, .. } if self.scroll_area(x, y, dy) => Vec::new(),
            _ => {
                let mut events = self.tree.input(input);
                self.place_caret(input);
                self.area_key(input);
                self.follow_caret();
                self.link_events(input, &mut events);
                events
            }
        }
    }

    /// The focused text area, if a text area has the focus.
    fn focused_area(&self) -> Option<u32> {
        self.tree.focus().filter(|n| self.tree.kind(*n) == Some(Kind::TextArea))
    }

    /// A wheel turn over a text area that has text to scroll to moves its text; true if it did.
    fn scroll_area(&mut self, x: f32, y: f32, dy: f32) -> bool {
        let Some(node) = self.tree.hit(x, y).filter(|n| self.tree.kind(*n) == Some(Kind::TextArea)) else { return false };
        let Some(layout) = self.painter.area_layout(&self.tree, node) else { return false };
        let before = self.tree.scroll_top(node).clamp(0.0, layout.max_top());
        let after = (before + dy).clamp(0.0, layout.max_top());
        self.tree.set_scroll_top(node, after);
        after != before
    }

    /// Up, Down, Home and End in a focused text area move the caret by wrapped line.
    fn area_key(&mut self, input: &Input) {
        let Input::Key { code, mods, down: true } = *input else { return };
        let Some(node) = self.focused_area() else { return };
        let command = mods & (keys::CTRL | keys::META) != 0;
        if !matches!(code, keys::UP | keys::DOWN) && (command || !matches!(code, keys::HOME | keys::END)) {
            return;
        }
        let Some(layout) = self.painter.area_layout(&self.tree, node) else { return };
        let Some((caret, _)) = self.tree.selection(node) else { return };
        let (line, x) = layout.locate(caret);
        let want = self.tree.want_x(node).unwrap_or(x);
        let extend = mods & keys::SHIFT != 0;
        let (to, keep) = match code {
            keys::UP if line == 0 => (0, None),
            keys::UP => (layout.index_on(line - 1, want), Some(want)),
            keys::DOWN if line + 1 == layout.lines.len() => (layout.len, None),
            keys::DOWN => (layout.index_on(line + 1, want), Some(want)),
            keys::HOME => (layout.start_of(line), None),
            _ => (layout.end_of(line), None),
        };
        self.tree.move_caret(node, to, extend, keep);
    }

    /// Scrolls a focused text area, if it must, to keep its caret in view.
    fn follow_caret(&mut self) {
        let Some(node) = self.focused_area() else { return };
        let Some(layout) = self.painter.area_layout(&self.tree, node) else { return };
        let Some((caret, _)) = self.tree.selection(node) else { return };
        let (line, _) = layout.locate(caret);
        let (top, bottom) = (layout.lines[line].top, layout.lines[line].top + layout.line_height);
        let now = self.tree.scroll_top(node).clamp(0.0, layout.max_top());
        let next = if top < now { top } else if bottom > now + layout.content.h { bottom - layout.content.h } else { now };
        self.tree.set_scroll_top(node, next.clamp(0.0, layout.max_top()));
    }

    /// A press and release on the same link of a text node raises `Link`.
    fn link_events(&mut self, input: &Input, events: &mut Vec<UiEvent>) {
        match *input {
            Input::PointerDown { x, y } => {
                self.press_link = self.tree.hit(x, y).and_then(|n| self.painter.link_at(&self.tree, n, x, y).map(|l| (n, l)));
            }
            Input::PointerUp { x, y } => {
                let Some((node, link)) = self.press_link.take() else { return };
                if self.tree.hit(x, y) == Some(node) && self.painter.link_at(&self.tree, node, x, y) == Some(link) {
                    events.push(UiEvent::Link(node, link));
                }
            }
            _ => {}
        }
    }

    /// A press on a focused input puts the caret under the pointer; a drag with it held extends the selection.
    fn place_caret(&mut self, input: &Input) {
        let (x, y, dragging) = match *input {
            Input::PointerDown { x, y } => (x, y, false),
            Input::PointerMove { x, y } => (x, y, true),
            _ => return,
        };
        let Some(node) = self.tree.focus().filter(|n| self.tree.kind(*n).is_some_and(Kind::is_input)) else { return };
        if dragging && !self.tree.is_pressed(node) {
            return;
        }
        if let Some(layout) = self.painter.area_layout(&self.tree, node) {
            let top = self.tree.scroll_top(node).clamp(0.0, layout.max_top());
            let index = layout.index_at(x - layout.content.x, y - layout.content.y + top);
            self.tree.set_caret(node, index, dragging);
        } else if let Some(index) = self.painter.input_index_at(&self.tree, node, x) {
            self.tree.set_caret(node, index, dragging);
        }
    }

    /// Lays out what changed and paints the damage; returns the rectangles painted.
    pub fn present(&mut self) -> Vec<Rect> {
        self.tree.layout(&mut self.painter);
        let damage = self.tree.take_damage();
        self.painter.paint(&mut self.tree, &mut self.pixmap, &damage);
        damage
    }

    /// The pixels as premultiplied RGBA bytes, row by row.
    pub fn pixels(&self) -> &[u8] {
        self.pixmap.data()
    }

    /// A pixel as packed `0xRRGGBBAA`, or `None` outside the surface.
    pub fn pixel(&self, x: u32, y: u32) -> Option<u32> {
        if x >= self.width() || y >= self.height() {
            return None;
        }
        let p = self.pixmap.pixel(x, y)?;
        Some(u32::from(p.red()) << 24 | u32::from(p.green()) << 16 | u32::from(p.blue()) << 8 | u32::from(p.alpha()))
    }

    /// The picture as a PNG.
    pub fn png(&self) -> Result<Vec<u8>, String> {
        self.pixmap.encode_png().map_err(|e| format!("cannot encode the picture: {e}"))
    }
}
