//! Pointer, wheel and key input: hover, press and focus state, and the events a program sees.

use super::{Kind, NONE, Tree};

pub(super) const HOVER: u8 = 1;
pub(super) const PRESSED: u8 = 2;
pub(super) const FOCUS: u8 = 4;

/// What the window system reports; coordinates are window pixels, wheel deltas are pixels of content moved (positive scrolls down).
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    PointerMove { x: f32, y: f32 },
    /// The primary button went down.
    PointerDown { x: f32, y: f32 },
    PointerUp { x: f32, y: f32 },
    PointerLeave,
    Wheel { x: f32, y: f32, dx: f32, dy: f32 },
    /// A key by its code (see [`crate::keys`]); a focused input takes the editing keys.
    Key { code: u32, mods: u32, down: bool },
    /// Text the user committed, typed or composed; a focused input inserts it.
    Text(String),
    Resize { width: u32, height: u32 },
    Close,
}

/// What a program is told; `Enter`, `Leave` and `Click` name buttons and inputs, the others the node under the pointer.
#[derive(Clone, Debug, PartialEq)]
pub enum UiEvent {
    Click(u32),
    PointerDown(u32, f32, f32),
    PointerUp(u32, f32, f32),
    PointerMove(u32, f32, f32),
    Enter(u32),
    Leave(u32),
    /// A wheel turn that scrolled nothing.
    Wheel(u32, f32, f32),
    Key(u32, u32, bool),
    /// An input's text changed.
    Changed(u32),
    /// Enter was pressed in an input.
    Submit(u32),
    Resize(u32, u32),
    Close,
    /// A number another task sent with `post`.
    User(i64),
    /// A press and release on the same link run of a text node: the node and the run's link id.
    Link(u32, u32),
}

impl Tree {
    /// Applies pointer, wheel and key input to the tree and answers the events it raises; resize and close are the window's.
    pub fn input(&mut self, input: &Input) -> Vec<UiEvent> {
        let mut out = Vec::new();
        if let Input::Text(text) = input {
            if let Some(node) = self.focused_input() {
                self.type_text(node, text, &mut out);
            }
            return out;
        }
        match *input {
            Input::PointerMove { x, y } => {
                let hit = self.hit(x, y);
                self.hover_over(hit, &mut out);
                if let Some(n) = hit {
                    out.push(UiEvent::PointerMove(n, x, y));
                }
            }
            Input::PointerDown { x, y } => {
                let hit = self.hit(x, y);
                self.hover_over(hit, &mut out);
                self.press(hit, x, y, &mut out);
            }
            Input::PointerUp { x, y } => {
                let hit = self.hit(x, y);
                self.hover_over(hit, &mut out);
                self.release(hit, x, y, &mut out);
            }
            Input::PointerLeave => self.hover_over(None, &mut out),
            Input::Wheel { x, y, dx, dy } => {
                let hit = self.hit(x, y);
                self.hover_over(hit, &mut out);
                if self.scroll_at(x, y, dx, dy).is_some() {
                    let moved = self.hit(x, y);
                    self.hover_over(moved, &mut out);
                } else if let Some(n) = hit {
                    out.push(UiEvent::Wheel(n, dx, dy));
                }
            }
            Input::Key { code, mods, down } => {
                out.push(UiEvent::Key(code, mods, down));
                if let Some(node) = self.focused_input().filter(|_| down) {
                    self.edit_key(node, code, mods, &mut out);
                }
            }
            Input::Text(_) | Input::Resize { .. } | Input::Close => {}
        }
        out
    }

    fn focused_input(&self) -> Option<u32> {
        self.focus().filter(|n| self.kind(*n).is_some_and(Kind::is_input))
    }

    /// The node and its ancestors, innermost first.
    fn chain(&self, node: Option<u32>) -> Vec<u32> {
        let mut out = Vec::new();
        let mut cur = node;
        while let Some(n) = cur {
            out.push(n);
            cur = self.parent(n);
        }
        out
    }

    fn set_flag(&mut self, node: u32, flag: u8, on: bool) {
        let Some(state) = self.state.get_mut(node as usize) else { return };
        let before = *state;
        if on {
            *state |= flag;
        } else {
            *state &= !flag;
        }
        if *state != before {
            self.damage_node(node);
        }
    }

    fn takes_input(&self, node: u32) -> bool {
        matches!(self.kind(node), Some(Kind::Button | Kind::Input | Kind::TextArea))
    }

    /// Moves the hover to the nodes above `hit`, raising `Leave` and `Enter` for buttons and inputs.
    fn hover_over(&mut self, hit: Option<u32>, out: &mut Vec<UiEvent>) {
        let new = self.chain(hit);
        let old = std::mem::take(&mut self.hover);
        for &n in &old {
            if !new.contains(&n) {
                self.set_flag(n, HOVER, false);
                if self.takes_input(n) {
                    out.push(UiEvent::Leave(n));
                }
            }
        }
        for &n in new.iter().rev() {
            if !old.contains(&n) {
                self.set_flag(n, HOVER, true);
                if self.takes_input(n) {
                    out.push(UiEvent::Enter(n));
                }
            }
        }
        self.hover = new;
    }

    fn press(&mut self, hit: Option<u32>, x: f32, y: f32, out: &mut Vec<UiEvent>) {
        let Some(h) = hit else {
            self.focus_on(None);
            return;
        };
        out.push(UiEvent::PointerDown(h, x, y));
        let held = self.chain(Some(h));
        for &n in &held {
            self.set_flag(n, PRESSED, true);
        }
        let target = held.iter().copied().find(|&n| self.takes_input(n));
        self.pressed = held;
        self.focus_on(target);
    }

    fn release(&mut self, hit: Option<u32>, x: f32, y: f32, out: &mut Vec<UiEvent>) {
        let held = std::mem::take(&mut self.pressed);
        for &n in &held {
            self.set_flag(n, PRESSED, false);
        }
        if let Some(h) = hit {
            out.push(UiEvent::PointerUp(h, x, y));
        }
        let under = self.chain(hit);
        if let Some(target) = held.iter().copied().find(|&n| self.takes_input(n)).filter(|n| under.contains(n)) {
            out.push(UiEvent::Click(target));
        }
    }

    /// Gives the keyboard focus to `node`, or takes it from everyone.
    pub fn focus_on(&mut self, node: Option<u32>) {
        let next = node.unwrap_or(NONE);
        if self.focus == next {
            return;
        }
        if self.focus != NONE {
            self.set_flag(self.focus, FOCUS, false);
        }
        self.focus = next;
        if next != NONE {
            self.set_flag(next, FOCUS, true);
        }
    }

    /// The node with the keyboard focus.
    pub fn focus(&self) -> Option<u32> {
        (self.focus != NONE).then_some(self.focus)
    }

    /// Whether the pointer is over the node or something inside it.
    pub fn is_hovered(&self, node: u32) -> bool {
        self.state.get(node as usize).is_some_and(|s| s & HOVER != 0)
    }

    /// Whether the pointer went down on the node or something inside it and is still down.
    pub fn is_pressed(&self, node: u32) -> bool {
        self.state.get(node as usize).is_some_and(|s| s & PRESSED != 0)
    }
}
