//! Text input: typing, the editing keys, selection, the mouse, and the caret and selection paint.

use gui::keys::{BACKSPACE, CTRL, DELETE, END, ENTER, HOME, LEFT, RIGHT, SHIFT};
use gui::{Dim, Input, Kind, Len, Painter, StyleDef, Surface, UiEvent};

/// A 200 by 30 input at (10, 10) in a 240 by 60 window, with no text yet.
fn scene() -> (Surface, u32) {
    let mut s = Surface::new(240, 60, Painter::bundled_only()).unwrap();
    let id = s.tree.intern_style(StyleDef {
        position: gui::Position::Absolute,
        inset: [Dim::Px(10.0), Dim::Auto, Dim::Auto, Dim::Px(10.0)],
        size: [Dim::Px(200.0), Dim::Px(30.0)],
        padding: [Len::Px(4.0); 4],
        ..StyleDef::default()
    });
    let input = s.tree.add(0, Kind::Input).unwrap();
    s.tree.set_style(input, id).unwrap();
    s.present();
    (s, input)
}

fn focused() -> (Surface, u32) {
    let (mut s, input) = scene();
    s.input(&Input::PointerDown { x: 20.0, y: 20.0 });
    s.input(&Input::PointerUp { x: 20.0, y: 20.0 });
    assert_eq!(s.tree.focus(), Some(input));
    (s, input)
}

fn text(s: &mut Surface, t: &str) -> Vec<UiEvent> {
    s.input(&Input::Text(t.to_string()))
}

fn key(s: &mut Surface, code: u32, mods: u32) -> Vec<UiEvent> {
    s.input(&Input::Key { code, mods, down: true })
}

fn only_changes(events: &[UiEvent]) -> Vec<&UiEvent> {
    events.iter().filter(|e| !matches!(e, UiEvent::Key(..))).collect()
}

#[test]
fn typed_text_goes_into_the_focused_input() {
    let (mut s, input) = focused();
    assert_eq!(text(&mut s, "abc"), [UiEvent::Changed(input)]);
    assert_eq!(s.tree.text(input).unwrap(), "abc");
    assert_eq!(s.tree.selection(input), Some((3, 3)));
    text(&mut s, "é!");
    assert_eq!(s.tree.text(input).unwrap(), "abcé!");
    assert_eq!(s.tree.selection(input), Some((5, 5)));
}

#[test]
fn text_with_no_focused_input_is_ignored() {
    let (mut s, input) = scene();
    assert_eq!(text(&mut s, "abc"), []);
    assert_eq!(s.tree.text(input).unwrap(), "");
}

#[test]
fn control_characters_are_dropped() {
    let (mut s, input) = focused();
    assert_eq!(text(&mut s, "a\nb\t"), [UiEvent::Changed(input)]);
    assert_eq!(s.tree.text(input).unwrap(), "ab");
    assert_eq!(text(&mut s, "\n"), []);
}

#[test]
fn backspace_and_delete_remove_around_the_caret() {
    let (mut s, input) = focused();
    text(&mut s, "abcd");
    key(&mut s, LEFT, 0);
    assert_eq!(only_changes(&key(&mut s, BACKSPACE, 0)), [&UiEvent::Changed(input)]);
    assert_eq!(s.tree.text(input).unwrap(), "abd");
    key(&mut s, DELETE, 0);
    assert_eq!(s.tree.text(input).unwrap(), "ab");
    key(&mut s, HOME, 0);
    assert_eq!(only_changes(&key(&mut s, BACKSPACE, 0)), [] as [&UiEvent; 0], "nothing before the start");
    key(&mut s, END, 0);
    assert_eq!(only_changes(&key(&mut s, DELETE, 0)), [] as [&UiEvent; 0], "nothing after the end");
}

#[test]
fn arrows_move_the_caret_and_stop_at_the_ends() {
    let (mut s, input) = focused();
    text(&mut s, "abc");
    for expected in [2, 1, 0, 0] {
        key(&mut s, LEFT, 0);
        assert_eq!(s.tree.selection(input), Some((expected, expected)));
    }
    for expected in [1, 2, 3, 3] {
        key(&mut s, RIGHT, 0);
        assert_eq!(s.tree.selection(input), Some((expected, expected)));
    }
}

#[test]
fn shift_extends_a_selection_that_typing_replaces() {
    let (mut s, input) = focused();
    text(&mut s, "hello");
    key(&mut s, LEFT, SHIFT);
    key(&mut s, LEFT, SHIFT);
    assert_eq!(s.tree.selection(input), Some((3, 5)));
    text(&mut s, "p");
    assert_eq!(s.tree.text(input).unwrap(), "help");
    assert_eq!(s.tree.selection(input), Some((4, 4)));
}

#[test]
fn an_arrow_collapses_a_selection_to_its_edge() {
    let (mut s, input) = focused();
    text(&mut s, "hello");
    key(&mut s, HOME, SHIFT);
    assert_eq!(s.tree.selection(input), Some((0, 5)));
    key(&mut s, RIGHT, 0);
    assert_eq!(s.tree.selection(input), Some((5, 5)));
}

