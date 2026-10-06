//! Text nodes with runs: weight, italic, mono, colour, underline and links, painted with only the bundled fonts.

use gui::{Align, Dim, Direction, Input, Kind, Painter, Rect, Run, StyleDef, Surface, UiEvent};

fn surface(w: u32, h: u32) -> Surface {
    let mut s = Surface::new(w, h, Painter::bundled_only()).unwrap();
    let id = style(&mut s, |d| {
        d.direction = Direction::Column;
        d.align_items = Some(Align::Start);
    });
    s.tree.set_style(0, id).unwrap();
    s
}

fn style(s: &mut Surface, f: impl FnOnce(&mut StyleDef)) -> u32 {
    let mut def = StyleDef::default();
    f(&mut def);
    s.tree.intern_style(def)
}

fn text(s: &mut Surface, text: &str, width: Option<f32>) -> u32 {
    let n = s.tree.add(0, Kind::Text).unwrap();
    s.tree.set_text(n, text).unwrap();
    let id = style(s, |d| {
        if let Some(w) = width {
            d.size = [Dim::Px(w), Dim::Auto];
        }
    });
    s.tree.set_style(n, id).unwrap();
    n
}

fn run(len: usize) -> Run {
    Run::plain(len)
}

fn width_of(s: &mut Surface, node: u32) -> f32 {
    s.present();
    s.tree.rect(node).w
}

/// Pixels in `area` for which `want` holds.
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

fn is_red(p: u32) -> bool {
    let (r, g, b) = ((p >> 24) & 0xff, (p >> 16) & 0xff, (p >> 8) & 0xff);
    r > 200 && g < 80 && b < 80
}

fn is_dark(p: u32) -> bool {
    (p >> 24) & 0xff < 128
}

#[test]
fn runs_must_cover_the_text_at_character_boundaries() {
    let mut s = surface(200, 100);
    let n = text(&mut s, "", None);
    assert!(s.tree.set_runs(n, "héllo", vec![run(2), run(4)]).is_err(), "a split inside é");
    assert!(s.tree.set_runs(n, "hello", vec![run(2), run(2)]).is_err(), "short");
    assert!(s.tree.set_runs(n, "hello", vec![run(9)]).is_err(), "long");
    assert!(s.tree.set_runs(n, "héllo", vec![run(3), run(3)]).is_ok());
    let b = s.tree.add(0, Kind::Box).unwrap();
    assert!(s.tree.set_runs(b, "x", vec![run(1)]).is_err(), "a box holds no text");
    let i = s.tree.add(0, Kind::Input).unwrap();
    assert!(s.tree.set_runs(i, "x", vec![run(1)]).is_err(), "an input holds plain text");
}

#[test]
fn set_text_drops_the_runs() {
    let mut s = surface(200, 100);
    let n = text(&mut s, "Hello", None);
    let plain = width_of(&mut s, n);
    s.tree.set_runs(n, "Hello", vec![Run { weight: 700, ..run(5) }]).unwrap();
    assert!(width_of(&mut s, n) > plain, "bold is wider");
    assert_eq!(s.tree.runs(n).len(), 1);
    s.tree.set_text(n, "Hello").unwrap();
    assert!(s.tree.runs(n).is_empty());
    assert_eq!(width_of(&mut s, n), plain, "the same text as plain again");
}

#[test]
fn a_bold_run_is_wider_than_a_plain_one() {
    let mut s = surface(300, 100);
    let plain = text(&mut s, "Hello world", None);
    let mixed = text(&mut s, "", None);
    s.tree.set_runs(mixed, "Hello world", vec![run(6), Run { weight: 700, ..run(5) }]).unwrap();
    s.present();
    let (a, b) = (s.tree.rect(plain).w, s.tree.rect(mixed).w);
    assert!(b > a, "{a} {b}");
}

#[test]
fn a_mono_run_has_equal_advances() {
    let mut s = surface(300, 100);
    let sans = text(&mut s, "iiiiiiii", None);
    let mono = text(&mut s, "", None);
    s.tree.set_runs(mono, "iiiiiiii", vec![Run { mono: true, ..run(8) }]).unwrap();
    s.present();
    let (a, b) = (s.tree.rect(sans).w, s.tree.rect(mono).w);
    assert!(b > a * 1.5, "a mono i is as wide as an m: {a} {b}");
    assert!((b / 8.0 - 9.6).abs() < 0.6, "DejaVu Sans Mono at 16 px is 9.6 wide: {}", b / 8.0);
}

#[test]
fn an_italic_run_paints_slanted_glyphs_of_the_same_advance() {
    let mut s = surface(300, 100);
    let upright = text(&mut s, "Hill", None);
    s.present();
    let before: Vec<u8> = s.pixels().to_vec();
    let width = s.tree.rect(upright).w;
    s.tree.set_runs(upright, "Hill", vec![Run { italic: true, ..run(4) }]).unwrap();
    s.present();
    assert_eq!(s.tree.rect(upright).w, width);
    assert_ne!(s.pixels(), &before[..], "the slant changes the pixels");
}

