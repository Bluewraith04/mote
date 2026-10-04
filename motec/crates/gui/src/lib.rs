//! A CPU-rendered UI tree: flat node arrays, interned styles, taffy layout and hit testing.

mod geom;
pub mod keys;
mod paint;
mod style;
mod surface;
mod tree;
pub mod wire;

pub use geom::Rect;
pub use paint::{InputGeometry, Painter};
pub use surface::{MAX_SIDE, Surface};
pub use style::{Align, Dim, Direction, Display, Len, NO_STYLE, Overflow, Place, Position, StyleDef, TextAlign, Track};
pub use tree::{Input, Kind, Measure, MeasureNode, NONE, Tree, UiEvent, UiNode};
