//! Layout, hit testing, scrolling and damage on the node tree, with no window.

use gui::{Align, Dim, Direction, Display, Kind, Len, Measure, MeasureNode, NO_STYLE, Overflow, Place, Position, Rect, StyleDef, Track, Tree};

/// Every character is 8 by 16 pixels; text wraps at spaces to the width it is given.
struct Mono;

impl Measure for Mono {
    fn measure(&mut self, node: &MeasureNode<'_>, max_width: Option<f32>) -> [f32; 2] {
        let words: Vec<&str> = node.text.split(' ').collect();
        let limit = max_width.unwrap_or(f32::INFINITY);
        let (mut lines, mut line_w, mut widest) = (1.0f32, 0.0f32, 0.0f32);
        for (i, word) in words.iter().enumerate() {
            let w = word.chars().count() as f32 * 8.0;
            let with_space = if i == 0 { w } else { w + 8.0 };
            if line_w > 0.0 && line_w + with_space > limit {
                lines += 1.0;
                line_w = w;
            } else {
                line_w += with_space;
            }
            widest = widest.max(line_w);
        }
        [widest, lines * 16.0]
    }
}

fn styled(tree: &mut Tree, node: u32, f: impl FnOnce(&mut StyleDef)) {
    let mut def = StyleDef::default();
    f(&mut def);
    let id = tree.intern_style(def);
    tree.set_style(node, id).unwrap();
}

fn px(w: f32, h: f32) -> [Dim; 2] {
    [Dim::Px(w), Dim::Px(h)]
}

fn laid_out(tree: &mut Tree) {
    tree.layout(&mut Mono);
}

#[test]
fn a_row_of_fixed_boxes_sits_side_by_side() {
    let mut t = Tree::new(400.0, 300.0);
    let a = t.add(0, Kind::Box).unwrap();
    let b = t.add(0, Kind::Box).unwrap();
    styled(&mut t, a, |s| s.size = px(100.0, 50.0));
    styled(&mut t, b, |s| s.size = px(60.0, 80.0));
    laid_out(&mut t);
    assert_eq!(t.rect(0), Rect::new(0.0, 0.0, 400.0, 300.0));
    assert_eq!(t.rect(a), Rect::new(0.0, 0.0, 100.0, 50.0));
    assert_eq!(t.rect(b), Rect::new(100.0, 0.0, 60.0, 80.0));
}

#[test]
fn a_column_with_padding_and_gap_places_children_inside_the_padding() {
    let mut t = Tree::new(200.0, 200.0);
    styled(&mut t, 0, |s| {
        s.direction = Direction::Column;
        s.padding = [Len::Px(10.0), Len::Px(10.0), Len::Px(10.0), Len::Px(20.0)];
        s.gap = [Len::Px(0.0), Len::Px(5.0)];
    });
    let a = t.add(0, Kind::Box).unwrap();
    let b = t.add(0, Kind::Box).unwrap();
    styled(&mut t, a, |s| s.size = px(50.0, 20.0));
    styled(&mut t, b, |s| s.size = px(50.0, 30.0));
    laid_out(&mut t);
    assert_eq!(t.rect(a), Rect::new(20.0, 10.0, 50.0, 20.0));
    assert_eq!(t.rect(b), Rect::new(20.0, 35.0, 50.0, 30.0));
}

#[test]
fn flex_grow_shares_the_leftover_space() {
    let mut t = Tree::new(300.0, 100.0);
    let a = t.add(0, Kind::Box).unwrap();
    let b = t.add(0, Kind::Box).unwrap();
    let c = t.add(0, Kind::Box).unwrap();
    styled(&mut t, a, |s| s.size = px(100.0, 100.0));
    styled(&mut t, b, |s| s.grow = 1.0);
    styled(&mut t, c, |s| s.size = px(50.0, 100.0));
    laid_out(&mut t);
    assert_eq!(t.rect(b), Rect::new(100.0, 0.0, 150.0, 100.0));
    assert_eq!(t.rect(c).x, 250.0);
}