#[test]
fn a_run_paints_in_its_own_colour() {
    let mut s = surface(300, 100);
    let n = text(&mut s, "", None);
    s.tree.set_runs(n, "0000000000red", vec![run(10), Run { color: 0xff0000ff, ..run(3) }]).unwrap();
    s.present();
    let r = s.tree.rect(n);
    let split = r.x + r.w * 10.0 / 13.0;
    assert_eq!(count(&s, Rect::new(r.x, r.y, split - r.x - 2.0, r.h), is_red), 0, "the plain run stays black");
    assert!(count(&s, Rect::new(split + 2.0, r.y, r.right() - split - 2.0, r.h), is_red) > 20, "the red run is red");
}

#[test]
fn an_underline_adds_a_line_under_the_run() {
    let mut s = surface(300, 100);
    let n = text(&mut s, "", None);
    s.tree.set_runs(n, "abcdefgh", vec![run(8)]).unwrap();
    s.present();
    let r = s.tree.rect(n);
    let plain = count(&s, r, is_dark);
    s.tree.set_runs(n, "abcdefgh", vec![Run { underline: true, ..run(8) }]).unwrap();
    s.present();
    let lined = count(&s, r, is_dark);
    assert!(lined >= plain + (r.w * 0.8) as usize, "a line about as long as the text: {plain} {lined}");
}

#[test]
fn a_strike_adds_a_line_through_the_run() {
    let mut s = surface(300, 100);
    let n = text(&mut s, "", None);
    s.tree.set_runs(n, "iiiiiiii", vec![run(8)]).unwrap();
    s.present();
    let r = s.tree.rect(n);
    let plain = count(&s, r, is_dark);
    s.tree.set_runs(n, "iiiiiiii", vec![Run { strike: true, ..run(8) }]).unwrap();
    s.present();
    let struck = count(&s, r, is_dark);
    assert!(struck >= plain + (r.w * 0.5) as usize, "a line about as long as the text: {plain} {struck}");
}

fn click(s: &mut Surface, x: f32, y: f32) -> Vec<UiEvent> {
    let mut events = s.input(&Input::PointerDown { x, y });
    events.extend(s.input(&Input::PointerUp { x, y }));
    events
}

/// A node holding ten mono characters, then the link `docs` (id 7), then ` here`.
fn link_scene() -> (Surface, u32) {
    let mut s = surface(300, 100);
    let n = text(&mut s, "", None);
    let mono = Run { mono: true, ..run(10) };
    s.tree.set_runs(n, "0000000000docs here", vec![mono, Run { link: 7, underline: true, ..run(4) }, run(5)]).unwrap();
    s.present();
    (s, n)
}

#[test]
fn a_click_on_a_link_run_raises_link_with_its_id() {
    let (mut s, n) = link_scene();
    let r = s.tree.rect(n);
    let events = click(&mut s, r.x + 110.0, r.y + 8.0);
    assert!(events.contains(&UiEvent::Link(n, 7)), "{events:?}");
}

#[test]
fn a_click_beside_the_link_raises_nothing() {
    let (mut s, n) = link_scene();
    let r = s.tree.rect(n);
    let events = click(&mut s, r.x + 30.0, r.y + 8.0);
    assert!(!events.iter().any(|e| matches!(e, UiEvent::Link(..))), "{events:?}");
}

#[test]
fn a_press_on_the_link_released_elsewhere_raises_nothing() {
    let (mut s, n) = link_scene();
    let r = s.tree.rect(n);
    s.input(&Input::PointerDown { x: r.x + 110.0, y: r.y + 8.0 });
    let events = s.input(&Input::PointerUp { x: r.x + 30.0, y: r.y + 8.0 });
    assert!(!events.iter().any(|e| matches!(e, UiEvent::Link(..))), "{events:?}");
}

#[test]
fn runs_wrap_across_their_boundaries() {
    let mut s = surface(300, 200);
    let n = text(&mut s, "", Some(100.0));
    s.tree.set_runs(n, "one two three four five six", vec![run(8), Run { weight: 700, ..run(10) }, run(9)]).unwrap();
    s.present();
    let r = s.tree.rect(n);
    assert_eq!(r.w, 100.0);
    assert!(r.h >= 3.0 * 19.0, "{r:?}");
}

#[test]
fn a_link_on_the_second_line_is_found() {
    let mut s = surface(300, 200);
    let n = text(&mut s, "", Some(100.0));
    s.tree.set_runs(n, "aaaa bbbb cccc dddd eeee", vec![run(20), Run { link: 3, ..run(4) }]).unwrap();
    s.present();
    let r = s.tree.rect(n);
    let line_height = r.h / 3.0;
    let events = click(&mut s, r.x + 20.0, r.y + line_height * 2.5);
    assert!(events.contains(&UiEvent::Link(n, 3)), "{events:?}");
}

#[test]
fn link_events_have_a_wire_form() {
    assert_eq!(UiEvent::Link(4, 9).to_wire(), [13.0, 4.0, 9.0, 0.0, 0.0]);
}
