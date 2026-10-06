//! Text areas: where each line of a wrapped, editable text sits, and how one is painted.

use cosmic_text::Buffer;
use tiny_skia::Pixmap;

use super::{CARET_WIDTH, Clip, Painter, SELECTION_COLOR, blend_rect, ink, shape_key, text_color};
use crate::geom::Rect;
use crate::style::StyleDef;
use crate::tree::{Kind, Tree};

/// A width added to the selection at a line break, so a selected break shows.
const BREAK_WIDTH: f32 = 4.0;

/// One line of a text area as it is wrapped: the characters `first` to `last`, whose boundaries sit at `xs`.
pub struct AreaLine {
    pub top: f32,
    pub first: usize,
    pub last: usize,
    pub xs: Vec<f32>,
}

/// Where a text area's text sits: its content box and each wrapped line.
pub struct AreaLayout {
    pub content: Rect,
    pub line_height: f32,
    pub lines: Vec<AreaLine>,
    /// The number of characters in the text.
    pub len: usize,
}

impl AreaLayout {
    /// The height of all the lines together.
    pub fn height(&self) -> f32 {
        self.lines.last().map_or(0.0, |l| l.top + self.line_height)
    }

    /// How far the text can scroll.
    pub fn max_top(&self) -> f32 {
        (self.height() - self.content.h).max(0.0)
    }

    /// The line the caret at character `index` is on; at a wrap it is the later line.
    pub fn line_of(&self, index: usize) -> usize {
        self.lines.iter().rposition(|l| l.first <= index).unwrap_or(0)
    }

    /// The line number and x of character boundary `index`.
    pub fn locate(&self, index: usize) -> (usize, f32) {
        let at = self.line_of(index);
        let line = &self.lines[at];
        let x = line.xs.get(index.saturating_sub(line.first)).copied().unwrap_or(0.0);
        (at, x)
    }

    /// The boundary of `line` nearest to text x `x`, kept off the end of a wrapped line, where the caret would show on the next.
    pub fn index_on(&self, line: usize, x: f32) -> usize {
        let l = &self.lines[line];
        let nearest = l.xs.iter().enumerate().min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs())).map_or(0, |(i, _)| i);
        self.keep_on(line, l.first + nearest)
    }

    fn keep_on(&self, line: usize, index: usize) -> usize {
        let l = &self.lines[line];
        let wrapped = self.lines.get(line + 1).is_some_and(|next| next.first == l.last);
        if wrapped && index == l.last && index > l.first { index - 1 } else { index }
    }

    /// The first boundary of `line`.
    pub fn start_of(&self, line: usize) -> usize {
        self.lines[line].first
    }

    /// The last boundary of `line` that still shows on it.
    pub fn end_of(&self, line: usize) -> usize {
        self.keep_on(line, self.lines[line].last)
    }

    /// The boundary nearest to the point `x`, `y` of the text, whose top left is the origin.
    pub fn index_at(&self, x: f32, y: f32) -> usize {
        let line = self.lines.iter().rposition(|l| l.top <= y).unwrap_or(0);
        self.index_on(line, x)
    }
}

impl Painter {
    /// The content box and wrapped lines of a text area; `None` for any other node.
    pub fn area_layout(&mut self, tree: &Tree, node: u32) -> Option<AreaLayout> {
        let (layout, buffer, key) = self.shaped_area(tree, node)?;
        self.keep_shaped(node, key, buffer);
        Some(layout)
    }

    /// The layout of a text area with the buffer and key it was laid out from; the caller keeps the buffer.
    fn shaped_area(&mut self, tree: &Tree, node: u32) -> Option<(AreaLayout, Buffer, u64)> {
        if tree.kind(node) != Some(Kind::TextArea) {
            return None;
        }
        let style = tree.style(node).clone();
        let rect = tree.rect(node);
        let [left, top, right, bottom] = style.insets(rect.w);
        let content = Rect::new(rect.x + left, rect.y + top, (rect.w - left - right).max(0.0), (rect.h - top - bottom).max(0.0));
        let text = tree.text(node).ok()?;
        let key = shape_key(text, &style, &[], Some(content.w));
        let buffer = self.take_shaped(node, key, text, &style, &[], Some(content.w));
        let layout = Self::lay_out_area(&buffer, text, &style, content);
        Some((layout, buffer, key))
    }

