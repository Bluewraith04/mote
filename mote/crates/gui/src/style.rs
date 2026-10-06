//! Node styles: layout fields (mapped to taffy) and paint fields, interned by the tree.

use taffy::style_helpers as th;

use crate::geom::Rect;

/// A size that may be automatic or relative; percentages are 0 to 100.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Dim {
    Auto,
    Px(f32),
    Pct(f32),
}

/// A length that is never automatic.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Len {
    Px(f32),
    Pct(f32),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Display {
    Flex,
    Grid,
    Block,
    None,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Position {
    Relative,
    Absolute,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Direction {
    Row,
    Column,
    RowReverse,
    ColumnReverse,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Align {
    Start,
    End,
    Center,
    Stretch,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Overflow {
    Visible,
    Hidden,
    Scroll,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Track {
    Px(f32),
    Fr(f32),
    Auto,
    MinContent,
    MaxContent,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

/// A grid placement along one axis: a 1-based start line (0 is automatic) and a span.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Place {
    pub start: i16,
    pub span: u16,
}

/// One interned style. Sides are in the order top, right, bottom, left; sizes and gaps are width then height.
#[derive(Clone, PartialEq, Debug)]
pub struct StyleDef {
    pub display: Display,
    pub position: Position,
    pub direction: Direction,
    pub wrap: bool,
    pub align_items: Option<Align>,
    pub align_self: Option<Align>,
    pub align_content: Option<Align>,
    pub justify_content: Option<Align>,
    pub gap: [Len; 2],
    pub grow: f32,
    pub shrink: f32,
    pub basis: Dim,
    pub size: [Dim; 2],
    pub min_size: [Dim; 2],
    pub max_size: [Dim; 2],
    pub margin: [Dim; 4],
    pub padding: [Len; 4],
    pub border: [f32; 4],
    pub inset: [Dim; 4],
    pub overflow: [Overflow; 2],
    pub columns: Vec<Track>,
    pub rows: Vec<Track>,
    pub grid_column: Place,
    pub grid_row: Place,
    /// Packed `0xRRGGBBAA`; alpha 0 paints nothing.
    pub background: u32,
    pub border_color: u32,
    pub radius: f32,
    pub opacity: f32,
    pub color: u32,
    pub font_size: f32,
    pub font_weight: u16,
    /// Pixels; 0 means the font's own.
    pub line_height: f32,
    pub text_align: TextAlign,
    /// Style ids whose paint fields replace these while the pointer is over, or presses, the node; [`NO_STYLE`] for none.
    pub hover: u32,
    pub pressed: u32,
}

/// The id meaning "no style" in a node's `style_id` or a style's variant.
pub const NO_STYLE: u32 = u32::MAX;

impl Default for StyleDef {
    fn default() -> Self {
        StyleDef {
            display: Display::Flex,
            position: Position::Relative,
            direction: Direction::Row,
            wrap: false,
            align_items: None,
            align_self: None,
            align_content: None,
            justify_content: None,
            gap: [Len::Px(0.0); 2],
            grow: 0.0,
            shrink: 1.0,
            basis: Dim::Auto,
            size: [Dim::Auto; 2],
            min_size: [Dim::Auto; 2],
            max_size: [Dim::Auto; 2],
            margin: [Dim::Px(0.0); 4],
            padding: [Len::Px(0.0); 4],
            border: [0.0; 4],
            inset: [Dim::Auto; 4],
            overflow: [Overflow::Visible; 2],
            columns: Vec::new(),
            rows: Vec::new(),
            grid_column: Place { start: 0, span: 1 },
            grid_row: Place { start: 0, span: 1 },
            background: 0,
            border_color: 0,
            radius: 0.0,
            opacity: 1.0,
            color: 0x000000ff,
            font_size: 16.0,
            font_weight: 400,
            line_height: 0.0,
            text_align: TextAlign::Left,
            hover: NO_STYLE,
            pressed: NO_STYLE,
        }
    }
}

impl StyleDef {
    /// The inner left, top, right and bottom insets (border plus padding) of a box of the given width.
    pub fn insets(&self, width: f32) -> [f32; 4] {
        let pad = |l: Len| match l {
            Len::Px(v) => v,
            Len::Pct(p) => width * p / 100.0,
        };
        [self.border[3] + pad(self.padding[3]), self.border[0] + pad(self.padding[0]), self.border[1] + pad(self.padding[1]), self.border[2] + pad(self.padding[2])]
    }

    pub(crate) fn clips(&self) -> bool {
        self.overflow.iter().any(|o| *o != Overflow::Visible)
    }
}

fn dim(d: Dim) -> taffy::Dimension {
    match d {
        Dim::Auto => taffy::Dimension::auto(),
        Dim::Px(v) => taffy::Dimension::length(v),
        Dim::Pct(p) => taffy::Dimension::percent(p / 100.0),
    }
}

fn dim_auto(d: Dim) -> taffy::LengthPercentageAuto {
    match d {
        Dim::Auto => taffy::LengthPercentageAuto::auto(),
        Dim::Px(v) => taffy::LengthPercentageAuto::length(v),
        Dim::Pct(p) => taffy::LengthPercentageAuto::percent(p / 100.0),
    }
}

fn len(l: Len) -> taffy::LengthPercentage {
    match l {
        Len::Px(v) => taffy::LengthPercentage::length(v),
        Len::Pct(p) => taffy::LengthPercentage::percent(p / 100.0),
    }
}

fn items(a: Align) -> taffy::AlignItems {
    match a {
        Align::Start => taffy::AlignItems::FLEX_START,
        Align::End => taffy::AlignItems::FLEX_END,
        Align::Center => taffy::AlignItems::CENTER,
        Align::Stretch | Align::SpaceBetween | Align::SpaceAround | Align::SpaceEvenly => taffy::AlignItems::STRETCH,
    }
}

fn content(a: Align) -> taffy::AlignContent {
    match a {
        Align::Start => taffy::AlignContent::FLEX_START,
        Align::End => taffy::AlignContent::FLEX_END,
        Align::Center => taffy::AlignContent::CENTER,
        Align::Stretch => taffy::AlignContent::STRETCH,
        Align::SpaceBetween => taffy::AlignContent::SPACE_BETWEEN,
        Align::SpaceAround => taffy::AlignContent::SPACE_AROUND,
        Align::SpaceEvenly => taffy::AlignContent::SPACE_EVENLY,
    }
}

fn overflow(o: Overflow) -> taffy::Overflow {
    match o {
        Overflow::Visible => taffy::Overflow::Visible,
        Overflow::Hidden => taffy::Overflow::Hidden,
        Overflow::Scroll => taffy::Overflow::Hidden,
    }
}

fn track<S: taffy::CheapCloneStr>(t: Track) -> taffy::GridTemplateComponent<S> {
    taffy::GridTemplateComponent::Single(match t {
        Track::Px(v) => th::length(v),
        Track::Fr(v) => th::fr(v),
        Track::Auto => th::auto(),
        Track::MinContent => th::min_content(),
        Track::MaxContent => th::max_content(),
    })
}

fn placement<S: taffy::CheapCloneStr>(p: Place) -> taffy::Line<taffy::GridPlacement<S>> {
    let start = if p.start == 0 { taffy::GridPlacement::Auto } else { <taffy::GridPlacement<S> as th::TaffyGridLine>::from_line_index(p.start) };
    taffy::Line { start, end: taffy::GridPlacement::Span(p.span.max(1)) }
}

/// The taffy style for `s`; a scroll or hidden node clips, and `forced` sets a definite size (the root's).
pub(crate) fn to_taffy(s: &StyleDef, forced: Option<Rect>) -> taffy::Style {
    taffy::Style {
        display: match s.display {
            Display::Flex => taffy::Display::Flex,
            Display::Grid => taffy::Display::Grid,
            Display::Block => taffy::Display::Block,
            Display::None => taffy::Display::None,
        },
        position: match s.position {
            Position::Relative => taffy::Position::Relative,
            Position::Absolute => taffy::Position::Absolute,
        },
        flex_direction: match s.direction {
            Direction::Row => taffy::FlexDirection::Row,
            Direction::Column => taffy::FlexDirection::Column,
            Direction::RowReverse => taffy::FlexDirection::RowReverse,
            Direction::ColumnReverse => taffy::FlexDirection::ColumnReverse,
        },
        flex_wrap: if s.wrap { taffy::FlexWrap::Wrap } else { taffy::FlexWrap::NoWrap },
        align_items: s.align_items.map(items),
        align_self: s.align_self.map(items),
        align_content: s.align_content.map(content),
        justify_content: s.justify_content.map(content),
        gap: taffy::Size { width: len(s.gap[0]), height: len(s.gap[1]) },
        flex_grow: s.grow,
        flex_shrink: s.shrink,
        flex_basis: dim(s.basis),
        size: match forced {
            Some(r) => taffy::Size { width: taffy::Dimension::length(r.w), height: taffy::Dimension::length(r.h) },
            None => taffy::Size { width: dim(s.size[0]), height: dim(s.size[1]) },
        },
        min_size: taffy::Size { width: dim_auto(s.min_size[0]), height: dim_auto(s.min_size[1]) },
        max_size: taffy::Size { width: dim_auto(s.max_size[0]), height: dim_auto(s.max_size[1]) },
        margin: taffy::Rect { top: dim_auto(s.margin[0]), right: dim_auto(s.margin[1]), bottom: dim_auto(s.margin[2]), left: dim_auto(s.margin[3]) },
        padding: taffy::Rect { top: len(s.padding[0]), right: len(s.padding[1]), bottom: len(s.padding[2]), left: len(s.padding[3]) },
        border: taffy::Rect {
            top: taffy::LengthPercentage::length(s.border[0]),
            right: taffy::LengthPercentage::length(s.border[1]),
            bottom: taffy::LengthPercentage::length(s.border[2]),
            left: taffy::LengthPercentage::length(s.border[3]),
        },
        inset: taffy::Rect { top: dim_auto(s.inset[0]), right: dim_auto(s.inset[1]), bottom: dim_auto(s.inset[2]), left: dim_auto(s.inset[3]) },
        overflow: taffy::Point { x: overflow(s.overflow[0]), y: overflow(s.overflow[1]) },
        scrollbar_width: 0.0,
        grid_template_columns: s.columns.iter().copied().map(track).collect(),
        grid_template_rows: s.rows.iter().copied().map(track).collect(),
        grid_column: placement(s.grid_column),
        grid_row: placement(s.grid_row),
        ..taffy::Style::default()
    }
}
