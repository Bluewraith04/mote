//! Text areas: line breaks, wrapping, caret movement by line, scrolling, undo, and the paint.

use gui::keys::{BACKSPACE, CTRL, DOWN, END, ENTER, HOME, LEFT, SHIFT, UP};
use gui::{Dim, Input, Kind, Len, Painter, Rect, StyleDef, Surface, UiEvent};

/// A 100 by 60 text area at (10, 10) in a 240 by 200 window, focused, with no text yet; its lines are 20 pixels tall.
fn scene() -> (Surface, u32) {
    let mut s = Surface::new(240, 200, Painter::bundled_only()).unwrap();
    let id = s.tree.intern_style(StyleDef {
        position: gui::Position::Absolute,
        inset: [Dim::Px(10.0), Dim::Auto, Dim::Auto, Dim::Px(10.0)],
        size: [Dim::Px(100.0), Dim::Px(60.0)],
        padding: [Len::Px(4.0); 4],
        ..StyleDef::default()
    });
    let area = s.tree.add(0, Kind::TextArea).unwrap();
    s.tree.set_style(area, id).unwrap();
    s.present();
    s.input(&Input::PointerDown { x: 20.0, y: 20.0 });
    s.input(&Input::PointerUp { x: 20.0, y: 20.0 });
    assert_eq!(s.tree.focus(), Some(area));
    (s, area)
}

fn text(s: &mut Surface, t: &str) -> Vec<UiEvent> {
    let events = s.input(&Input::Text(t.to_string()));
    s.present();
    events
}

fn key(s: &mut Surface, code: u32, mods: u32) -> Vec<UiEvent> {
    let events = s.input(&Input::Key { code, mods, down: true });
    s.present();
    events
}

fn caret(s: &Surface, area: u32) -> usize {
    s.tree.selection(area).unwrap().0
}

fn count(s: &Surface, area: Rect, want: impl Fn(u32) -> bool) -> usize {
    let mut n = 0;
    for y in area.y as u32..area.bottom() as u32 {
        for x in area.x as u32..area.right() as u32 {
            if s.pixel(x, y).is_some_and(&want) {
                n += 1;
            }
        }
    }
    n
}

fn is_dark(p: u32) -> bool {
    (p >> 24) & 0xff < 128
}

fn is_blue(p: u32) -> bool {
    let (r, b) = ((p >> 24) & 0xff, (p >> 8) & 0xff);
    b > r + 40
}

#[test]
fn enter_breaks_the_line_and_control_enter_submits() {
    let (mut s, area) = scene();
    text(&mut s, "ab");
    let events = key(&mut s, ENTER, 0);
    assert!(events.contains(&UiEvent::Changed(area)) && !events.contains(&UiEvent::Submit(area)), "{events:?}");
    text(&mut s, "cd");
    assert_eq!(s.tree.text(area).unwrap(), "ab\ncd");
    let events = key(&mut s, ENTER, CTRL);
    assert!(events.contains(&UiEvent::Submit(area)) && !events.contains(&UiEvent::Changed(area)), "{events:?}");
    assert_eq!(s.tree.text(area).unwrap(), "ab\ncd");
}

#[test]
fn the_text_the_system_sends_with_enter_is_not_inserted_again() {
    let (mut s, area) = scene();
    key(&mut s, ENTER, 0);
    assert_eq!(text(&mut s, "\r"), []);
    assert_eq!(s.tree.text(area).unwrap(), "\n");
    text(&mut s, "a\tb\r\nc");
    assert_eq!(s.tree.text(area).unwrap(), "\na\tb\nc");
}

#[test]
fn a_line_input_still_drops_line_breaks() {
    let mut s = Surface::new(100, 40, Painter::bundled_only()).unwrap();
    let i = s.tree.add(0, Kind::Input).unwrap();
    s.present();
    s.tree.focus_on(Some(i));
    s.input(&Input::Text("a\nb".to_string()));
    assert_eq!(s.tree.text(i).unwrap(), "ab");
}

#[test]
fn text_wraps_at_the_width_and_a_final_break_is_a_line() {
    let (mut s, _) = scene();
    let column = s.tree.intern_style(StyleDef { direction: gui::Direction::Column, align_items: Some(gui::Align::Start), ..StyleDef::default() });
    s.tree.set_style(0, column).unwrap();
    let style = s.tree.intern_style(StyleDef { size: [Dim::Px(100.0), Dim::Auto], ..StyleDef::default() });
    let free = s.tree.add(0, Kind::TextArea).unwrap();
    s.tree.set_style(free, style).unwrap();
    s.tree.set_text(free, "one two three four five six").unwrap();
    s.present();
    let wrapped = s.tree.rect(free).h;
    assert!(wrapped >= 60.0, "{wrapped}");
    s.tree.set_text(free, "one").unwrap();
    s.present();
    let one = s.tree.rect(free).h;
    s.tree.set_text(free, "one\n").unwrap();
    s.present();
    assert_eq!(s.tree.rect(free).h, one * 2.0);
}

