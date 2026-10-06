//! Hover, press, click, wheel and focus on a surface, with no window.

use gui::{Dim, Direction, Input, Kind, Painter, StyleDef, Surface, UiEvent};

const RED: u32 = 0xff0000ff;
const GREEN: u32 = 0x00ff00ff;
const BLUE: u32 = 0x0000ffff;

fn surface(w: u32, h: u32) -> Surface {
    Surface::new(w, h, Painter::bundled_only()).unwrap()
}

fn style(s: &mut Surface, f: impl FnOnce(&mut StyleDef)) -> u32 {
    let mut def = StyleDef::default();
    f(&mut def);
    s.tree.intern_style(def)
}

fn sized(s: &mut Surface, node: u32, w: f32, h: f32, background: u32) {
    let id = style(s, |d| {
        d.size = [Dim::Px(w), Dim::Px(h)];
        d.background = background;
    });
    s.tree.set_style(node, id).unwrap();
}

/// A 100 by 50 red button at the top left that turns green under the pointer and blue while held.
fn button_scene() -> (Surface, u32) {
    let mut s = surface(200, 100);
    let hover = style(&mut s, |d| d.background = GREEN);
    let pressed = style(&mut s, |d| d.background = BLUE);
    let id = style(&mut s, |d| {
        d.size = [Dim::Px(100.0), Dim::Px(50.0)];
        d.background = RED;
        d.hover = hover;
        d.pressed = pressed;
    });
    let button = s.tree.add(0, Kind::Button).unwrap();
    s.tree.set_style(button, id).unwrap();
    s.present();
    (s, button)
}

fn at(s: &Surface, x: u32, y: u32) -> u32 {
    s.pixel(x, y).unwrap()
}

fn mv(x: f32, y: f32) -> Input {
    Input::PointerMove { x, y }
}

fn down(x: f32, y: f32) -> Input {
    Input::PointerDown { x, y }
}

fn up(x: f32, y: f32) -> Input {
    Input::PointerUp { x, y }
}

#[test]
fn the_pointer_entering_and_leaving_a_button_raises_events_and_the_hover_style() {
    let (mut s, button) = button_scene();
    assert_eq!(at(&s, 10, 10), RED);
    assert_eq!(s.input(&mv(10.0, 10.0)), [UiEvent::Enter(button), UiEvent::PointerMove(button, 10.0, 10.0)]);
    assert!(s.tree.is_hovered(button));
    assert!(!s.present().is_empty());
    assert_eq!(at(&s, 10, 10), GREEN);
    assert_eq!(s.input(&mv(20.0, 20.0)), [UiEvent::PointerMove(button, 20.0, 20.0)]);
    assert_eq!(s.input(&mv(150.0, 80.0)).first(), Some(&UiEvent::Leave(button)));
    s.present();
    assert_eq!(at(&s, 10, 10), RED);
}

#[test]
fn the_pointer_leaving_the_window_clears_the_hover() {
    let (mut s, button) = button_scene();
    s.input(&mv(10.0, 10.0));
    assert_eq!(s.input(&Input::PointerLeave), [UiEvent::Leave(button)]);
    assert!(!s.tree.is_hovered(button));
}

#[test]
fn a_press_and_release_on_a_button_is_a_click() {
    let (mut s, button) = button_scene();
    s.input(&mv(10.0, 10.0));
    assert_eq!(s.input(&down(10.0, 10.0)), [UiEvent::PointerDown(button, 10.0, 10.0)]);
    s.present();
    assert_eq!(at(&s, 10, 10), BLUE);
    assert_eq!(s.input(&up(10.0, 10.0)), [UiEvent::PointerUp(button, 10.0, 10.0), UiEvent::Click(button)]);
    s.present();
    assert_eq!(at(&s, 10, 10), GREEN);
}

#[test]
fn a_release_away_from_the_button_is_no_click() {
    let (mut s, button) = button_scene();
    s.input(&down(10.0, 10.0));
    let events = s.input(&up(150.0, 80.0));
    assert!(events.contains(&UiEvent::Leave(button)));
    assert!(!events.iter().any(|e| matches!(e, UiEvent::Click(_))));
}

#[test]
fn the_pressed_look_shows_only_while_the_pointer_is_over_the_button() {
    let (mut s, _) = button_scene();
    s.input(&down(10.0, 10.0));
    s.input(&mv(150.0, 80.0));
    s.present();
    assert_eq!(at(&s, 10, 10), RED);
    s.input(&mv(10.0, 10.0));
    s.present();
    assert_eq!(at(&s, 10, 10), BLUE);
}

#[test]
fn a_click_on_text_inside_a_button_names_the_button() {
    let mut s = surface(200, 100);
    let button = s.tree.add(0, Kind::Button).unwrap();
    let label = s.tree.add(button, Kind::Text).unwrap();
    s.tree.set_text(label, "Go").unwrap();
    sized(&mut s, button, 100.0, 50.0, RED);
    s.present();
    s.input(&down(2.0, 2.0));
    let events = s.input(&up(2.0, 2.0));
    assert!(events.contains(&UiEvent::Click(button)), "{events:?}");
    assert!(events.iter().any(|e| matches!(e, UiEvent::PointerUp(n, ..) if *n == label || *n == button)));
}

