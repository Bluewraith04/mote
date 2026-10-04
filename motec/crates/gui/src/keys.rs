//! Key codes and modifier bits: a printable key is its Unicode code point, a named key is one of these.

pub const BACKSPACE: u32 = 8;
pub const TAB: u32 = 9;
pub const ENTER: u32 = 13;
pub const ESCAPE: u32 = 27;
pub const DELETE: u32 = 127;
pub const UP: u32 = 0xF700;
pub const DOWN: u32 = 0xF701;
pub const LEFT: u32 = 0xF702;
pub const RIGHT: u32 = 0xF703;
pub const HOME: u32 = 0xF729;
pub const END: u32 = 0xF72B;

pub const SHIFT: u32 = 1;
pub const CTRL: u32 = 2;
pub const ALT: u32 = 4;
pub const META: u32 = 8;