#[test]
fn text_sizes_its_node_and_a_button_adds_its_padding() {
    let mut t = Tree::new(400.0, 300.0);
    styled(&mut t, 0, |s| {
        s.direction = Direction::Column;
        s.align_items = Some(Align::Start);
    });
    let label = t.add(0, Kind::Text).unwrap();
    t.set_text(label, "hello").unwrap();
    let button = t.add(0, Kind::Button).unwrap();
    t.set_text(button, "ok").unwrap();
    styled(&mut t, button, |s| s.padding = [Len::Px(4.0), Len::Px(10.0), Len::Px(4.0), Len::Px(10.0)]);
    laid_out(&mut t);
    assert_eq!(t.rect(label).h, 16.0);
    assert_eq!(t.rect(button).w, 16.0 + 20.0);
    assert_eq!(t.rect(button).h, 16.0 + 8.0);
}

#[test]
fn text_wraps_to_the_width_it_is_given() {
    let mut t = Tree::new(80.0, 300.0);
    styled(&mut t, 0, |s| s.direction = Direction::Column);
    let p = t.add(0, Kind::Text).unwrap();
    t.set_text(p, "one two three four").unwrap();
    laid_out(&mut t);
    assert_eq!(t.rect(p).w, 80.0);
    assert!(t.rect(p).h >= 32.0, "{:?}", t.rect(p));
}

#[test]
fn move_before_reorders_siblings_and_their_layout() {
    let mut t = Tree::new(400.0, 300.0);
    let a = t.add(0, Kind::Box).unwrap();
    let b = t.add(0, Kind::Box).unwrap();
    let c = t.add(0, Kind::Box).unwrap();
    for (n, w) in [(a, 10.0), (b, 20.0), (c, 30.0)] {
        styled(&mut t, n, |s| s.size = px(w, 50.0));
    }
    laid_out(&mut t);
    t.move_before(c, Some(a)).unwrap();
    assert_eq!(t.children(0), vec![c, a, b]);
    laid_out(&mut t);
    assert_eq!(t.rect(c), Rect::new(0.0, 0.0, 30.0, 50.0));
    assert_eq!(t.rect(a), Rect::new(30.0, 0.0, 10.0, 50.0));
    t.move_before(c, None).unwrap();
    assert_eq!(t.children(0), vec![a, b, c]);
    laid_out(&mut t);
    assert_eq!(t.rect(c), Rect::new(30.0, 0.0, 30.0, 50.0));
}

#[test]
fn move_before_rejects_the_root_and_a_non_sibling() {
    let mut t = Tree::new(400.0, 300.0);
    let a = t.add(0, Kind::Box).unwrap();
    let inner = t.add(a, Kind::Box).unwrap();
    assert!(t.move_before(0, None).is_err());
    assert!(t.move_before(inner, Some(a)).is_err());
    assert!(t.move_before(a, Some(99)).is_err());
}

#[test]
fn a_grid_places_children_in_its_columns() {
    let mut t = Tree::new(200.0, 100.0);
    styled(&mut t, 0, |s| {
        s.display = Display::Grid;
        s.columns = vec![Track::Fr(1.0), Track::Fr(1.0)];
        s.rows = vec![Track::Px(40.0), Track::Px(40.0)];
    });
    let kids: Vec<u32> = (0..4).map(|_| t.add(0, Kind::Box).unwrap()).collect();
    styled(&mut t, kids[3], |s| s.grid_column = Place { start: 1, span: 2 });
    laid_out(&mut t);
    assert_eq!(t.rect(kids[0]), Rect::new(0.0, 0.0, 100.0, 40.0));
    assert_eq!(t.rect(kids[1]), Rect::new(100.0, 0.0, 100.0, 40.0));
    assert_eq!(t.rect(kids[2]), Rect::new(0.0, 40.0, 100.0, 40.0));
    assert_eq!(t.rect(kids[3]).w, 200.0);
}

#[test]
fn equal_styles_share_one_id() {
    let mut t = Tree::new(10.0, 10.0);
    let a = t.intern_style(StyleDef { radius: 4.0, ..StyleDef::default() });
    let b = t.intern_style(StyleDef { radius: 4.0, ..StyleDef::default() });
    let c = t.intern_style(StyleDef { radius: 5.0, ..StyleDef::default() });
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_eq!(t.style_count(), 2);
    assert!(t.set_style(0, 99).is_err());
    assert!(t.set_style(0, NO_STYLE).is_ok());
}

