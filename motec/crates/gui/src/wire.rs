//! A style as a flat list of numbers, the form that crosses into Mote and back.
//!
//! Slots 0 to 72 are fixed (the `*_AT` constants); the grid column tracks then the row tracks follow, each a count and that many `(kind, value)` pairs.
//! A length is `(kind, value)`: a [`Dim`] is auto 0, px 1, percent 2; a [`Len`] is px 0, percent 1; a [`Track`] is px 0, fr 1, auto 2, min-content 3, max-content 4.

use crate::style::{Align, Dim, Direction, Display, Len, NO_STYLE, Overflow, Place, Position, StyleDef, TextAlign, Track};
use crate::tree::{Input, UiEvent};

pub const DISPLAY_AT: usize = 0;
pub const POSITION_AT: usize = 1;
pub const DIRECTION_AT: usize = 2;
pub const WRAP_AT: usize = 3;
pub const ALIGN_ITEMS_AT: usize = 4;
pub const ALIGN_SELF_AT: usize = 5;
pub const ALIGN_CONTENT_AT: usize = 6;
pub const JUSTIFY_AT: usize = 7;
pub const GAP_COLUMN_AT: usize = 8;
pub const GAP_ROW_AT: usize = 10;
pub const GROW_AT: usize = 12;
pub const SHRINK_AT: usize = 13;
pub const BASIS_AT: usize = 14;
pub const WIDTH_AT: usize = 16;
pub const HEIGHT_AT: usize = 18;
pub const MIN_WIDTH_AT: usize = 20;
pub const MIN_HEIGHT_AT: usize = 22;
pub const MAX_WIDTH_AT: usize = 24;
pub const MAX_HEIGHT_AT: usize = 26;
pub const MARGIN_AT: usize = 28;
pub const PADDING_AT: usize = 36;
pub const BORDER_AT: usize = 44;
pub const INSET_AT: usize = 48;
pub const OVERFLOW_AT: usize = 56;
pub const GRID_COLUMN_AT: usize = 58;
pub const GRID_ROW_AT: usize = 60;
pub const BACKGROUND_AT: usize = 62;
pub const BORDER_COLOR_AT: usize = 63;
pub const RADIUS_AT: usize = 64;
pub const OPACITY_AT: usize = 65;
pub const COLOR_AT: usize = 66;
pub const FONT_SIZE_AT: usize = 67;
pub const FONT_WEIGHT_AT: usize = 68;
pub const LINE_HEIGHT_AT: usize = 69;
pub const TEXT_ALIGN_AT: usize = 70;
pub const HOVER_AT: usize = 71;
pub const PRESSED_AT: usize = 72;
/// The number of fixed slots.
pub const FIXED_LEN: usize = 73;

struct Reader<'a> {
    wire: &'a [f64],
    at: usize,
}

impl Reader<'_> {
    fn next(&mut self) -> Result<f64, String> {
        let v = *self.wire.get(self.at).ok_or("the style list is too short")?;
        self.at += 1;
        Ok(v)
    }

    fn code(&mut self, what: &str, max: i64) -> Result<i64, String> {
        let v = self.next()?;
        if v.fract() != 0.0 || v < 0.0 || v > max as f64 {
            return Err(format!("bad {what} {v}"));
        }
        Ok(v as i64)
    }

    fn float(&mut self, what: &str) -> Result<f32, String> {
        let v = self.next()?;
        if !v.is_finite() {
            return Err(format!("bad {what} {v}"));
        }
        Ok(v as f32)
    }

    fn dim(&mut self, what: &str) -> Result<Dim, String> {
        let kind = self.code(what, 2)?;
        let v = self.float(what)?;
        Ok(match kind {
            0 => Dim::Auto,
            1 => Dim::Px(v),
            _ => Dim::Pct(v),
        })
    }

    fn len(&mut self, what: &str) -> Result<Len, String> {
        let kind = self.code(what, 1)?;
        let v = self.float(what)?;
        Ok(if kind == 0 { Len::Px(v) } else { Len::Pct(v) })
    }

    fn align(&mut self, what: &str) -> Result<Option<Align>, String> {
        let v = self.next()?;
        if v == -1.0 {
            return Ok(None);
        }
        if v.fract() != 0.0 || !(0.0..=6.0).contains(&v) {
            return Err(format!("bad {what} {v}"));
        }
        Ok(Some(ALIGNS[v as usize]))
    }

    fn style_id(&mut self, what: &str) -> Result<u32, String> {
        let v = self.next()?;
        if v == -1.0 {
            return Ok(NO_STYLE);
        }
        if v.fract() != 0.0 || !(0.0..4_294_967_295.0).contains(&v) {
            return Err(format!("bad {what} {v}"));
        }
        Ok(v as u32)
    }

    fn tracks(&mut self, what: &str) -> Result<Vec<Track>, String> {
        let n = self.code(what, 4096)?;
        let mut out = Vec::new();
        for _ in 0..n {
            let kind = self.code(what, 4)?;
            let v = self.float(what)?;
            out.push(match kind {
                0 => Track::Px(v),
                1 => Track::Fr(v),
                2 => Track::Auto,
                3 => Track::MinContent,
                _ => Track::MaxContent,
            });
        }
        Ok(out)
    }
}

