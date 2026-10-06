//! The text editor of `Input` and `TextArea` nodes: caret, selection, undo and the editing keys. Positions count characters.

use super::{Kind, Tree, UiEvent};
use crate::keys;

/// The most undo steps an input keeps.
const HISTORY_LIMIT: usize = 200;

/// An input's caret, the other end of its selection, and a text area's scroll and remembered column.
#[derive(Clone, Copy)]
pub(super) struct Edit {
    caret: usize,
    anchor: usize,
    /// How far a text area's text is scrolled up, in pixels.
    top: f32,
    /// The x the caret keeps while it moves up and down a text area.
    want_x: Option<f32>,
}

/// What the last edit was, so that typing one letter after another is one undo step.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Last {
    #[default]
    Other,
    Typing,
    Deleting,
}

/// The text and selection an undo returns to.
struct Snapshot {
    text: String,
    caret: usize,
    anchor: usize,
}

/// The undo and redo steps of one input.
#[derive(Default)]
pub(super) struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last: Last,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn word_left(chars: &[char], mut i: usize) -> usize {
    while i > 0 && !is_word(chars[i - 1]) {
        i -= 1;
    }
    while i > 0 && is_word(chars[i - 1]) {
        i -= 1;
    }
    i
}

fn word_right(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && !is_word(chars[i]) {
        i += 1;
    }
    while i < chars.len() && is_word(chars[i]) {
        i += 1;
    }
    i
}

impl Tree {
    fn chars_of(&self, node: u32) -> Vec<char> {
        self.text(node).map(|t| t.chars().collect()).unwrap_or_default()
    }

    fn edit_of(&self, node: u32) -> Option<Edit> {
        if !self.kind(node).is_some_and(Kind::is_input) {
            return None;
        }
        let len = self.text(node).map_or(0, |t| t.chars().count());
        let edit = self.edits.get(&node).copied().unwrap_or(Edit { caret: len, anchor: len, top: 0.0, want_x: None });
        Some(Edit { caret: edit.caret.min(len), anchor: edit.anchor.min(len), ..edit })
    }

    /// An input's caret and selection anchor as character positions; a new input has the caret at the end of its text.
    pub fn selection(&self, node: u32) -> Option<(usize, usize)> {
        self.edit_of(node).map(|e| (e.caret, e.anchor))
    }

    /// Puts the caret at character `index` of an input; `extend` keeps the selection's other end where it was.
    pub fn set_caret(&mut self, node: u32, index: usize, extend: bool) {
        self.move_caret(node, index, extend, None);
    }

    /// Like `set_caret`, and remembers the column `want_x` for the next move up or down a text area.
    pub fn move_caret(&mut self, node: u32, index: usize, extend: bool, want_x: Option<f32>) {
        let Some(edit) = self.edit_of(node) else { return };
        let len = self.text(node).map_or(0, |t| t.chars().count());
        let caret = index.min(len);
        self.edits.insert(node, Edit { caret, anchor: if extend { edit.anchor } else { caret }, want_x, ..edit });
        if let Some(h) = self.history.get_mut(&node) {
            h.last = Last::Other;
        }
        self.damage_node(node);
    }

    /// How far a text area's text is scrolled up, in pixels.
    pub fn scroll_top(&self, node: u32) -> f32 {
        self.edit_of(node).map_or(0.0, |e| e.top)
    }

    /// Scrolls a text area's text so that `top` pixels of it are above the box.
    pub fn set_scroll_top(&mut self, node: u32, top: f32) {
        let Some(edit) = self.edit_of(node) else { return };
        if edit.top != top {
            self.edits.insert(node, Edit { top, ..edit });
            self.damage_node(node);
        }
    }

    /// The column a text area's caret keeps while it moves up and down.
    pub fn want_x(&self, node: u32) -> Option<f32> {
        self.edit_of(node).and_then(|e| e.want_x)
    }

    fn snapshot(&self, node: u32) -> Snapshot {
        let (caret, anchor) = self.selection(node).unwrap_or((0, 0));
        Snapshot { text: self.text(node).unwrap_or("").to_string(), caret, anchor }
    }

    /// Drops an input's undo steps, for a text a program set.
    pub(super) fn forget_history(&mut self, node: u32) {
        self.history.remove(&node);
    }

    /// Keeps the text as an undo step, unless `how` continues the run of edits before it.
    fn record(&mut self, node: u32, how: Last) {
        let continues = how != Last::Other && self.history.get(&node).is_some_and(|h| h.last == how);
        if !continues {
            let snap = self.snapshot(node);
            let h = self.history.entry(node).or_default();
            h.undo.push(snap);
            if h.undo.len() > HISTORY_LIMIT {
                h.undo.remove(0);
            }
        }
        let h = self.history.entry(node).or_default();
        h.redo.clear();
        h.last = how;
    }