#[test]
fn hit_testing_finds_the_topmost_node() {
    let mut t = Tree::new(200.0, 200.0);
    let under = t.add(0, Kind::Box).unwrap();
    let over = t.add(0, Kind::Box).unwrap();
    styled(&mut t, under, |s| {
        s.position = Position::Absolute;
        s.size = px(100.0, 100.0);
    });
    styled(&mut t, over, |s| {
        s.position = Position::Absolute;
        s.inset = [Dim::Px(50.0), Dim::Auto, Dim::Auto, Dim::Px(50.0)];
        s.size = px(100.0, 100.0);
    });
    let inner = t.add(over, Kind::Box).unwrap();
    styled(&mut t, inner, |s| s.size = px(10.0, 10.0));
    laid_out(&mut t);
    assert_eq!(t.hit(10.0, 10.0), Some(under));
    assert_eq!(t.hit(55.0, 55.0), Some(inner));
    assert_eq!(t.hit(120.0, 120.0), Some(over));
    assert_eq!(t.hit(180.0, 180.0), Some(0));
    assert_eq!(t.hit(-1.0, 5.0), None);
    t.remove(over).unwrap();
    laid_out(&mut t);
    assert_eq!(t.hit(120.0, 120.0), Some(0));
}

fn scroller(t: &mut Tree) -> (u32, Vec<u32>) {
    let view = t.add(0, Kind::Scroll).unwrap();
    styled(t, view, |s| {
        s.direction = Direction::Column;
        s.size = px(100.0, 100.0);
    });
    let rows: Vec<u32> = (0..6).map(|_| t.add(view, Kind::Box).unwrap()).collect();
    for &r in &rows {
        styled(t, r, |s| {
            s.size = px(100.0, 50.0);
            s.shrink = 0.0;
        });
    }
    laid_out(t);
    (view, rows)
}

#[test]
fn a_scroll_node_shifts_and_clips_its_children() {
    let mut t = Tree::new(300.0, 300.0);
    let (view, rows) = scroller(&mut t);
    assert_eq!(t.content_size(view), [100.0, 300.0]);
    assert_eq!(t.rect(rows[2]), Rect::new(0.0, 100.0, 100.0, 50.0));
    assert!(t.visible(rows[2]).is_empty());
    assert!(t.scroll_by(view, 0.0, 120.0));
    assert_eq!(t.scroll_offset(view), [0.0, 120.0]);
    assert_eq!(t.rect(rows[2]), Rect::new(0.0, -20.0, 100.0, 50.0));
    assert_eq!(t.visible(rows[2]), Rect::new(0.0, 0.0, 100.0, 30.0));
    assert_eq!(t.hit(10.0, 10.0), Some(rows[2]));
}

#[test]
fn scrolling_stops_at_the_ends_and_reports_no_move() {
    let mut t = Tree::new(300.0, 300.0);
    let (view, _) = scroller(&mut t);
    assert!(t.scroll_by(view, 0.0, 10_000.0));
    assert_eq!(t.scroll_offset(view), [0.0, 200.0]);
    assert!(!t.scroll_by(view, 0.0, 10.0));
    assert!(t.scroll_by(view, 0.0, -10_000.0));
    assert_eq!(t.scroll_offset(view), [0.0, 0.0]);
    assert!(!t.scroll_by(view, 50.0, 0.0), "no horizontal overflow");
}

#[test]
fn the_wheel_scrolls_the_nearest_scrollable_ancestor_that_can_move() {
    let mut t = Tree::new(300.0, 300.0);
    let (view, rows) = scroller(&mut t);
    assert_eq!(t.scroll_at(10.0, 10.0, 0.0, 30.0), Some(view));
    assert_eq!(t.scroll_offset(view)[1], 30.0);
    assert_eq!(t.hit(10.0, 10.0), Some(rows[0]));
    assert_eq!(t.scroll_at(250.0, 250.0, 0.0, 30.0), None, "the root has nothing to scroll");
}

#[test]
fn a_hidden_overflow_box_clips_without_scrolling() {
    let mut t = Tree::new(200.0, 200.0);
    let boxed = t.add(0, Kind::Box).unwrap();
    styled(&mut t, boxed, |s| {
        s.size = px(50.0, 50.0);
        s.overflow = [Overflow::Hidden, Overflow::Hidden];
    });
    let big = t.add(boxed, Kind::Box).unwrap();
    styled(&mut t, big, |s| {
        s.size = px(200.0, 200.0);
        s.shrink = 0.0;
    });
    laid_out(&mut t);
    assert_eq!(t.visible(big), Rect::new(0.0, 0.0, 50.0, 50.0));
    assert!(!t.scroll_by(boxed, 0.0, 10.0));
}