const ALIGNS: [Align; 7] = [Align::Start, Align::End, Align::Center, Align::Stretch, Align::SpaceBetween, Align::SpaceAround, Align::SpaceEvenly];

fn align_code(a: Option<Align>) -> f64 {
    a.map_or(-1.0, |a| ALIGNS.iter().position(|x| *x == a).unwrap_or(0) as f64)
}

fn push_dim(out: &mut Vec<f64>, d: Dim) {
    out.extend(match d {
        Dim::Auto => [0.0, 0.0],
        Dim::Px(v) => [1.0, v as f64],
        Dim::Pct(v) => [2.0, v as f64],
    });
}

fn push_len(out: &mut Vec<f64>, l: Len) {
    out.extend(match l {
        Len::Px(v) => [0.0, v as f64],
        Len::Pct(v) => [1.0, v as f64],
    });
}

fn push_tracks(out: &mut Vec<f64>, tracks: &[Track]) {
    out.push(tracks.len() as f64);
    for t in tracks {
        out.extend(match *t {
            Track::Px(v) => [0.0, v as f64],
            Track::Fr(v) => [1.0, v as f64],
            Track::Auto => [2.0, 0.0],
            Track::MinContent => [3.0, 0.0],
            Track::MaxContent => [4.0, 0.0],
        });
    }
}

fn style_id_code(id: u32) -> f64 {
    if id == NO_STYLE { -1.0 } else { id as f64 }
}

impl UiEvent {
    /// The event as `[code, node, a, b, c]`: codes 0 Click, 1 PointerDown, 2 PointerUp, 3 PointerMove, 4 Enter, 5 Leave, 6 Wheel, 7 Key, 8 Changed, 9 Submit, 10 Resize, 11 Close, 12 User.
    ///
    /// Pointer events carry `x, y` in `a, b`; Wheel `dx, dy`; Key `code, mods, down`; Resize `width, height`; User the number in `a`. Unused slots are 0.
    pub fn to_wire(&self) -> [f64; 5] {
        let n = |v: u32| f64::from(v);
        match *self {
            UiEvent::Click(node) => [0.0, n(node), 0.0, 0.0, 0.0],
            UiEvent::PointerDown(node, x, y) => [1.0, n(node), f64::from(x), f64::from(y), 0.0],
            UiEvent::PointerUp(node, x, y) => [2.0, n(node), f64::from(x), f64::from(y), 0.0],
            UiEvent::PointerMove(node, x, y) => [3.0, n(node), f64::from(x), f64::from(y), 0.0],
            UiEvent::Enter(node) => [4.0, n(node), 0.0, 0.0, 0.0],
            UiEvent::Leave(node) => [5.0, n(node), 0.0, 0.0, 0.0],
            UiEvent::Wheel(node, dx, dy) => [6.0, n(node), f64::from(dx), f64::from(dy), 0.0],
            UiEvent::Key(code, mods, down) => [7.0, 0.0, n(code), n(mods), if down { 1.0 } else { 0.0 }],
            UiEvent::Changed(node) => [8.0, n(node), 0.0, 0.0, 0.0],
            UiEvent::Submit(node) => [9.0, n(node), 0.0, 0.0, 0.0],
            UiEvent::Resize(w, h) => [10.0, 0.0, n(w), n(h), 0.0],
            UiEvent::Close => [11.0, 0.0, 0.0, 0.0, 0.0],
            UiEvent::User(v) => [12.0, 0.0, v as f64, 0.0, 0.0],
        }
    }
}

