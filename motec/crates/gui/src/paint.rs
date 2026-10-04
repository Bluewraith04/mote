//! Painting: tiny-skia fills and borders, cosmic-text text, and text measuring for layout.

use std::collections::HashMap;
use std::io::Read;

use cosmic_text::{Align, Attrs, Buffer, Color as TextColor, Family, FontSystem, Metrics, Shaping, SwashCache, Weight, fontdb};
use tiny_skia::{BlendMode, Color, FillRule, FilterQuality, Mask, Paint, PathBuilder, Pixmap, PixmapPaint, Transform};

use crate::geom::Rect;
use crate::style::{StyleDef, TextAlign};
use crate::tree::{Kind, Measure, MeasureNode, Tree};

const SANS_GZ: &[u8] = include_bytes!("../fonts/DejaVuSans.ttf.gz");
const SANS_BOLD_GZ: &[u8] = include_bytes!("../fonts/DejaVuSans-Bold.ttf.gz");
const SANS_FAMILY: &str = "DejaVu Sans";
const MEASURE_CACHE_LIMIT: usize = 4096;
const SCROLLBAR_WIDTH: f32 = 6.0;
const SCROLLBAR_MIN_THUMB: f32 = 20.0;
const SCROLLBAR_COLOR: u32 = 0x00000059;
const SELECTION_COLOR: u32 = 0x3390ff66;
const CARET_WIDTH: f32 = 1.5;

/// A bundled font, which is stored gzipped.
fn gunzip(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes).read_to_end(&mut out).expect("the bundled font is valid gzip");
    out
}

/// An input's text placement; see [`Painter::input_geometry`].
pub struct InputGeometry {
    pub content: Rect,
    pub xs: Vec<f32>,
    pub shift: f32,
}

/// Paints a [`Tree`] into a pixmap and gives its text a size.
pub struct Painter {
    fonts: FontSystem,
    swash: SwashCache,
    images: HashMap<u32, Pixmap>,
    measured: HashMap<(String, u32, u16, u32, u32), [f32; 2]>,
    /// What the window is cleared to before anything paints.
    pub background: u32,
}

fn color(rgba: u32, opacity: f32) -> Color {
    let a = (rgba & 0xff) as f32 / 255.0 * opacity.clamp(0.0, 1.0);
    Color::from_rgba(((rgba >> 24) & 0xff) as f32 / 255.0, ((rgba >> 16) & 0xff) as f32 / 255.0, ((rgba >> 8) & 0xff) as f32 / 255.0, a).unwrap_or(Color::TRANSPARENT)
}

fn solid(c: Color, anti_alias: bool) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(c);
    p.anti_alias = anti_alias;
    p
}

fn tiny(r: &Rect) -> Option<tiny_skia::Rect> {
    tiny_skia::Rect::from_xywh(r.x, r.y, r.w, r.h)
}

/// A rounded rectangle as four corner curves.
fn rounded(pb: &mut PathBuilder, r: &Rect, radius: f32) {
    let rad = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    if rad <= 0.0 {
        if let Some(t) = tiny(r) {
            pb.push_rect(t);
        }
        return;
    }
    const K: f32 = 0.552_284_8;
    let (x, y, right, bottom) = (r.x, r.y, r.right(), r.bottom());
    let c = rad * K;
    pb.move_to(x + rad, y);
    pb.line_to(right - rad, y);
    pb.cubic_to(right - rad + c, y, right, y + rad - c, right, y + rad);
    pb.line_to(right, bottom - rad);
    pb.cubic_to(right, bottom - rad + c, right - rad + c, bottom, right - rad, bottom);
    pb.line_to(x + rad, bottom);
    pb.cubic_to(x + rad - c, bottom, x, bottom - rad + c, x, bottom - rad);
    pb.line_to(x, y + rad);
    pb.cubic_to(x, y + rad - c, x + rad - c, y, x + rad, y);
    pb.close();
}

/// Clip masks for rectangles, built once per paint call.
struct Masks {
    width: u32,
    height: u32,
    made: HashMap<[u32; 4], Option<Mask>>,
}

fn mask_key(r: &Rect) -> [u32; 4] {
    [r.x.to_bits(), r.y.to_bits(), r.w.to_bits(), r.h.to_bits()]
}

impl Masks {
    /// Builds the mask for `r` if it has none yet.
    fn ensure(&mut self, r: &Rect) {
        let (w, h) = (self.width, self.height);
        self.made.entry(mask_key(r)).or_insert_with(|| {
            let mut mask = Mask::new(w, h)?;
            let path = PathBuilder::from_rect(tiny(r)?);
            mask.fill_path(&path, FillRule::Winding, false, Transform::identity());
            Some(mask)
        });
    }

