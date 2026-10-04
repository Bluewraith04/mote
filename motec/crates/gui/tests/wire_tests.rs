//! The style list that crosses into Mote and back.

use gui::wire::*;
use gui::{Align, Dim, Direction, Display, Input, Len, Overflow, Place, Position, StyleDef, TextAlign, Track, UiEvent};

fn busy() -> StyleDef {
    StyleDef {
        display: Display::Grid,
        position: Position::Absolute,
        direction: Direction::ColumnReverse,
        wrap: true,
        align_items: Some(Align::Center),
        align_self: Some(Align::End),
        align_content: Some(Align::SpaceBetween),
        justify_content: Some(Align::SpaceEvenly),
        gap: [Len::Px(4.0), Len::Pct(10.0)],
        grow: 2.0,
        shrink: 0.0,
        basis: Dim::Px(30.0),
        size: [Dim::Pct(50.0), Dim::Px(20.0)],
        min_size: [Dim::Px(1.0), Dim::Auto],
        max_size: [Dim::Auto, Dim::Pct(90.0)],
        margin: [Dim::Px(1.0), Dim::Px(2.0), Dim::Px(3.0), Dim::Auto],
        padding: [Len::Px(5.0), Len::Pct(6.0), Len::Px(7.0), Len::Px(8.0)],
        border: [1.0, 2.0, 3.0, 4.0],
        inset: [Dim::Px(9.0), Dim::Auto, Dim::Auto, Dim::Pct(5.0)],
        overflow: [Overflow::Hidden, Overflow::Scroll],
        columns: vec![Track::Px(100.0), Track::Fr(1.0), Track::Auto, Track::MinContent, Track::MaxContent],
        rows: vec![Track::Fr(2.0)],
        grid_column: Place { start: 2, span: 3 },
        grid_row: Place { start: -1, span: 1 },
        background: 0x11223344,
        border_color: 0xffeeddcc,
        radius: 6.5,
        opacity: 0.25,
        color: 0xff0000ff,
        font_size: 13.0,
        font_weight: 700,
        line_height: 18.0,
        text_align: TextAlign::Right,
        hover: 3,
        pressed: 4,
    }
}

#[test]
fn the_default_style_survives_the_round_trip() {
    let d = StyleDef::default();
    assert_eq!(StyleDef::from_wire(&d.to_wire()).unwrap(), d);
    assert_eq!(d.to_wire().len(), FIXED_LEN + 2, "no tracks: two counts");
}

#[test]
fn a_style_with_every_field_set_survives_the_round_trip() {
    let s = busy();
    assert_eq!(StyleDef::from_wire(&s.to_wire()).unwrap(), s);
}

#[test]
fn the_named_slots_hold_the_fields_they_name() {
    let w = busy().to_wire();
    assert_eq!(w[DISPLAY_AT], 1.0);
    assert_eq!(w[POSITION_AT], 1.0);
    assert_eq!(w[DIRECTION_AT], 3.0);
    assert_eq!(w[WRAP_AT], 1.0);
    assert_eq!([w[ALIGN_ITEMS_AT], w[ALIGN_SELF_AT], w[ALIGN_CONTENT_AT], w[JUSTIFY_AT]], [2.0, 1.0, 4.0, 6.0]);
    assert_eq!(w[GAP_COLUMN_AT..GAP_COLUMN_AT + 2], [0.0, 4.0]);
    assert_eq!(w[GAP_ROW_AT..GAP_ROW_AT + 2], [1.0, 10.0]);
    assert_eq!([w[GROW_AT], w[SHRINK_AT]], [2.0, 0.0]);
    assert_eq!(w[BASIS_AT..BASIS_AT + 2], [1.0, 30.0]);
    assert_eq!(w[WIDTH_AT..WIDTH_AT + 2], [2.0, 50.0]);
    assert_eq!(w[HEIGHT_AT..HEIGHT_AT + 2], [1.0, 20.0]);
    assert_eq!(w[MIN_WIDTH_AT..MIN_WIDTH_AT + 2], [1.0, 1.0]);
    assert_eq!(w[MIN_HEIGHT_AT..MIN_HEIGHT_AT + 2], [0.0, 0.0]);
    assert_eq!(w[MAX_WIDTH_AT..MAX_WIDTH_AT + 2], [0.0, 0.0]);
    assert_eq!(w[MAX_HEIGHT_AT..MAX_HEIGHT_AT + 2], [2.0, 90.0]);
    assert_eq!(w[MARGIN_AT..MARGIN_AT + 8], [1.0, 1.0, 1.0, 2.0, 1.0, 3.0, 0.0, 0.0]);
    assert_eq!(w[PADDING_AT..PADDING_AT + 8], [0.0, 5.0, 1.0, 6.0, 0.0, 7.0, 0.0, 8.0]);
    assert_eq!(w[BORDER_AT..BORDER_AT + 4], [1.0, 2.0, 3.0, 4.0]);
    assert_eq!(w[INSET_AT..INSET_AT + 8], [1.0, 9.0, 0.0, 0.0, 0.0, 0.0, 2.0, 5.0]);
    assert_eq!(w[OVERFLOW_AT..OVERFLOW_AT + 2], [1.0, 2.0]);
    assert_eq!(w[GRID_COLUMN_AT..GRID_COLUMN_AT + 2], [2.0, 3.0]);
    assert_eq!(w[GRID_ROW_AT..GRID_ROW_AT + 2], [-1.0, 1.0]);
    assert_eq!(w[BACKGROUND_AT], 0x11223344u32 as f64);
    assert_eq!(w[BORDER_COLOR_AT], 0xffeeddccu32 as f64);
    assert_eq!([w[RADIUS_AT], w[OPACITY_AT]], [6.5, 0.25]);
    assert_eq!(w[COLOR_AT], 0xff0000ffu32 as f64);
    assert_eq!([w[FONT_SIZE_AT], w[FONT_WEIGHT_AT], w[LINE_HEIGHT_AT]], [13.0, 700.0, 18.0]);
    assert_eq!(w[TEXT_ALIGN_AT], 2.0);
    assert_eq!([w[HOVER_AT], w[PRESSED_AT]], [3.0, 4.0]);
    assert_eq!(w[FIXED_LEN], 5.0, "five column tracks follow");
    assert_eq!(w[FIXED_LEN + 1..FIXED_LEN + 3], [0.0, 100.0]);
}