impl Input {
    /// An input from the numbers `__ui_input` passes: `kind` 0 PointerMove, 1 PointerDown, 2 PointerUp, 3 PointerLeave, 4 Wheel, 5 Key, 6 Resize, 7 Close.
    pub fn from_wire(kind: i64, a: f64, b: f64, c: f64, d: f64) -> Result<Input, String> {
        let (x, y) = (a as f32, b as f32);
        let whole = |v: f64, what: &str| -> Result<u32, String> {
            if v.fract() != 0.0 || !(0.0..=4_294_967_295.0).contains(&v) {
                return Err(format!("bad {what} {v}"));
            }
            Ok(v as u32)
        };
        Ok(match kind {
            0 => Input::PointerMove { x, y },
            1 => Input::PointerDown { x, y },
            2 => Input::PointerUp { x, y },
            3 => Input::PointerLeave,
            4 => Input::Wheel { x, y, dx: c as f32, dy: d as f32 },
            5 => Input::Key { code: whole(a, "key")?, mods: whole(b, "modifiers")?, down: c != 0.0 },
            6 => Input::Resize { width: whole(a, "width")?, height: whole(b, "height")? },
            7 => Input::Close,
            _ => return Err(format!("no input kind {kind}")),
        })
    }
}

impl StyleDef {
    /// The style as a flat list of numbers.
    pub fn to_wire(&self) -> Vec<f64> {
        let mut o: Vec<f64> = Vec::with_capacity(FIXED_LEN + 8);
        o.push(match self.display {
            Display::Flex => 0.0,
            Display::Grid => 1.0,
            Display::Block => 2.0,
            Display::None => 3.0,
        });
        o.push(if self.position == Position::Absolute { 1.0 } else { 0.0 });
        o.push(match self.direction {
            Direction::Row => 0.0,
            Direction::Column => 1.0,
            Direction::RowReverse => 2.0,
            Direction::ColumnReverse => 3.0,
        });
        o.push(if self.wrap { 1.0 } else { 0.0 });
        for a in [self.align_items, self.align_self, self.align_content, self.justify_content] {
            o.push(align_code(a));
        }
        push_len(&mut o, self.gap[0]);
        push_len(&mut o, self.gap[1]);
        o.extend([self.grow as f64, self.shrink as f64]);
        push_dim(&mut o, self.basis);
        for d in self.size.iter().chain(&self.min_size).chain(&self.max_size) {
            push_dim(&mut o, *d);
        }
        for d in self.margin {
            push_dim(&mut o, d);
        }
        for l in self.padding {
            push_len(&mut o, l);
        }
        o.extend(self.border.iter().map(|b| *b as f64));
        for d in self.inset {
            push_dim(&mut o, d);
        }
        for ov in self.overflow {
            o.push(match ov {
                Overflow::Visible => 0.0,
                Overflow::Hidden => 1.0,
                Overflow::Scroll => 2.0,
            });
        }
        o.extend([self.grid_column.start as f64, self.grid_column.span as f64, self.grid_row.start as f64, self.grid_row.span as f64]);
        o.extend([self.background as f64, self.border_color as f64, self.radius as f64, self.opacity as f64, self.color as f64]);
        o.extend([self.font_size as f64, self.font_weight as f64, self.line_height as f64]);
        o.push(match self.text_align {
            TextAlign::Left => 0.0,
            TextAlign::Center => 1.0,
            TextAlign::Right => 2.0,
        });
        o.extend([style_id_code(self.hover), style_id_code(self.pressed)]);
        debug_assert_eq!(o.len(), FIXED_LEN);
        push_tracks(&mut o, &self.columns);
        push_tracks(&mut o, &self.rows);
        o
    }

