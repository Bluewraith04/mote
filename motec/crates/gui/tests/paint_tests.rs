//! Painting a tree into a pixmap with only the bundled fonts, so the pixels do not depend on the machine.

use gui::{Align, Dim, Direction, Kind, Painter, Position, Rect, StyleDef, Tree};
use tiny_skia::Pixmap;

const RED: u32 = 0xff0000ff;
const GREEN: u32 = 0x00ff00ff;
const BLUE: u32 = 0x0000ffff;
const WHITE: [u8; 4] = [255, 255, 255, 255];

struct Scene {
    tree: Tree,
    painter: Painter,
    pixmap: Pixmap,
}

impl Scene {
    fn new(w: u32, h: u32) -> Scene {
        Scene { tree: Tree::new(w as f32, h as f32), painter: Painter::bundled_only(), pixmap: Pixmap::new(w, h).unwrap() }
    }

    fn style(&mut self, node: u32, f: impl FnOnce(&mut StyleDef)) {
        let mut def = StyleDef::default();
        f(&mut def);
        let id = self.tree.intern_style(def);
        self.tree.set_style(node, id).unwrap();
    }

    fn boxed(&mut self, parent: u32, x: f32, y: f32, w: f32, h: f32, color: u32) -> u32 {
        let n = self.tree.add(parent, Kind::Box).unwrap();
        self.style(n, |s| {
            s.position = Position::Absolute;
            s.inset = [Dim::Px(y), Dim::Auto, Dim::Auto, Dim::Px(x)];
            s.size = [Dim::Px(w), Dim::Px(h)];
            s.background = color;
        });
        n
    }

    /// Lays out and paints what changed; returns the damage painted.
    fn render(&mut self) -> Vec<Rect> {
        self.tree.layout(&mut self.painter);
        let damage = self.tree.take_damage();
        self.painter.paint(&mut self.tree, &mut self.pixmap, &damage);
        damage
    }

    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let p = self.pixmap.pixel(x, y).unwrap();
        [p.red(), p.green(), p.blue(), p.alpha()]
    }

    /// A fresh full repaint of the same tree.
    fn full_repaint(&mut self) -> Vec<u8> {
        let mut pixmap = Pixmap::new(self.pixmap.width(), self.pixmap.height()).unwrap();
        let all = [self.tree.viewport()];
        self.painter.paint(&mut self.tree, &mut pixmap, &all);
        pixmap.data().to_vec()
    }
}

#[test]
fn a_box_fills_exactly_its_rectangle() {
    let mut s = Scene::new(100, 100);
    s.boxed(0, 10.0, 10.0, 20.0, 20.0, RED);
    s.render();
    assert_eq!(s.pixel(10, 10), [255, 0, 0, 255]);
    assert_eq!(s.pixel(29, 29), [255, 0, 0, 255]);
    assert_eq!(s.pixel(9, 15), WHITE);
    assert_eq!(s.pixel(30, 15), WHITE);
    assert_eq!(s.pixel(15, 30), WHITE);
}

#[test]
fn a_later_sibling_paints_over_an_earlier_one() {
    let mut s = Scene::new(100, 100);
    s.boxed(0, 10.0, 10.0, 40.0, 40.0, RED);
    s.boxed(0, 30.0, 30.0, 40.0, 40.0, GREEN);
    s.render();
    assert_eq!(s.pixel(20, 20), [255, 0, 0, 255]);
    assert_eq!(s.pixel(40, 40), [0, 255, 0, 255]);
    assert_eq!(s.pixel(60, 60), [0, 255, 0, 255]);
}

#[test]
fn a_radius_rounds_the_corners() {
    let mut s = Scene::new(100, 100);
    let b = s.boxed(0, 10.0, 10.0, 40.0, 40.0, RED);
    s.style(b, |d| {
        d.position = Position::Absolute;
        d.inset = [Dim::Px(10.0), Dim::Auto, Dim::Auto, Dim::Px(10.0)];
        d.size = [Dim::Px(40.0), Dim::Px(40.0)];
        d.background = RED;
        d.radius = 20.0;
    });
    s.render();
    assert_eq!(s.pixel(10, 10), WHITE, "the corner is cut away");
    assert_eq!(s.pixel(30, 30), [255, 0, 0, 255]);
    assert_eq!(s.pixel(30, 10), [255, 0, 0, 255], "the middle of the top edge is inside");
}

#[test]
fn a_border_paints_only_its_ring() {
    let mut s = Scene::new(100, 100);
    let b = s.boxed(0, 10.0, 10.0, 40.0, 40.0, 0);
    s.style(b, |d| {
        d.position = Position::Absolute;
        d.inset = [Dim::Px(10.0), Dim::Auto, Dim::Auto, Dim::Px(10.0)];
        d.size = [Dim::Px(40.0), Dim::Px(40.0)];
        d.border = [3.0; 4];
        d.border_color = BLUE;
    });
    s.render();
    assert_eq!(s.pixel(11, 30), [0, 0, 255, 255]);
    assert_eq!(s.pixel(30, 11), [0, 0, 255, 255]);
    assert_eq!(s.pixel(30, 30), WHITE);
    assert_eq!(s.pixel(14, 30), WHITE);
}