    fn lay_out_area(buffer: &Buffer, text: &str, style: &StyleDef, content: Rect) -> AreaLayout {
        let line_height = Self::metrics(style).line_height;
        let len = text.chars().count();
        let mut char_at: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
        char_at.push(text.len());
        let index_of = |byte: usize| char_at.partition_point(|b| *b < byte);
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(b, _)| b + 1));
        let mut lines: Vec<AreaLine> = Vec::new();
        for run in buffer.layout_runs() {
            let base = starts.get(run.line_i).copied().unwrap_or(text.len());
            let (from, to) = run.glyphs.iter().fold((usize::MAX, 0), |(lo, hi), g| (lo.min(base + g.start), hi.max(base + g.end)));
            let (from, to) = if from == usize::MAX { (base, base) } else { (from, to) };
            let (first, last) = (index_of(from), index_of(to));
            let xs = (first..=last)
                .map(|c| {
                    let byte = char_at[c];
                    run.glyphs
                        .iter()
                        .find(|g| base + g.start <= byte && byte < base + g.end)
                        .map_or(run.line_w, |g| g.x + g.w * (byte - base - g.start) as f32 / (g.end - g.start).max(1) as f32)
                })
                .collect();
            lines.push(AreaLine { top: run.line_top, first, last, xs });
        }
        let covered = lines.last().map_or(0, |l| l.last);
        if lines.is_empty() || covered < len {
            let top = lines.last().map_or(0.0, |l| l.top + line_height);
            lines.push(AreaLine { top, first: len, last: len, xs: vec![0.0] });
        }
        AreaLayout { content, line_height, lines, len }
    }

    /// Paints a text area: its selection, wrapped text scrolled by its offset, and the caret when it has focus.
    pub(super) fn paint_area(&mut self, tree: &Tree, pixmap: &mut Pixmap, node: u32, style: &StyleDef, visible: &Rect) {
        let Some((layout, mut buffer, key)) = self.shaped_area(tree, node) else { return };
        if layout.content.is_empty() {
            self.keep_shaped(node, key, buffer);
            return;
        }
        let clip = Clip::new(&visible.intersect(&layout.content), pixmap);
        let top = tree.scroll_top(node).clamp(0.0, layout.max_top());
        let (ox, oy) = (layout.content.x, layout.content.y - top);
        let focused = tree.focus() == Some(node);
        let selection = tree.selection(node).filter(|_| focused);
        if let Some((caret, anchor)) = selection {
            let (lo, hi) = (caret.min(anchor), caret.max(anchor));
            for (i, line) in layout.lines.iter().enumerate().filter(|(_, l)| l.last >= lo && l.first <= hi && lo != hi) {
                let (from, to) = (lo.max(line.first), hi.min(line.last));
                let breaks = hi > line.last && layout.lines.get(i + 1).is_some_and(|next| next.first > line.last);
                if from == to && !breaks {
                    continue;
                }
                let (x0, x1) = (line.xs[from - line.first], line.xs[to - line.first] + if breaks { BREAK_WIDTH } else { 0.0 });
                let (rgb, alpha) = ink(SELECTION_COLOR, 1.0);
                blend_rect(pixmap, clip, [ox + x0, oy + line.top, x1 - x0, layout.line_height], rgb, alpha);
            }
        }
        let text = tree.text(node).unwrap_or("");
        if !text.is_empty() {
            let default = text_color(style.color);
            let opacity = style.opacity.clamp(0.0, 1.0);
            buffer.draw(&mut self.fonts, &mut self.swash, default, |x, y, w, h, c| {
                let alpha = (f32::from(c.a()) * opacity).round() as u32;
                blend_rect(pixmap, clip, [ox + x as f32, oy + y as f32, w as f32, h as f32], [c.r(), c.g(), c.b()], alpha);
            });
        }
        if let Some((caret, _)) = selection {
            let (line, x) = layout.locate(caret);
            let (rgb, alpha) = ink(style.color, style.opacity);
            blend_rect(pixmap, clip, [ox + x, oy + layout.lines[line].top, CARET_WIDTH, layout.line_height], rgb, alpha);
        }
        self.keep_shaped(node, key, buffer);
    }
}
