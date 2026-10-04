//! A single-line text editor for `Input` nodes: caret, selection, and the editing keys. Positions count characters.

use super::{Kind, Tree, UiEvent};
use crate::keys;

/// An input's caret and the other end of its selection.
#[derive(Clone, Copy)]
pub(super) struct Edit {
    caret: usize,
    anchor: usize,
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

    /// An input's caret and selection anchor as character positions; a new input has the caret at the end of its text.
    pub fn selection(&self, node: u32) -> Option<(usize, usize)> {
        if self.kind(node) != Some(Kind::Input) {
            return None;
        }
        let len = self.text(node).map_or(0, |t| t.chars().count());
        let edit = self.edits.get(&node).copied().unwrap_or(Edit { caret: len, anchor: len });
        Some((edit.caret.min(len), edit.anchor.min(len)))
    }

    /// Puts the caret at character `index` of an input; `extend` keeps the selection's other end where it was.
    pub fn set_caret(&mut self, node: u32, index: usize, extend: bool) {
        let Some((_, anchor)) = self.selection(node) else { return };
        let len = self.text(node).map_or(0, |t| t.chars().count());
        let caret = index.min(len);
        self.edits.insert(node, Edit { caret, anchor: if extend { anchor } else { caret } });
        self.damage_node(node);
    }

    fn replace(&mut self, node: u32, from: usize, to: usize, insert: &str, out: &mut Vec<UiEvent>) {
        let chars = self.chars_of(node);
        let (from, to) = (from.min(chars.len()), to.min(chars.len()));
        let mut text: String = chars[..from].iter().collect();
        text.push_str(insert);
        text.extend(&chars[to..]);
        let at = from + insert.chars().count();
        let _ = self.set_text(node, &text);
        self.edits.insert(node, Edit { caret: at, anchor: at });
        self.damage_node(node);
        out.push(UiEvent::Changed(node));
    }

    /// Types `text` over the selection of an input; control characters are dropped.
    pub(super) fn type_text(&mut self, node: u32, text: &str, out: &mut Vec<UiEvent>) {
        let Some((caret, anchor)) = self.selection(node) else { return };
        let clean: String = text.chars().filter(|c| !c.is_control()).collect();
        if !clean.is_empty() {
            self.replace(node, caret.min(anchor), caret.max(anchor), &clean, out);
        }
    }

    /// Applies an editing key to an input: arrows, Home, End, Backspace, Delete, select all, and Enter, which submits.
    pub(super) fn edit_key(&mut self, node: u32, code: u32, mods: u32, out: &mut Vec<UiEvent>) {
        let Some((caret, anchor)) = self.selection(node) else { return };
        let chars = self.chars_of(node);
        let extend = mods & keys::SHIFT != 0;
        let by_word = mods & (keys::CTRL | keys::ALT) != 0;
        let (lo, hi) = (caret.min(anchor), caret.max(anchor));
        match code {
            keys::ENTER => out.push(UiEvent::Submit(node)),
            keys::LEFT => {
                let to = if by_word { word_left(&chars, caret) } else if !extend && lo != hi { lo } else { caret.saturating_sub(1) };
                self.set_caret(node, to, extend);
            }
            keys::RIGHT => {
                let to = if by_word { word_right(&chars, caret) } else if !extend && lo != hi { hi } else { caret + 1 };
                self.set_caret(node, to, extend);
            }
            keys::HOME => self.set_caret(node, 0, extend),
            keys::END => self.set_caret(node, chars.len(), extend),
            keys::BACKSPACE => {
                if lo != hi {
                    self.replace(node, lo, hi, "", out);
                } else if caret > 0 {
                    let from = if by_word { word_left(&chars, caret) } else { caret - 1 };
                    self.replace(node, from, caret, "", out);
                }
            }
            keys::DELETE => {
                if lo != hi {
                    self.replace(node, lo, hi, "", out);
                } else if caret < chars.len() {
                    let to = if by_word { word_right(&chars, caret) } else { caret + 1 };
                    self.replace(node, caret, to, "", out);
                }
            }
            c if mods & (keys::CTRL | keys::META) != 0 && (c == 'a' as u32 || c == 'A' as u32) => {
                self.edits.insert(node, Edit { caret: chars.len(), anchor: 0 });
                self.damage_node(node);
            }
            _ => {}
        }
    }
}