#[test]
fn opacity_blends_with_what_is_behind() {
    let mut s = Scene::new(50, 50);
    let b = s.boxed(0, 0.0, 0.0, 50.0, 50.0, 0x000000ff);
    s.style(b, |d| {
        d.position = Position::Absolute;
        d.size = [Dim::Px(50.0), Dim::Px(50.0)];
        d.background = 0x000000ff;
        d.opacity = 0.5;
    });
    s.render();
    let [r, g, bl, a] = s.pixel(25, 25);
    assert!((126..=129).contains(&r) && (126..=129).contains(&g) && (126..=129).contains(&bl) && a == 255, "{r} {g} {bl} {a}");
}

fn text_node(s: &mut Scene, text: &str, size: f32, width: Option<f32>) -> u32 {
    s.style(0, |d| {
        d.direction = Direction::Column;
        d.align_items = Some(Align::Start);
    });
    let n = s.tree.add(0, Kind::Text).unwrap();
    s.tree.set_text(n, text).unwrap();
    s.style(n, |d| {
        d.font_size = size;
        if let Some(w) = width {
            d.size = [Dim::Px(w), Dim::Auto];
        }
    });
    n
}

fn dark_pixels(s: &Scene, area: Rect) -> usize {
    let mut count = 0;
    for y in area.y as u32..area.bottom() as u32 {
        for x in area.x as u32..area.right() as u32 {
            if s.pixel(x, y)[0] < 128 {
                count += 1;
            }
        }
    }
    count
}

#[test]
fn text_is_sized_by_its_font_and_painted_inside_its_rectangle() {
    let mut s = Scene::new(300, 100);
    let n = text_node(&mut s, "Hello", 20.0, None);
    s.render();
    let r = s.tree.rect(n);
    assert!(r.w > 30.0 && r.w < 80.0, "{r:?}");
    assert_eq!(r.h, 24.0);
    assert!(dark_pixels(&s, r) > 40, "glyphs paint dark pixels");
    assert_eq!(dark_pixels(&s, Rect::new(r.right() + 2.0, 0.0, 300.0 - r.right() - 2.0, 100.0)), 0, "nothing paints beside the text");
    assert_eq!(dark_pixels(&s, Rect::new(0.0, r.bottom() + 2.0, 300.0, 100.0 - r.bottom() - 2.0)), 0, "nothing paints below the text");
}

#[test]
fn a_bigger_font_makes_a_bigger_node() {
    let mut s = Scene::new(300, 100);
    let small = text_node(&mut s, "Hello", 12.0, None);
    s.render();
    let small_rect = s.tree.rect(small);
    s.style(small, |d| d.font_size = 24.0);
    s.render();
    let big_rect = s.tree.rect(small);
    assert!(big_rect.w > small_rect.w * 1.8 && big_rect.h > small_rect.h * 1.8, "{small_rect:?} {big_rect:?}");
}

#[test]
fn long_text_wraps_to_a_fixed_width() {
    let mut s = Scene::new(300, 200);
    let n = text_node(&mut s, "one two three four five six seven eight", 16.0, Some(100.0));
    s.render();
    let r = s.tree.rect(n);
    assert_eq!(r.w, 100.0);
    assert!(r.h >= 3.0 * 19.0, "{r:?}");
    assert!(dark_pixels(&s, r) > 100);
}

#[test]
fn text_color_and_alignment_apply() {
    let mut s = Scene::new(200, 60);
    let n = text_node(&mut s, "ab", 20.0, Some(200.0));
    s.style(n, |d| {
        d.font_size = 20.0;
        d.size = [Dim::Px(200.0), Dim::Auto];
        d.color = RED;
        d.text_align = gui::TextAlign::Right;
    });
    s.render();
    let r = s.tree.rect(n);
    let left = dark_pixels(&s, Rect::new(0.0, r.y, 100.0, r.h));
    let mut reddish = 0;
    for y in 0..60 {
        for x in 0..200 {
            let [pr, pg, _, _] = s.pixel(x, y);
            if pr > 200 && pg < 100 {
                reddish += 1;
            }
        }
    }
    assert!(reddish > 20, "the ink is red");
    assert_eq!(left, 0, "right-aligned text leaves the left half empty");
}