#[test]
fn damage_covers_the_first_paint_changes_and_resizes() {
    let mut t = Tree::new(200.0, 100.0);
    let a = t.add(0, Kind::Box).unwrap();
    styled(&mut t, a, |s| s.size = px(40.0, 40.0));
    laid_out(&mut t);
    assert_eq!(t.take_damage(), vec![Rect::new(0.0, 0.0, 200.0, 100.0)]);
    assert!(t.take_damage().is_empty());

    styled(&mut t, a, |s| {
        s.size = px(40.0, 40.0);
        s.background = 0xff0000ff;
    });
    laid_out(&mut t);
    assert_eq!(t.take_damage(), vec![Rect::new(0.0, 0.0, 40.0, 40.0)]);

    styled(&mut t, a, |s| s.size = px(60.0, 40.0));
    laid_out(&mut t);
    assert_eq!(t.take_damage(), vec![Rect::new(0.0, 0.0, 60.0, 40.0)]);

    t.damage_node(a);
    laid_out(&mut t);
    assert_eq!(t.take_damage(), vec![Rect::new(0.0, 0.0, 60.0, 40.0)]);

    t.set_viewport(300.0, 100.0);
    laid_out(&mut t);
    assert_eq!(t.take_damage(), vec![Rect::new(0.0, 0.0, 300.0, 100.0)]);
    assert!(t.take_damage().is_empty());
}

#[test]
fn moving_a_node_damages_where_it_was_and_where_it_went() {
    let mut t = Tree::new(200.0, 100.0);
    let a = t.add(0, Kind::Box).unwrap();
    let b = t.add(0, Kind::Box).unwrap();
    styled(&mut t, a, |s| s.size = px(20.0, 20.0));
    styled(&mut t, b, |s| s.size = px(20.0, 20.0));
    laid_out(&mut t);
    t.take_damage();
    styled(&mut t, a, |s| s.size = px(60.0, 20.0));
    laid_out(&mut t);
    let damage = t.take_damage();
    assert_eq!(damage, vec![Rect::new(0.0, 0.0, 80.0, 20.0)]);
}

#[test]
fn removing_a_subtree_damages_it_and_frees_its_ids() {
    let mut t = Tree::new(200.0, 100.0);
    let a = t.add(0, Kind::Box).unwrap();
    styled(&mut t, a, |s| s.size = px(30.0, 30.0));
    let inner = t.add(a, Kind::Box).unwrap();
    laid_out(&mut t);
    t.take_damage();
    let before = t.live_nodes();
    t.remove(a).unwrap();
    assert_eq!(t.live_nodes(), before - 2);
    assert!(t.kind(a).is_none() && t.kind(inner).is_none());
    assert!(t.text(a).is_err());
    laid_out(&mut t);
    assert_eq!(t.take_damage(), vec![Rect::new(0.0, 0.0, 30.0, 30.0)]);
    let reused = t.add(0, Kind::Text).unwrap();
    assert!(reused == a || reused == inner);
    assert_eq!(t.kind(reused), Some(Kind::Text));
}

#[test]
fn the_root_cannot_be_removed_and_a_dead_node_is_an_error() {
    let mut t = Tree::new(10.0, 10.0);
    assert!(t.remove(0).is_err());
    assert!(t.add(7, Kind::Box).is_err());
    assert!(t.set_text(7, "x").is_err());
    assert!(t.remove(7).is_err());
}

#[test]
fn paint_order_runs_parents_first_and_siblings_in_order() {
    let mut t = Tree::new(100.0, 100.0);
    let a = t.add(0, Kind::Box).unwrap();
    let b = t.add(0, Kind::Box).unwrap();
    let a1 = t.add(a, Kind::Box).unwrap();
    laid_out(&mut t);
    assert_eq!(t.paint_order(), &[0, a, a1, b]);
    assert_eq!(t.children(0), vec![a, b]);
    assert_eq!(t.parent(a1), Some(a));
}
