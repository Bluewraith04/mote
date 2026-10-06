//! Painting: tiny-skia fills and borders, cosmic-text text, and text measuring for layout.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Read;

use cosmic_text::{Align, Attrs, Buffer, Color as TextColor, Family, FontSystem, Metrics, Shaping, Style, SwashCache, UnderlineStyle, Weight, fontdb};
use tiny_skia::{BlendMode, Color, FillRule, FilterQuality, Mask, Paint, PathBuilder, Pixmap, PixmapPaint, Transform};

mod area;

pub use area::{AreaLayout, AreaLine};

use crate::geom::Rect;
use crate::style::{StyleDef, TextAlign};
use crate::tree::{Kind, Measure, MeasureNode, Run, Tree};

const SANS_GZ: &[u8] = include_bytes!("../fonts/DejaVuSans.ttf.gz");
const SANS_BOLD_GZ: &[u8] = include_bytes!("../fonts/DejaVuSans-Bold.ttf.gz");
const MONO_GZ: &[u8] = include_bytes!("../fonts/DejaVuSansMono.ttf.gz");
const SANS_FAMILY: &str = "DejaVu Sans";
const MONO_FAMILY: &str = "DejaVu Sans Mono";
const MEASURE_CACHE_LIMIT: usize = 4096;
const SHAPE_CACHE_LIMIT: usize = 2048;
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

/// What a measured size depends on: the text, its runs, font size, weight, line height and the width it wraps to.
type MeasureKey = (String, Vec<Run>, u32, u16, u32, u32);

/// Paints a [`Tree`] into a pixmap and gives its text a size.
pub struct Painter {
    fonts: FontSystem,
    swash: SwashCache,
    images: HashMap<u32, Pixmap>,
    measured: HashMap<MeasureKey, [f32; 2]>,
    /// Each painted text node's shaped buffer, with the hash of what it was shaped from.
    shapes: HashMap<u32, (u64, Buffer)>,
    /// What the window is cleared to before anything paints.
    pub background: u32,
}

