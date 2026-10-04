//! A tree with its painter and pixel buffer: everything a window shows, without the window.

use tiny_skia::Pixmap;

use crate::geom::Rect;
use crate::paint::Painter;
use crate::tree::{Input, Kind, Tree, UiEvent};

pub struct Surface {
    pub tree: Tree,
    pub painter: Painter,
    pixmap: Pixmap,
}

/// The largest side a surface may have.
pub const MAX_SIDE: u32 = 16_384;

impl Surface {
    /// A surface of `width` by `height` pixels, each between 1 and [`MAX_SIDE`].
    pub fn new(width: u32, height: u32, painter: Painter) -> Result<Surface, String> {
        let pixmap = Self::pixmap(width, height)?;
        Ok(Surface { tree: Tree::new(width as f32, height as f32), painter, pixmap })
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
            _ => {
                let events = self.tree.input(input);
                self.place_caret(input);
                events
            }
        }
    }

    /// A press on a focused input puts the caret under the pointer; a drag with it held extends the selection.
    fn place_caret(&mut self, input: &Input) {
        let (x, dragging) = match *input {
            Input::PointerDown { x, .. } => (x, false),
            Input::PointerMove { x, .. } => (x, true),
            _ => return,
        };
        let Some(node) = self.tree.focus().filter(|n| self.tree.kind(*n) == Some(Kind::Input)) else { return };
        if dragging && !self.tree.is_pressed(node) {
            return;
        }
        if let Some(index) = self.painter.input_index_at(&self.tree, node, x) {
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