    /// The mask [`ensure`](Self::ensure) built for `r`.
    fn peek(&self, r: &Rect) -> Option<&Mask> {
        self.made.get(&mask_key(r)).and_then(Option::as_ref)
    }
}

impl Painter {
    /// A painter with the bundled fonts and the system's.
    pub fn new() -> Painter {
        Painter::with_system_fonts(true)
    }

    /// A painter with only the bundled fonts, so output does not depend on the machine.
    pub fn bundled_only() -> Painter {
        Painter::with_system_fonts(false)
    }

    fn with_system_fonts(system: bool) -> Painter {
        let mut db = fontdb::Database::new();
        if system {
            db.load_system_fonts();
        }
        db.load_font_data(gunzip(SANS_GZ));
        db.load_font_data(gunzip(SANS_BOLD_GZ));
        if !system {
            db.set_sans_serif_family(SANS_FAMILY);
        }
        Painter {
            fonts: FontSystem::new_with_locale_and_db("en-US".to_string(), db),
            swash: SwashCache::new(),
            images: HashMap::new(),
            measured: HashMap::new(),
            background: 0xffffffff,
        }
    }

    /// Decodes a PNG for an image node and gives the node that pixel size unless its style sets one.
    pub fn load_image(&mut self, tree: &mut Tree, node: u32, png: &[u8]) -> Result<(), String> {
        let pixmap = Pixmap::decode_png(png).map_err(|e| format!("not a PNG image: {e}"))?;
        tree.set_image_size(node, pixmap.width() as f32, pixmap.height() as f32)?;
        self.images.insert(node, pixmap);
        Ok(())
    }