#[test]
fn up_and_down_move_by_line_and_keep_the_column() {
    let (mut s, area) = scene();
    text(&mut s, "abcdef\nab\nabcdef");
    assert_eq!(caret(&s, area), 16);
    key(&mut s, UP, 0);
    assert_eq!(caret(&s, area), 9, "the short line's end");
    key(&mut s, UP, 0);
    assert_eq!(caret(&s, area), 6, "the column of the first move is kept");
    key(&mut s, UP, 0);
    assert_eq!(caret(&s, area), 0, "up on the first line goes to the start");
    key(&mut s, DOWN, 0);
    key(&mut s, DOWN, 0);
    key(&mut s, DOWN, 0);
    assert_eq!(caret(&s, area), 16, "down on the last line goes to the end");
}

#[test]
fn home_and_end_go_to_the_ends_of_the_line() {
    let (mut s, area) = scene();
    text(&mut s, "abc\ndef");
    key(&mut s, HOME, 0);
    assert_eq!(caret(&s, area), 4);
    key(&mut s, UP, 0);
    key(&mut s, END, 0);
    assert_eq!(caret(&s, area), 3);
    key(&mut s, HOME, CTRL);
    assert_eq!(caret(&s, area), 0);
    key(&mut s, END, CTRL);
    assert_eq!(caret(&s, area), 7);
}

#[test]
fn shift_with_up_selects_across_lines() {
    let (mut s, area) = scene();
    text(&mut s, "abc\ndef");
    key(&mut s, UP, SHIFT);
    assert_eq!(s.tree.selection(area), Some((3, 7)));
    text(&mut s, "X");
    assert_eq!(s.tree.text(area).unwrap(), "abcX");
}

#[test]
fn a_wrapped_line_is_walked_by_its_pieces() {
    let (mut s, area) = scene();
    text(&mut s, "aaaa bbbb cccc dddd eeee ffff");
    let end = caret(&s, area);
    key(&mut s, UP, 0);
    let one_up = caret(&s, area);
    assert!(one_up < end && one_up > 0, "{one_up}");
    key(&mut s, HOME, 0);
    let home = caret(&s, area);
    assert!(home > 0 && home < one_up, "the start of that wrapped line: {home}");
    key(&mut s, END, 0);
    assert!(caret(&s, area) > home && caret(&s, area) < end);
}

#[test]
fn a_click_puts_the_caret_on_the_line_under_it() {
    let (mut s, area) = scene();
    text(&mut s, "abc\ndef\nghi");
    s.input(&Input::PointerDown { x: 14.0, y: 36.0 });
    s.input(&Input::PointerUp { x: 14.0, y: 36.0 });
    assert_eq!(caret(&s, area), 4, "the start of the second line");
    s.input(&Input::PointerDown { x: 104.0, y: 36.0 });
    s.input(&Input::PointerUp { x: 104.0, y: 36.0 });
    assert_eq!(caret(&s, area), 7, "the end of the second line");
}

#[test]
fn dragging_selects_across_lines() {
    let (mut s, area) = scene();
    text(&mut s, "abc\ndef\nghi");
    s.input(&Input::PointerDown { x: 14.0, y: 20.0 });
    s.input(&Input::PointerMove { x: 104.0, y: 36.0 });
    s.input(&Input::PointerUp { x: 104.0, y: 36.0 });
    assert_eq!(s.tree.selection(area), Some((7, 0)));
}

#[test]
fn the_text_scrolls_to_keep_the_caret_in_view() {
    let (mut s, area) = scene();
    text(&mut s, "1\n2\n3\n4\n5\n6\n7\n8");
    let top = s.tree.scroll_top(area);
    assert!(top >= 100.0 - 4.0, "eight lines of 20 in 52: {top}");
    for _ in 0..7 {
        key(&mut s, UP, 0);
    }
    assert_eq!(s.tree.scroll_top(area), 0.0);
}