#[test]
fn a_bad_list_names_the_first_bad_value() {
    let good = StyleDef::default().to_wire();
    let mut bad = good.clone();
    bad[DISPLAY_AT] = 9.0;
    assert!(StyleDef::from_wire(&bad).unwrap_err().contains("display"));
    let mut bad = good.clone();
    bad[WIDTH_AT] = 7.0;
    assert!(StyleDef::from_wire(&bad).unwrap_err().contains("width"));
    let mut bad = good.clone();
    bad[OPACITY_AT] = f64::NAN;
    assert!(StyleDef::from_wire(&bad).unwrap_err().contains("opacity"));
    let mut bad = good.clone();
    bad[BACKGROUND_AT] = -1.0;
    assert!(StyleDef::from_wire(&bad).unwrap_err().contains("background"));
    assert!(StyleDef::from_wire(&good[..10]).unwrap_err().contains("too short"));
    let mut bad = good;
    let n = bad.len();
    bad[n - 2] = 3.0;
    assert!(StyleDef::from_wire(&bad).is_err(), "a track count with no tracks after it");
}

#[test]
fn an_event_is_five_numbers() {
    assert_eq!(UiEvent::Click(7).to_wire(), [0.0, 7.0, 0.0, 0.0, 0.0]);
    assert_eq!(UiEvent::PointerMove(3, 1.5, 2.5).to_wire(), [3.0, 3.0, 1.5, 2.5, 0.0]);
    assert_eq!(UiEvent::Wheel(2, 0.0, -4.0).to_wire(), [6.0, 2.0, 0.0, -4.0, 0.0]);
    assert_eq!(UiEvent::Key(13, 2, true).to_wire(), [7.0, 0.0, 13.0, 2.0, 1.0]);
    assert_eq!(UiEvent::Resize(640, 480).to_wire(), [10.0, 0.0, 640.0, 480.0, 0.0]);
    assert_eq!(UiEvent::Close.to_wire()[0], 11.0);
    assert_eq!(UiEvent::User(-5).to_wire(), [12.0, 0.0, -5.0, 0.0, 0.0]);
}

#[test]
fn an_input_comes_from_five_numbers() {
    assert_eq!(Input::from_wire(0, 4.0, 5.0, 0.0, 0.0), Ok(Input::PointerMove { x: 4.0, y: 5.0 }));
    assert_eq!(Input::from_wire(4, 1.0, 2.0, 3.0, 4.0), Ok(Input::Wheel { x: 1.0, y: 2.0, dx: 3.0, dy: 4.0 }));
    assert_eq!(Input::from_wire(5, 13.0, 1.0, 1.0, 0.0), Ok(Input::Key { code: 13, mods: 1, down: true }));
    assert_eq!(Input::from_wire(6, 80.0, 60.0, 0.0, 0.0), Ok(Input::Resize { width: 80, height: 60 }));
    assert_eq!(Input::from_wire(7, 0.0, 0.0, 0.0, 0.0), Ok(Input::Close));
    assert!(Input::from_wire(9, 0.0, 0.0, 0.0, 0.0).is_err());
    assert!(Input::from_wire(5, -1.0, 0.0, 0.0, 0.0).is_err());
}