#[test]
fn a_button_centers_its_text_vertically() {
    let mut s = Scene::new(200, 100);
    s.style(0, |d| {
        d.align_items = Some(Align::Start);
    });
    let b = s.tree.add(0, Kind::Button).unwrap();
    s.tree.set_text(b, "OK").unwrap();
    s.style(b, |d| {
        d.size = [Dim::Px(100.0), Dim::Px(60.0)];
        d.background = 0xdddd_ddff;
    });
    s.render();
    let r = s.tree.rect(b);
    let mut first = None;
    let mut last = 0;
    for y in r.y as u32..r.bottom() as u32 {
        if (r.x as u32..r.right() as u32).any(|x| s.pixel(x, y)[0] < 100) {
            first.get_or_insert(y);
            last = y;
        }
    }
    let (top_gap, bottom_gap) = (first.unwrap() as f32 - r.y, r.bottom() - last as f32);
    assert!((top_gap - bottom_gap).abs() < 8.0, "{top_gap} {bottom_gap}");
}

#[test]
fn painting_only_the_damage_matches_a_full_repaint() {
    let mut s = Scene::new(200, 100);
    let a = s.boxed(0, 10.0, 10.0, 50.0, 50.0, RED);
    s.boxed(0, 100.0, 10.0, 50.0, 50.0, GREEN);
    let t = text_node(&mut s, "text", 16.0, None);
    s.render();
    s.style(a, |d| {
        d.position = Position::Absolute;
        d.inset = [Dim::Px(10.0), Dim::Auto, Dim::Auto, Dim::Px(10.0)];
        d.size = [Dim::Px(50.0), Dim::Px(50.0)];
        d.background = BLUE;
    });
    s.tree.set_text(t, "changed").unwrap();
    let damage = s.render();
    assert!(!damage.is_empty() && damage.iter().all(|r| r.w < 200.0 || r.h < 100.0), "{damage:?}");
    assert_eq!(s.pixel(20, 20), [0, 0, 255, 255]);
    assert_eq!(s.pixel(120, 20), [0, 255, 0, 255]);
    let incremental = s.pixmap.data().to_vec();
    assert_eq!(incremental, s.full_repaint());
}

#[test]
fn scroll_children_are_clipped_and_a_thumb_shows() {
    let mut s = Scene::new(200, 200);
    let view = s.tree.add(0, Kind::Scroll).unwrap();
    s.style(view, |d| {
        d.direction = Direction::Column;
        d.size = [Dim::Px(100.0), Dim::Px(100.0)];
        d.background = 0xeeeeeeff;
    });
    for color in [RED, GREEN, BLUE] {
        let row = s.tree.add(view, Kind::Box).unwrap();
        s.style(row, |d| {
            d.size = [Dim::Px(90.0), Dim::Px(60.0)];
            d.shrink = 0.0;
            d.background = color;
        });
    }
    s.render();
    assert_eq!(s.pixel(10, 10), [255, 0, 0, 255]);
    assert_eq!(s.pixel(10, 70), [0, 255, 0, 255]);
    assert_eq!(s.pixel(10, 110), WHITE, "the third row is cut off at the viewport");
    let thumb = s.pixel(94, 20);
    assert!(thumb[0] < 240 && thumb[3] == 255, "a scrollbar thumb at the right edge: {thumb:?}");

    s.tree.scroll_by(view, 0.0, 80.0);
    s.render();
    assert_eq!(s.pixel(10, 10), [0, 255, 0, 255]);
    assert_eq!(s.pixel(10, 100), WHITE, "nothing leaks below the viewport");
    assert_eq!(s.pixel(10, 99), [0, 0, 255, 255]);
}

#[test]
fn an_image_is_drawn_scaled_into_its_rectangle() {
    let mut source = Pixmap::new(2, 2).unwrap();
    source.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));
    let png = source.encode_png().unwrap();

    let mut s = Scene::new(100, 100);
    let img = s.tree.add(0, Kind::Image).unwrap();
    s.painter.load_image(&mut s.tree, img, &png).unwrap();
    s.style(img, |d| d.size = [Dim::Px(40.0), Dim::Px(40.0)]);
    s.render();
    assert_eq!(s.tree.rect(img), Rect::new(0.0, 0.0, 40.0, 40.0));
    assert_eq!(s.pixel(20, 20), [255, 0, 0, 255]);
    assert_eq!(s.pixel(60, 20), WHITE);

    assert!(s.painter.load_image(&mut s.tree, img, b"not a png").is_err());
}

#[test]
fn an_image_node_takes_the_pixel_size_of_its_picture() {
    let source = Pixmap::new(30, 20).unwrap();
    let mut s = Scene::new(100, 100);
    s.style(0, |d| d.align_items = Some(Align::Start));
    let img = s.tree.add(0, Kind::Image).unwrap();
    s.painter.load_image(&mut s.tree, img, &source.encode_png().unwrap()).unwrap();
    s.render();
    assert_eq!(s.tree.rect(img), Rect::new(0.0, 0.0, 30.0, 20.0));
}

#[test]
fn the_window_clears_to_its_background_colour() {
    let mut s = Scene::new(20, 20);
    s.painter.background = 0x336699ff;
    s.render();
    assert_eq!(s.pixel(5, 5), [0x33, 0x66, 0x99, 255]);
}