    fn attrs(style: &StyleDef) -> Attrs<'static> {
        Attrs::new().family(Family::SansSerif).weight(Weight(style.font_weight))
    }

    fn metrics(style: &StyleDef) -> Metrics {
        let size = style.font_size.max(1.0);
        Metrics::new(size, if style.line_height > 0.0 { style.line_height } else { (size * 1.2).ceil() })
    }

    /// A shaped buffer for `text` wrapping at `width`.
    fn shaped(&mut self, text: &str, style: &StyleDef, width: Option<f32>) -> Buffer {
        let mut buffer = Buffer::new(&mut self.fonts, Self::metrics(style));
        buffer.set_size(width, None);
        let align = match style.text_align {
            TextAlign::Left => None,
            TextAlign::Center => Some(Align::Center),
            TextAlign::Right => Some(Align::Right),
        };
        buffer.set_text(text, &Self::attrs(style), Shaping::Advanced, align);
        buffer.shape_until_scroll(&mut self.fonts, false);
        buffer
    }

    /// Paints every damaged rectangle of the tree into `pixmap`, which is the window's size.
    pub fn paint(&mut self, tree: &mut Tree, pixmap: &mut Pixmap, damage: &[Rect]) {
        for id in tree.take_removed() {
            self.images.remove(&id);
        }
        let mut masks = Masks { width: pixmap.width(), height: pixmap.height(), made: HashMap::new() };
        for d in damage {
            let Some(region) = tiny(d) else { continue };
            pixmap.fill_rect(region, &solid(color(self.background, 1.0), false), Transform::identity(), None);
            let order: Vec<u32> = tree.paint_order().to_vec();
            for &node in &order {
                let visible = tree.visible(node).intersect(d);
                if visible.is_empty() {
                    continue;
                }
                self.paint_node(tree, pixmap, &mut masks, node, &visible);
            }
            for &node in &order {
                let visible = tree.clip(node).intersect(d);
                if !visible.is_empty() {
                    self.paint_scrollbar(tree, pixmap, &mut masks, node, &visible);
                }
            }
        }
    }

    fn paint_node(&mut self, tree: &Tree, pixmap: &mut Pixmap, masks: &mut Masks, node: u32, visible: &Rect) {
        let style = tree.painted_style(node);
        let rect = tree.rect(node);
        masks.ensure(visible);
        let mask = masks.peek(visible);
        if style.background & 0xff != 0 {
            let mut pb = PathBuilder::new();
            rounded(&mut pb, &rect, style.radius);
            if let Some(path) = pb.finish() {
                pixmap.fill_path(&path, &solid(color(style.background, style.opacity), true), FillRule::Winding, Transform::identity(), mask);
            }
        }
        if style.border.iter().any(|b| *b > 0.0) && style.border_color & 0xff != 0 {
            let inner = Rect::new(rect.x + style.border[3], rect.y + style.border[0], rect.w - style.border[3] - style.border[1], rect.h - style.border[0] - style.border[2]);
            let widest = style.border.iter().copied().fold(0.0, f32::max);
            let mut pb = PathBuilder::new();
            rounded(&mut pb, &rect, style.radius);
            if !inner.is_empty() {
                rounded(&mut pb, &inner, (style.radius - widest).max(0.0));
            }
            if let Some(path) = pb.finish() {
                pixmap.fill_path(&path, &solid(color(style.border_color, style.opacity), true), FillRule::EvenOdd, Transform::identity(), mask);
            }
        }
        if tree.kind(node) == Some(Kind::Image) {
            self.paint_image(pixmap, mask, node, &rect, style.opacity);
        }
        if matches!(tree.kind(node), Some(Kind::Text | Kind::Button | Kind::Input)) {
            self.paint_text(tree, pixmap, masks, node, &style, visible);
        }
    }

    /// The x of each character boundary of one line of text, the end of the line last.
    fn caret_xs(&mut self, text: &str, style: &StyleDef) -> Vec<f32> {
        let buffer = self.shaped(text, style, None);
        let mut line_w = 0.0f32;
        let mut glyphs: Vec<(usize, usize, f32, f32)> = Vec::new();
        for run in buffer.layout_runs() {
            line_w = line_w.max(run.line_w);
            glyphs.extend(run.glyphs.iter().map(|g| (g.start, g.end, g.x, g.w)));
        }
        let mut xs: Vec<f32> = text
            .char_indices()
            .map(|(byte, _)| {
                glyphs.iter().find(|(start, end, ..)| *start <= byte && byte < *end).map_or(line_w, |(start, end, x, w)| x + w * (byte - start) as f32 / (end - start).max(1) as f32)
            })
            .collect();
        xs.push(line_w);
        xs
    }

    /// Where an input's text sits: its content box, the x of each character boundary, and how far the text is shifted left to keep the caret in view.
    pub fn input_geometry(&mut self, tree: &Tree, node: u32) -> Option<InputGeometry> {
        if tree.kind(node) != Some(Kind::Input) {
            return None;
        }
        let style = tree.style(node).clone();
        let rect = tree.rect(node);
        let [left, top, right, bottom] = style.insets(rect.w);
        let content = Rect::new(rect.x + left, rect.y + top, rect.w - left - right, rect.h - top - bottom);
        let xs = self.caret_xs(tree.text(node).unwrap_or(""), &style);
        let caret_x = tree.selection(node).and_then(|(caret, _)| xs.get(caret).copied()).unwrap_or(0.0);
        let shift = (caret_x - (content.w - 1.0)).max(0.0);
        Some(InputGeometry { content, xs, shift })
    }

    /// The character boundary of an input nearest to window x `x`.
    pub fn input_index_at(&mut self, tree: &Tree, node: u32, x: f32) -> Option<usize> {
        let g = self.input_geometry(tree, node)?;
        let local = x - g.content.x + g.shift;
        let nearest = g.xs.iter().enumerate().min_by(|a, b| (a.1 - local).abs().total_cmp(&(b.1 - local).abs()));
        nearest.map(|(i, _)| i)
    }

    fn paint_text(&mut self, tree: &Tree, pixmap: &mut Pixmap, masks: &mut Masks, node: u32, style: &StyleDef, visible: &Rect) {
        let text = tree.text(node).unwrap_or("");
        let rect = tree.rect(node);
        let [left, top, right, bottom] = style.insets(rect.w);
        let content = Rect::new(rect.x + left, rect.y + top, rect.w - left - right, rect.h - top - bottom);
        if content.is_empty() {
            return;
        }
        let geometry = self.input_geometry(tree, node);
        let shift = geometry.as_ref().map_or(0.0, |g| g.shift);
        let lines = if geometry.is_some() { 1.0 } else { self.shaped(text, style, Some(content.w)).layout_runs().count().max(1) as f32 };
        let height = lines * Self::metrics(style).line_height;
        let dy = if matches!(tree.kind(node), Some(Kind::Button | Kind::Input)) { ((content.h - height) / 2.0).max(0.0) } else { 0.0 };
        let clip = visible.intersect(&content);
        masks.ensure(&clip);
        let Some(mask) = masks.peek(&clip) else { return };
        let ink = color(style.color, style.opacity);
        let (ox, oy) = (content.x - shift, content.y + dy);
        if let (Some(g), Some((caret, anchor)), true) = (&geometry, tree.selection(node), tree.focus() == Some(node)) {
            let (lo, hi) = (caret.min(anchor), caret.max(anchor));
            if lo != hi {
                let band = Rect::new(ox + g.xs[lo], oy, g.xs[hi] - g.xs[lo], height);
                if let Some(r) = tiny(&band) {
                    pixmap.fill_rect(r, &solid(color(SELECTION_COLOR, 1.0), false), Transform::identity(), Some(mask));
                }
            }
        }
        if !text.is_empty() {
            let mut buffer = self.shaped(text, style, if geometry.is_some() { None } else { Some(content.w) });
            let default = TextColor::rgba(0, 0, 0, 255);
            buffer.draw(&mut self.fonts, &mut self.swash, default, |x, y, w, h, c| {
                let alpha = c.a() as f32 / 255.0 * ink.alpha();
                let Some(px) = Color::from_rgba(ink.red(), ink.green(), ink.blue(), alpha) else { return };
                if let Some(r) = tiny_skia::Rect::from_xywh(ox + x as f32, oy + y as f32, w as f32, h as f32) {
                    pixmap.fill_rect(r, &solid(px, false), Transform::identity(), Some(mask));
                }
            });
        }
        if let (Some(g), Some((caret, _)), true) = (&geometry, tree.selection(node), tree.focus() == Some(node)) {
            let bar = Rect::new(ox + g.xs[caret], oy, CARET_WIDTH, height);
            if let Some(r) = tiny(&bar) {
                pixmap.fill_rect(r, &solid(ink, false), Transform::identity(), Some(mask));
            }
        }
    }

    fn paint_image(&self, pixmap: &mut Pixmap, mask: Option<&Mask>, node: u32, rect: &Rect, opacity: f32) {
        let Some(image) = self.images.get(&node) else { return };
        if rect.is_empty() {
            return;
        }
        let sx = rect.w / image.width() as f32;
        let sy = rect.h / image.height() as f32;
        let paint = PixmapPaint { opacity: opacity.clamp(0.0, 1.0), blend_mode: BlendMode::SourceOver, quality: FilterQuality::Bilinear };
        pixmap.draw_pixmap(0, 0, image.as_ref(), &paint, Transform::from_row(sx, 0.0, 0.0, sy, rect.x, rect.y), mask);
    }

    fn paint_scrollbar(&mut self, tree: &Tree, pixmap: &mut Pixmap, masks: &mut Masks, node: u32, visible: &Rect) {
        let (rect, content, offset) = (tree.rect(node), tree.content_size(node), tree.scroll_offset(node));
        if !tree.is_scrollable(node) || content[1] <= rect.h || rect.h <= 0.0 {
            return;
        }
        let max = content[1] - rect.h;
        let thumb_h = (rect.h * rect.h / content[1]).clamp(SCROLLBAR_MIN_THUMB.min(rect.h), rect.h);
        let y = rect.y + (rect.h - thumb_h) * (offset[1] / max);
        let thumb = Rect::new(rect.right() - SCROLLBAR_WIDTH - 2.0, y, SCROLLBAR_WIDTH, thumb_h);
        let clip = visible.intersect(&tree.visible(node));
        masks.ensure(&clip);
        let mut pb = PathBuilder::new();
        rounded(&mut pb, &thumb, SCROLLBAR_WIDTH / 2.0);
        if let Some(path) = pb.finish() {
            pixmap.fill_path(&path, &solid(color(SCROLLBAR_COLOR, 1.0), true), FillRule::Winding, Transform::identity(), masks.peek(&clip));
        }
    }
}

impl Default for Painter {
    fn default() -> Self {
        Painter::new()
    }
}

impl Measure for Painter {
    fn measure(&mut self, node: &MeasureNode<'_>, max_width: Option<f32>) -> [f32; 2] {
        let style = node.style;
        let max_width = if node.kind == Kind::Input { None } else { max_width };
        let key = (node.text.to_string(), style.font_size.to_bits(), style.font_weight, style.line_height.to_bits(), max_width.map_or(u32::MAX, f32::to_bits));
        if let Some(size) = self.measured.get(&key) {
            return *size;
        }
        let buffer = self.shaped(node.text, style, max_width);
        let line_height = Self::metrics(style).line_height;
        let (mut width, mut lines) = (0.0f32, 0usize);
        for run in buffer.layout_runs() {
            width = width.max(run.line_w);
            lines += 1;
        }
        let size = [width.ceil(), lines.max(1) as f32 * line_height];
        if self.measured.len() >= MEASURE_CACHE_LIMIT {
            self.measured.clear();
        }
        self.measured.insert(key, size);
        size
    }
}