    fn restore(&mut self, node: u32, snap: Snapshot, out: &mut Vec<UiEvent>) {
        let top = self.scroll_top(node);
        let _ = self.put_text(node, &snap.text);
        self.edits.insert(node, Edit { caret: snap.caret, anchor: snap.anchor, top, want_x: None });
        self.damage_node(node);
        out.push(UiEvent::Changed(node));
    }

    /// Goes back one edit of an input, or forward one when `redo`.
    fn step_history(&mut self, node: u32, redo: bool, out: &mut Vec<UiEvent>) {
        let now = self.snapshot(node);
        let Some(h) = self.history.get_mut(&node) else { return };
        let Some(snap) = (if redo { h.redo.pop() } else { h.undo.pop() }) else { return };
        if redo { h.undo.push(now) } else { h.redo.push(now) }
        h.last = Last::Other;
        self.restore(node, snap, out);
    }

    fn replace(&mut self, node: u32, from: usize, to: usize, insert: &str, how: Last, out: &mut Vec<UiEvent>) {
        self.record(node, how);
        let chars = self.chars_of(node);
        let (from, to) = (from.min(chars.len()), to.min(chars.len()));
        let mut text: String = chars[..from].iter().collect();
        text.push_str(insert);
        text.extend(&chars[to..]);
        let at = from + insert.chars().count();
        let top = self.scroll_top(node);
        let _ = self.put_text(node, &text);
        self.edits.insert(node, Edit { caret: at, anchor: at, top, want_x: None });
        self.damage_node(node);
        out.push(UiEvent::Changed(node));
    }

    /// Types `text` over the selection of an input; a line input drops control characters, a text area keeps tabs and line breaks.
    pub(super) fn type_text(&mut self, node: u32, text: &str, out: &mut Vec<UiEvent>) {
        let Some((caret, anchor)) = self.selection(node) else { return };
        let area = self.kind(node) == Some(Kind::TextArea);
        if area && matches!(text, "\r" | "\n" | "\r\n") {
            return;
        }
        let clean: String = text.chars().filter(|c| !c.is_control() || (area && matches!(c, '\n' | '\t'))).collect();
        if !clean.is_empty() {
            let (lo, hi) = (caret.min(anchor), caret.max(anchor));
            let how = if lo == hi && !clean.contains('\n') { Last::Typing } else { Last::Other };
            self.replace(node, lo, hi, &clean, how, out);
        }
    }

    /// Applies an editing key to an input: arrows, Home, End, Backspace, Delete, select all, undo, and Enter, which submits a line input and breaks the line of a text area.
    pub(super) fn edit_key(&mut self, node: u32, code: u32, mods: u32, out: &mut Vec<UiEvent>) {
        let Some((caret, anchor)) = self.selection(node) else { return };
        let area = self.kind(node) == Some(Kind::TextArea);
        let chars = self.chars_of(node);
        let extend = mods & keys::SHIFT != 0;
        let by_word = mods & (keys::CTRL | keys::ALT) != 0;
        let command = mods & (keys::CTRL | keys::META) != 0;
        let (lo, hi) = (caret.min(anchor), caret.max(anchor));
        match code {
            keys::ENTER => {
                if area && !command {
                    self.replace(node, lo, hi, "\n", Last::Other, out);
                } else {
                    out.push(UiEvent::Submit(node));
                }
            }
            keys::LEFT => {
                let to = if by_word { word_left(&chars, caret) } else if !extend && lo != hi { lo } else { caret.saturating_sub(1) };
                self.set_caret(node, to, extend);
            }
            keys::RIGHT => {
                let to = if by_word { word_right(&chars, caret) } else if !extend && lo != hi { hi } else { caret + 1 };
                self.set_caret(node, to, extend);
            }
            keys::HOME if !area || command => self.set_caret(node, 0, extend),
            keys::END if !area || command => self.set_caret(node, chars.len(), extend),
            keys::BACKSPACE => {
                if lo != hi {
                    self.replace(node, lo, hi, "", Last::Other, out);
                } else if caret > 0 {
                    let from = if by_word { word_left(&chars, caret) } else { caret - 1 };
                    self.replace(node, from, caret, "", if by_word { Last::Other } else { Last::Deleting }, out);
                }
            }
            keys::DELETE => {
                if lo != hi {
                    self.replace(node, lo, hi, "", Last::Other, out);
                } else if caret < chars.len() {
                    let to = if by_word { word_right(&chars, caret) } else { caret + 1 };
                    self.replace(node, caret, to, "", if by_word { Last::Other } else { Last::Deleting }, out);
                }
            }
            c if command && (c == 'a' as u32 || c == 'A' as u32) => {
                let top = self.scroll_top(node);
                self.edits.insert(node, Edit { caret: chars.len(), anchor: 0, top, want_x: None });
                self.damage_node(node);
            }
            c if command && (c == 'z' as u32 || c == 'Z' as u32) => self.step_history(node, extend, out),
            c if command && (c == 'y' as u32 || c == 'Y' as u32) => self.step_history(node, true, out),
            _ => {}
        }
    }
}