    /// A style from a flat list of numbers; a message names the first bad value.
    pub fn from_wire(wire: &[f64]) -> Result<StyleDef, String> {
        let mut r = Reader { wire, at: 0 };
        let display = match r.code("display", 3)? {
            0 => Display::Flex,
            1 => Display::Grid,
            2 => Display::Block,
            _ => Display::None,
        };
        let position = if r.code("position", 1)? == 1 { Position::Absolute } else { Position::Relative };
        let direction = match r.code("direction", 3)? {
            0 => Direction::Row,
            1 => Direction::Column,
            2 => Direction::RowReverse,
            _ => Direction::ColumnReverse,
        };
        let wrap = r.code("wrap", 1)? == 1;
        let (align_items, align_self, align_content, justify_content) = (r.align("align")?, r.align("align")?, r.align("align")?, r.align("justify")?);
        let gap = [r.len("gap")?, r.len("gap")?];
        let (grow, shrink) = (r.float("grow")?, r.float("shrink")?);
        let basis = r.dim("basis")?;
        let size = [r.dim("width")?, r.dim("height")?];
        let min_size = [r.dim("min width")?, r.dim("min height")?];
        let max_size = [r.dim("max width")?, r.dim("max height")?];
        let margin = [r.dim("margin")?, r.dim("margin")?, r.dim("margin")?, r.dim("margin")?];
        let padding = [r.len("padding")?, r.len("padding")?, r.len("padding")?, r.len("padding")?];
        let border = [r.float("border")?, r.float("border")?, r.float("border")?, r.float("border")?];
        let inset = [r.dim("inset")?, r.dim("inset")?, r.dim("inset")?, r.dim("inset")?];
        let mut overflow = [Overflow::Visible; 2];
        for o in &mut overflow {
            *o = match r.code("overflow", 2)? {
                0 => Overflow::Visible,
                1 => Overflow::Hidden,
                _ => Overflow::Scroll,
            };
        }
        let mut place = |what: &str| -> Result<Place, String> {
            let start = r.float(what)?;
            let span = r.float(what)?;
            if start.fract() != 0.0 || span.fract() != 0.0 || !(-32768.0..=32767.0).contains(&start) || !(0.0..=65535.0).contains(&span) {
                return Err(format!("bad {what}"));
            }
            Ok(Place { start: start as i16, span: span as u16 })
        };
        let (grid_column, grid_row) = (place("grid column")?, place("grid row")?);
        let color = |r: &mut Reader<'_>, what: &str| -> Result<u32, String> {
            let v = r.next()?;
            if v.fract() != 0.0 || !(0.0..=4_294_967_295.0).contains(&v) {
                return Err(format!("bad {what} {v}"));
            }
            Ok(v as u32)
        };
        let background = color(&mut r, "background")?;
        let border_color = color(&mut r, "border color")?;
        let radius = r.float("radius")?;
        let opacity = r.float("opacity")?;
        let text_color = color(&mut r, "color")?;
        let font_size = r.float("font size")?;
        let font_weight = r.code("font weight", 1000)? as u16;
        let line_height = r.float("line height")?;
        let text_align = match r.code("text align", 2)? {
            0 => TextAlign::Left,
            1 => TextAlign::Center,
            _ => TextAlign::Right,
        };
        let hover = r.style_id("hover style")?;
        let pressed = r.style_id("pressed style")?;
        let columns = r.tracks("grid columns")?;
        let rows = r.tracks("grid rows")?;
        Ok(StyleDef {
            display,
            position,
            direction,
            wrap,
            align_items,
            align_self,
            align_content,
            justify_content,
            gap,
            grow,
            shrink,
            basis,
            size,
            min_size,
            max_size,
            margin,
            padding,
            border,
            inset,
            overflow,
            columns,
            rows,
            grid_column,
            grid_row,
            background,
            border_color,
            radius,
            opacity,
            color: text_color,
            font_size,
            font_weight,
            line_height,
            text_align,
            hover,
            pressed,
        })
    }
}