#[test]
fn control_a_selects_everything_and_backspace_clears_it() {
    let (mut s, input) = focused();
    text(&mut s, "one two");
    key(&mut s, 'a' as u32, CTRL);
    assert_eq!(s.tree.selection(input), Some((7, 0)));
    key(&mut s, BACKSPACE, 0);
    assert_eq!(s.tree.text(input).unwrap(), "");
}

#[test]
fn control_moves_and_deletes_by_word() {
    let (mut s, input) = focused();
    text(&mut s, "one two three");
    key(&mut s, LEFT, CTRL);
    assert_eq!(s.tree.selection(input), Some((8, 8)));
    key(&mut s, LEFT, CTRL);
    assert_eq!(s.tree.selection(input), Some((4, 4)));
    key(&mut s, RIGHT, CTRL);
    assert_eq!(s.tree.selection(input), Some((7, 7)));
    key(&mut s, BACKSPACE, CTRL);
    assert_eq!(s.tree.text(input).unwrap(), "one  three");
}

#[test]
fn enter_submits_and_keys_are_still_reported() {
    let (mut s, input) = focused();
    assert_eq!(key(&mut s, ENTER, 0), [UiEvent::Key(ENTER, 0, true), UiEvent::Submit(input)]);
    assert_eq!(s.input(&Input::Key { code: ENTER, mods: 0, down: false }), [UiEvent::Key(ENTER, 0, false)]);
}

#[test]
fn setting_the_text_from_outside_keeps_the_caret_in_range() {
    let (mut s, input) = focused();
    text(&mut s, "abcdef");
    s.tree.set_text(input, "ab").unwrap();
    assert_eq!(s.tree.selection(input), Some((2, 2)));
    text(&mut s, "!");
    assert_eq!(s.tree.text(input).unwrap(), "ab!");
}

#[test]
fn only_inputs_have_a_selection() {
    let (s, _) = scene();
    assert_eq!(s.tree.selection(0), None);
}

#[test]
fn a_press_places_the_caret_and_a_drag_selects() {
    let (mut s, input) = focused();
    text(&mut s, "abcdef");
    s.present();
    let g = s.painter.input_geometry(&s.tree, input).unwrap();
    let at = |i: usize| g.content.x + g.xs[i] + 1.0;
    s.input(&Input::PointerDown { x: at(3), y: 20.0 });
    assert_eq!(s.tree.selection(input), Some((3, 3)));
    s.input(&Input::PointerMove { x: at(5), y: 20.0 });
    assert_eq!(s.tree.selection(input), Some((5, 3)));
    s.input(&Input::PointerUp { x: at(5), y: 20.0 });
    s.input(&Input::PointerMove { x: at(1), y: 20.0 });
    assert_eq!(s.tree.selection(input), Some((5, 3)), "no drag without the button");
}

fn bluish(s: &Surface, rect: gui::Rect) -> usize {
    let mut n = 0;
    for y in rect.y as u32..rect.bottom() as u32 {
        for x in rect.x as u32..rect.right() as u32 {
            let p = s.pixel(x, y).unwrap();
            let (r, b) = (p >> 24 & 0xff, p >> 8 & 0xff);
            if b > r + 40 {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn a_selection_is_painted_only_while_the_input_has_the_focus() {
    let (mut s, input) = focused();
    text(&mut s, "select me");
    s.present();
    let area = s.tree.rect(input);
    assert_eq!(bluish(&s, area), 0);
    key(&mut s, 'a' as u32, CTRL);
    s.present();
    assert!(bluish(&s, area) > 200, "{}", bluish(&s, area));
    s.input(&Input::PointerDown { x: 230.0, y: 55.0 });
    s.present();
    assert_eq!(bluish(&s, area), 0, "blurred");
}

#[test]
fn the_caret_is_a_bar_in_the_text_color_while_focused() {
    let (mut s, input) = focused();
    s.present();
    let g = s.painter.input_geometry(&s.tree, input).unwrap();
    let (x, y) = (g.content.x as u32, (g.content.y + 8.0) as u32);
    assert_eq!(s.pixel(x, y), Some(0x000000ff), "the caret at the start of an empty input");
    s.input(&Input::PointerDown { x: 230.0, y: 55.0 });
    s.present();
    assert_eq!(s.pixel(x, y), Some(0xffffffff));
}

#[test]
fn a_long_text_scrolls_to_keep_the_caret_in_view() {
    let (mut s, input) = focused();
    text(&mut s, &"w".repeat(60));
    s.present();
    let g = s.painter.input_geometry(&s.tree, input).unwrap();
    assert!(g.shift > 0.0);
    let caret_x = g.content.x - g.shift + g.xs[60];
    assert!(caret_x >= g.content.x && caret_x <= g.content.right(), "{caret_x} in {:?}", g.content);
}