fn text_color(rgba: u32) -> TextColor {
    TextColor::rgba((rgba >> 24) as u8, (rgba >> 16) as u8, (rgba >> 8) as u8, rgba as u8)
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

/// The pixel columns or rows a span covers: those whose centers lie inside it.
fn span(lo: f32, hi: f32) -> (i32, i32) {
    ((lo - 0.5).ceil() as i32, (hi - 0.5).ceil() as i32)
}

/// A pixel rectangle, right and bottom excluded, that painting is limited to.
#[derive(Clone, Copy)]
struct Clip {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl Clip {
    /// The pixels of `pixmap` that `r` covers.
    fn new(r: &Rect, pixmap: &Pixmap) -> Clip {
        let (x0, x1) = span(r.x, r.right());
        let (y0, y1) = span(r.y, r.bottom());
        Clip { x0: x0.max(0), y0: y0.max(0), x1: x1.min(pixmap.width() as i32), y1: y1.min(pixmap.height() as i32) }
    }
}

/// The straight color of `rgba` and its alpha from 0 to 255 after `opacity`.
fn ink(rgba: u32, opacity: f32) -> ([u8; 3], u32) {
    ([(rgba >> 24) as u8, (rgba >> 16) as u8, (rgba >> 8) as u8], ((rgba & 0xff) as f32 * opacity.clamp(0.0, 1.0)).round() as u32)
}

/// Blends a solid color over the rectangle `x`, `y`, `w`, `h`, inside `clip`, with no mask.
fn blend_rect(pixmap: &mut Pixmap, clip: Clip, rect: [f32; 4], rgb: [u8; 3], alpha: u32) {
    let [x, y, w, h] = rect;
    let (x0, x1) = span(x, x + w);
    let (y0, y1) = span(y, y + h);
    let (x0, x1, y0, y1) = (x0.max(clip.x0), x1.min(clip.x1), y0.max(clip.y0), y1.min(clip.y1));
    if alpha == 0 || x0 >= x1 || y0 >= y1 {
        return;
    }
    let stride = pixmap.width() as usize;
    let data = pixmap.data_mut();
    let inv = 255 - alpha;
    let src = [u32::from(rgb[0]) * alpha, u32::from(rgb[1]) * alpha, u32::from(rgb[2]) * alpha];
    for row in y0..y1 {
        let start = (row as usize * stride + x0 as usize) * 4;
        for px in data[start..start + (x1 - x0) as usize * 4].as_chunks_mut::<4>().0 {
            for c in 0..3 {
                px[c] = ((src[c] + u32::from(px[c]) * inv + 127) / 255) as u8;
            }
            px[3] = ((alpha * 255 + u32::from(px[3]) * inv + 127) / 255) as u8;
        }
    }
}

/// What a shaped buffer depends on: the text, its runs, the style's font and color, and the width it wraps to.
fn shape_key(text: &str, style: &StyleDef, runs: &[Run], width: Option<f32>) -> u64 {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    runs.hash(&mut h);
    (style.font_size.to_bits(), style.font_weight, style.line_height.to_bits(), style.color, style.text_align as u8).hash(&mut h);
    width.map_or(u32::MAX, f32::to_bits).hash(&mut h);
    h.finish()
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
        db.load_font_data(gunzip(MONO_GZ));
        if !system {
            db.set_sans_serif_family(SANS_FAMILY);
        }
        Painter {
            fonts: FontSystem::new_with_locale_and_db("en-US".to_string(), db),
            swash: SwashCache::new(),
            images: HashMap::new(),
            measured: HashMap::new(),
            shapes: HashMap::new(),
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
        Attrs::new().family(Family::SansSerif).weight(Weight(style.font_weight)).color(text_color(style.color))
    }

    /// The attributes of run number `index`, which the glyphs carry back as their metadata.
    fn run_attrs(style: &StyleDef, run: &Run, index: usize) -> Attrs<'static> {
        let ink = if run.color != 0 { run.color } else { style.color };
        let mut attrs = Self::attrs(style).metadata(index).color(text_color(ink));
        if run.weight != 0 {
            attrs = attrs.weight(Weight(run.weight));
        }
        if run.italic {
            attrs = attrs.style(Style::Italic);
        }
        if run.mono {
            attrs = attrs.family(Family::Name(MONO_FAMILY));
        }
        if run.underline {
            attrs = attrs.underline(UnderlineStyle::Single).underline_color(text_color(ink));
        }
        if run.strike {
            attrs = attrs.strikethrough().strikethrough_color(text_color(ink));
        }
        attrs
    }

    fn metrics(style: &StyleDef) -> Metrics {
        let size = style.font_size.max(1.0);
        Metrics::new(size, if style.line_height > 0.0 { style.line_height } else { (size * 1.2).ceil() })
    }

    /// A shaped buffer for `text`, split into `runs` when it has any, wrapping at `width`.
    fn shaped(&mut self, text: &str, style: &StyleDef, runs: &[Run], width: Option<f32>) -> Buffer {
        let mut buffer = Buffer::new(&mut self.fonts, Self::metrics(style));
        buffer.set_size(width, None);
        let align = match style.text_align {
            TextAlign::Left => None,
            TextAlign::Center => Some(Align::Center),
            TextAlign::Right => Some(Align::Right),
        };
        let base = Self::attrs(style);
        if runs.is_empty() {
            buffer.set_text(text, &base, Shaping::Advanced, align);
        } else {
            let mut at = 0;
            let mut spans = Vec::with_capacity(runs.len());
            for (i, run) in runs.iter().enumerate() {
                spans.push((&text[at..at + run.len], Self::run_attrs(style, run, i)));
                at += run.len;
            }
            buffer.set_rich_text(spans, &base, Shaping::Advanced, align);
        }
        buffer.shape_until_scroll(&mut self.fonts, false);
        buffer
    }

    /// The buffer of a node's text: the one kept from the last paint if `key` still matches, else a new one.
    fn take_shaped(&mut self, node: u32, key: u64, text: &str, style: &StyleDef, runs: &[Run], width: Option<f32>) -> Buffer {
        match self.shapes.remove(&node) {
            Some((kept, buffer)) if kept == key => buffer,
            _ => self.shaped(text, style, runs, width),
        }
    }

    /// Keeps a node's buffer for the next paint.
    fn keep_shaped(&mut self, node: u32, key: u64, buffer: Buffer) {
        if self.shapes.len() >= SHAPE_CACHE_LIMIT {
            self.shapes.clear();
        }
        self.shapes.insert(node, (key, buffer));
    }

    /// Paints every damaged rectangle of the tree into `pixmap`, which is the window's size.
    pub fn paint(&mut self, tree: &mut Tree, pixmap: &mut Pixmap, damage: &[Rect]) {
        for id in tree.take_removed() {
            self.images.remove(&id);
            self.shapes.remove(&id);
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
        let kind = tree.kind(node);
        let draws_shape = style.background & 0xff != 0 || (style.border.iter().any(|b| *b > 0.0) && style.border_color & 0xff != 0) || kind == Some(Kind::Image);
        let clipped = draws_shape && !visible.covers(&rect);
        if clipped {
            masks.ensure(visible);
        }
        let mask = if clipped { masks.peek(visible) } else { None };
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
        if kind == Some(Kind::Image) {
            self.paint_image(pixmap, mask, node, &rect, style.opacity);
        }
        match kind {
            Some(Kind::TextArea) => self.paint_area(tree, pixmap, node, &style, visible),
            Some(Kind::Text | Kind::Button | Kind::Input) => self.paint_text(tree, pixmap, node, &style, visible),
            _ => {}
        }
    }

    /// The x of each character boundary of one line of text, the end of the line last.
    fn caret_xs(&mut self, text: &str, style: &StyleDef) -> Vec<f32> {
        let buffer = self.shaped(text, style, &[], None);
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

    /// How far down a text node's lines sit in its content box: buttons and inputs centre them.
    fn text_dy(kind: Option<Kind>, box_height: f32, text_height: f32) -> f32 {
        if matches!(kind, Some(Kind::Button | Kind::Input)) { ((box_height - text_height) / 2.0).max(0.0) } else { 0.0 }
    }

    /// The link id under window point `x`, `y` in a text node with link runs.
    pub fn link_at(&mut self, tree: &Tree, node: u32, x: f32, y: f32) -> Option<u32> {
        let runs = tree.runs(node);
        if runs.iter().all(|r| r.link == 0) {
            return None;
        }
        let style = tree.painted_style(node);
        let rect = tree.rect(node);
        let [left, top, right, bottom] = style.insets(rect.w);
        let content = Rect::new(rect.x + left, rect.y + top, rect.w - left - right, rect.h - top - bottom);
        if content.is_empty() {
            return None;
        }
        let buffer = self.shaped(tree.text(node).ok()?, &style, runs, Some(content.w));
        let lines = buffer.layout_runs().count().max(1) as f32;
        let dy = Self::text_dy(tree.kind(node), content.h, lines * Self::metrics(&style).line_height);
        let (lx, ly) = (x - content.x, y - content.y - dy);
        let line = buffer.layout_runs().find(|run| ly >= run.line_top && ly < run.line_top + run.line_height)?;
        let glyph = line.glyphs.iter().find(|g| lx >= g.x && lx < g.x + g.w)?;
        runs.get(glyph.metadata).map(|r| r.link).filter(|l| *l != 0)
    }

    fn paint_text(&mut self, tree: &Tree, pixmap: &mut Pixmap, node: u32, style: &StyleDef, visible: &Rect) {
        let text = tree.text(node).unwrap_or("");
        let rect = tree.rect(node);
        let [left, top, right, bottom] = style.insets(rect.w);
        let content = Rect::new(rect.x + left, rect.y + top, rect.w - left - right, rect.h - top - bottom);
        if content.is_empty() {
            return;
        }
        let geometry = self.input_geometry(tree, node);
        let shift = geometry.as_ref().map_or(0.0, |g| g.shift);
        let runs = tree.runs(node);
        let wrap = if geometry.is_some() { None } else { Some(content.w) };
        let key = shape_key(text, style, runs, wrap);
        let mut buffer = self.take_shaped(node, key, text, style, runs, wrap);
        let lines = if geometry.is_some() { 1.0 } else { buffer.layout_runs().count().max(1) as f32 };
        let height = lines * Self::metrics(style).line_height;
        let dy = Self::text_dy(tree.kind(node), content.h, height);
        let clip = Clip::new(&visible.intersect(&content), pixmap);
        let (ox, oy) = (content.x - shift, content.y + dy);
        let opacity = style.opacity.clamp(0.0, 1.0);
        if let (Some(g), Some((caret, anchor)), true) = (&geometry, tree.selection(node), tree.focus() == Some(node)) {
            let (lo, hi) = (caret.min(anchor), caret.max(anchor));
            if lo != hi {
                let (rgb, alpha) = ink(SELECTION_COLOR, 1.0);
                blend_rect(pixmap, clip, [ox + g.xs[lo], oy, g.xs[hi] - g.xs[lo], height], rgb, alpha);
            }
        }
        if !text.is_empty() {
            let default = text_color(style.color);
            buffer.draw(&mut self.fonts, &mut self.swash, default, |x, y, w, h, c| {
                let alpha = (f32::from(c.a()) * opacity).round() as u32;
                blend_rect(pixmap, clip, [ox + x as f32, oy + y as f32, w as f32, h as f32], [c.r(), c.g(), c.b()], alpha);
            });
        }
        if let (Some(g), Some((caret, _)), true) = (&geometry, tree.selection(node), tree.focus() == Some(node)) {
            let (rgb, alpha) = ink(style.color, style.opacity);
            blend_rect(pixmap, clip, [ox + g.xs[caret], oy, CARET_WIDTH, height], rgb, alpha);
        }
        self.keep_shaped(node, key, buffer);
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
        let key = (node.text.to_string(), node.runs.to_vec(), style.font_size.to_bits(), style.font_weight, style.line_height.to_bits(), max_width.map_or(u32::MAX, f32::to_bits));
        if let Some(size) = self.measured.get(&key) {
            return *size;
        }
        let buffer = self.shaped(node.text, style, node.runs, max_width);
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