#[test]
fn the_wheel_scrolls_the_text_and_stops_at_its_ends() {
    let (mut s, area) = scene();
    text(&mut s, "1\n2\n3\n4\n5\n6\n7\n8");
    key(&mut s, HOME, CTRL);
    assert_eq!(s.tree.scroll_top(area), 0.0);
    let events = s.input(&Input::Wheel { x: 50.0, y: 40.0, dx: 0.0, dy: 30.0 });
    assert_eq!(events, []);
    assert_eq!(s.tree.scroll_top(area), 30.0);
    s.input(&Input::Wheel { x: 50.0, y: 40.0, dx: 0.0, dy: 1000.0 });
    let max = s.tree.scroll_top(area);
    assert!(max > 30.0 && max < 200.0, "{max}");
    let events = s.input(&Input::Wheel { x: 50.0, y: 40.0, dx: 0.0, dy: 10.0 });
    assert_eq!(events, [UiEvent::Wheel(area, 0.0, 10.0)], "at the end the turn is the program's");
}

#[test]
fn undo_and_redo_step_through_edits_and_typing_is_one_step() {
    let (mut s, area) = scene();
    text(&mut s, "a");
    text(&mut s, "b");
    text(&mut s, "c");
    key(&mut s, ENTER, 0);
    text(&mut s, "d");
    key(&mut s, 'z' as u32, CTRL);
    assert_eq!(s.tree.text(area).unwrap(), "abc\n");
    key(&mut s, 'z' as u32, CTRL);
    assert_eq!(s.tree.text(area).unwrap(), "abc");
    let events = key(&mut s, 'z' as u32, CTRL);
    assert!(events.contains(&UiEvent::Changed(area)));
    assert_eq!(s.tree.text(area).unwrap(), "");
    assert_eq!(key(&mut s, 'z' as u32, CTRL).iter().filter(|e| matches!(e, UiEvent::Changed(_))).count(), 0, "nothing left to undo");
    key(&mut s, 'y' as u32, CTRL);
    assert_eq!(s.tree.text(area).unwrap(), "abc");
    assert_eq!(caret(&s, area), 3);
    key(&mut s, 'Z' as u32, CTRL | SHIFT);
    assert_eq!(s.tree.text(area).unwrap(), "abc\n");
}

#[test]
fn a_new_edit_clears_redo_and_deleting_is_one_step() {
    let (mut s, area) = scene();
    text(&mut s, "hello");
    key(&mut s, BACKSPACE, 0);
    key(&mut s, BACKSPACE, 0);
    assert_eq!(s.tree.text(area).unwrap(), "hel");
    key(&mut s, 'z' as u32, CTRL);
    assert_eq!(s.tree.text(area).unwrap(), "hello");
    text(&mut s, "!");
    key(&mut s, 'y' as u32, CTRL);
    assert_eq!(s.tree.text(area).unwrap(), "hello!", "redo was cleared");
}

#[test]
fn moving_the_caret_ends_a_typing_step_and_a_program_text_clears_the_history() {
    let (mut s, area) = scene();
    text(&mut s, "ab");
    key(&mut s, LEFT, 0);
    text(&mut s, "X");
    key(&mut s, 'z' as u32, CTRL);
    assert_eq!(s.tree.text(area).unwrap(), "ab");
    s.tree.set_text(area, "from the program").unwrap();
    key(&mut s, 'z' as u32, CTRL);
    assert_eq!(s.tree.text(area).unwrap(), "from the program");
    s.tree.set_text(area, "from the program").unwrap();
}

#[test]
fn a_selection_over_lines_paints_a_band_per_line_and_the_caret_a_bar() {
    let (mut s, area) = scene();
    text(&mut s, "abc\ndef\nghi");
    s.present();
    let rect = s.tree.rect(area);
    let none = count(&s, rect, is_blue);
    key(&mut s, HOME, CTRL);
    key(&mut s, DOWN, SHIFT);
    key(&mut s, DOWN, SHIFT);
    s.present();
    let bands = count(&s, rect, is_blue);
    assert!(bands > none + 40 * 20, "two lines of band: {none} {bands}");
    let dark = count(&s, rect, is_dark);
    assert!(dark > 0);
}

#[test]
fn the_text_is_clipped_to_the_box() {
    let (mut s, area) = scene();
    text(&mut s, "1\n2\n3\n4\n5\n6\n7\n8\n9");
    s.present();
    let rect = s.tree.rect(area);
    let below = Rect::new(rect.x, rect.bottom() + 1.0, rect.w, 40.0);
    assert_eq!(count(&s, below, is_dark), 0);
}