#[test]
fn pressing_a_button_focuses_it_and_pressing_a_plain_box_does_not() {
    let (mut s, button) = button_scene();
    s.input(&down(10.0, 10.0));
    assert_eq!(s.tree.focus(), Some(button));
    s.input(&up(10.0, 10.0));
    s.input(&down(150.0, 80.0));
    assert_eq!(s.tree.focus(), None);
}

#[test]
fn removing_the_hovered_node_clears_the_state() {
    let (mut s, button) = button_scene();
    s.input(&mv(10.0, 10.0));
    s.input(&down(10.0, 10.0));
    s.tree.remove(button).unwrap();
    assert_eq!(s.tree.focus(), None);
    s.present();
    assert_eq!(s.input(&up(10.0, 10.0)), [UiEvent::PointerUp(0, 10.0, 10.0)]);
    let again = s.tree.add(0, Kind::Button).unwrap();
    assert!(!s.tree.is_hovered(again) && !s.tree.is_pressed(again));
}

fn scroll_scene() -> (Surface, u32) {
    let mut s = surface(100, 100);
    let area = s.tree.add(0, Kind::Scroll).unwrap();
    let id = style(&mut s, |d| d.size = [Dim::Px(100.0), Dim::Px(100.0)]);
    s.tree.set_style(area, id).unwrap();
    let tall = s.tree.add(area, Kind::Box).unwrap();
    sized(&mut s, tall, 100.0, 300.0, RED);
    s.present();
    (s, area)
}

#[test]
fn the_wheel_scrolls_a_scroll_node_and_raises_nothing() {
    let (mut s, area) = scroll_scene();
    assert_eq!(s.input(&Input::Wheel { x: 50.0, y: 50.0, dx: 0.0, dy: 40.0 }), []);
    assert_eq!(s.tree.scroll_offset(area), [0.0, 40.0]);
    assert!(!s.present().is_empty());
}

#[test]
fn the_wheel_stops_at_the_end_of_the_content() {
    let (mut s, area) = scroll_scene();
    s.input(&Input::Wheel { x: 50.0, y: 50.0, dx: 0.0, dy: 1000.0 });
    assert_eq!(s.tree.scroll_offset(area), [0.0, 200.0]);
    let at_end = s.input(&Input::Wheel { x: 50.0, y: 50.0, dx: 0.0, dy: 10.0 });
    assert!(matches!(at_end[..], [UiEvent::Wheel(_, 0.0, 10.0)]), "{at_end:?}");
}

#[test]
fn the_wheel_over_something_that_does_not_scroll_is_an_event() {
    let (mut s, _) = button_scene();
    assert_eq!(s.input(&Input::Wheel { x: 150.0, y: 80.0, dx: 0.0, dy: 5.0 }), [UiEvent::Wheel(0, 0.0, 5.0)]);
}

#[test]
fn an_inner_scroll_passes_the_wheel_to_the_outer_one_at_its_end() {
    let mut s = surface(100, 100);
    let outer = s.tree.add(0, Kind::Scroll).unwrap();
    let id = style(&mut s, |d| {
        d.size = [Dim::Px(100.0), Dim::Px(100.0)];
        d.direction = Direction::Column;
    });
    s.tree.set_style(outer, id).unwrap();
    let inner = s.tree.add(outer, Kind::Scroll).unwrap();
    let inner_style = style(&mut s, |d| {
        d.size = [Dim::Px(100.0), Dim::Px(100.0)];
        d.shrink = 0.0;
    });
    s.tree.set_style(inner, inner_style).unwrap();
    let spacer = s.tree.add(outer, Kind::Box).unwrap();
    let spacer_style = style(&mut s, |d| {
        d.size = [Dim::Px(100.0), Dim::Px(100.0)];
        d.shrink = 0.0;
        d.background = GREEN;
    });
    s.tree.set_style(spacer, spacer_style).unwrap();
    let tall = s.tree.add(inner, Kind::Box).unwrap();
    sized(&mut s, tall, 100.0, 150.0, RED);
    s.present();
    s.input(&Input::Wheel { x: 50.0, y: 50.0, dx: 0.0, dy: 500.0 });
    assert_eq!(s.tree.scroll_offset(inner), [0.0, 50.0]);
    assert_eq!(s.tree.scroll_offset(outer), [0.0, 0.0]);
    s.input(&Input::Wheel { x: 50.0, y: 50.0, dx: 0.0, dy: 30.0 });
    assert_eq!(s.tree.scroll_offset(outer), [0.0, 30.0]);
}

#[test]
fn resize_and_close_pass_through_and_keys_are_reported() {
    let (mut s, _) = button_scene();
    assert_eq!(s.input(&Input::Resize { width: 300, height: 120 }), [UiEvent::Resize(300, 120)]);
    assert_eq!((s.width(), s.height()), (300, 120));
    assert_eq!(s.input(&Input::Resize { width: 0, height: 120 }), []);
    assert_eq!(s.input(&Input::Key { code: 13, mods: 0, down: true }), [UiEvent::Key(13, 0, true)]);
    assert_eq!(s.input(&Input::Close), [UiEvent::Close]);
}
