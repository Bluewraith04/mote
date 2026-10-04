//! The `pane` package over a headless window, run as real programs.

mod common;

use std::process::Command;

fn run_text(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_pane_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    common::install_package(&dir, "pane");
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote"))
        .arg("run")
        .arg(dir.join("main.mote"))
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .output()
        .unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn run(source: &str, tag: &str) -> Vec<String> {
    let (ok, text) = run_text(source, tag);
    assert!(ok, "{text}");
    text.lines().map(String::from).collect()
}

/// A program: the shared prelude, `body` as `main`'s statements after the app mounts as `app` on `win` with `state`.
fn program(view: &str, body: &str) -> String {
    format!(
        r#"import std.sys.gui as gui
import pane

class State {{
    var count: Int
    var name: String
    var done: Bool
}}

{view}

/// The first node under the point; clicks and typing go through the window the way a person's would.
fn at(win: gui.Window, x: Float, y: Float) -> gui.Node {{
    return win.hit(x, y).unwrap().unwrap()
}}

fn click(win: gui.Window, x: Float, y: Float) {{
    win.input(gui.Input.PointerMove(x, y)).unwrap()
    win.input(gui.Input.PointerDown(x, y)).unwrap()
    win.input(gui.Input.PointerUp(x, y)).unwrap()
}}

/// Steps the app through every event queued so far, stopping at the marker `User(1)`.
fn settle(var app: pane.App<State>, win: gui.Window, events: Stream<gui.UiEvent>) {{
    win.post(1).unwrap()
    for e in events {{
        match e {{
            gui.UiEvent.User(n) => {{ return }}
            _ => {{ app.step(e).unwrap() }}
        }}
    }}
}}

fn main() {{
    let win = gui.headless(300, 300).unwrap()
    let state = State {{ count: 0, name: "", done: false }}
    var app = pane.mount(win, state, view).unwrap()
    let events = win.events()
{body}
}}
"#
    )
}

const COUNTER: &str = r#"fn view(s: State) -> pane.Element<State> {
    return pane.vstack([
        pane.h1("Counter"),
        pane.text("Count: ${s.count}"),
        pane.hstack([
            pane.button("-").on_click(|var s| { s.count -= 1 }),
            pane.button("+").on_click(|var s| { s.count += 1 }),
        ]),
        pane.input(s.name).on_change(|var s, t| { s.name = t }).on_submit(|var s| { s.done = true }),
        pane.text("Hello ${s.name}"),
    ])
}"#;

#[test]
fn clicking_a_button_runs_its_handler_and_redraws() {
    // `settle` takes the app by `var`, so it steps the app held here.
    let got = run(
        &program(
            COUNTER,
            r#"    click(win, 70.0, 100.0)
    click(win, 70.0, 100.0)
    click(win, 30.0, 100.0)
    settle(app, win, events)
    println(state.count)
    println(win.text(at(win, 20.0, 60.0)).unwrap())"#,
        ),
        "counter",
    );
    assert_eq!(got, ["1", "Count: 1"]);
}

#[test]
fn center_puts_a_button_in_the_middle_of_a_stack_that_fills_the_window() {
    let got = run(
        &program(
            r#"fn view(s: State) -> pane.Element<State> {
    return pane.vstack([
        pane.button("+").on_click(|var s| { s.count += 1 }),
    ]).grow().center()
}"#,
            r#"    click(win, 30.0, 150.0)
    settle(app, win, events)
    println(state.count)
    click(win, 150.0, 150.0)
    settle(app, win, events)
    println(state.count)"#,
        ),
        "center",
    );
    assert_eq!(got, ["0", "1"]);
}

#[test]
fn typing_in_an_input_updates_the_state_and_the_text_that_shows_it() {
    let got = run(
        &program(
            COUNTER,
            r#"    click(win, 100.0, 140.0)
    win.input(gui.Input.Text("ann")).unwrap()
    win.input(gui.Input.Key(gui.Key.Enter, gui.no_mods(), true)).unwrap()
    settle(app, win, events)
    println(state.name)
    println(state.done)
    println(win.text(at(win, 20.0, 180.0)).unwrap())
    println(win.text(at(win, 100.0, 140.0)).unwrap())"#,
        ),
        "typing",
    );
    assert_eq!(got, ["ann", "true", "Hello ann", "ann"]);
}

#[test]
fn a_hovered_button_takes_a_darker_color() {
    let got = run(
        &program(
            COUNTER,
            r#"    win.present().unwrap()
    println(win.pixel(80, 90).unwrap())
    win.input(gui.Input.PointerMove(80.0, 90.0)).unwrap()
    win.present().unwrap()
    println(win.pixel(80, 90).unwrap())"#,
        ),
        "hover",
    );
    assert_eq!(got, ["627305471", "559535103"]);
}

#[test]
fn children_that_come_and_go_are_added_and_removed() {
    let view = r#"fn view(s: State) -> pane.Element<State> {
    var rows: List<pane.Element<State>> = []
    var i = 0
    while i < s.count {
        rows.push(pane.text("row ${i}"))
        i = i + 1
    }
    return pane.vstack([
        pane.button("more").on_click(|var s| { s.count += 1 }),
        pane.button("fewer").on_click(|var s| { s.count -= 1 }),
        pane.vstack(rows),
    ])
}"#;
    let got = run(
        &program(
            view,
            r#"    click(win, 40.0, 40.0)
    click(win, 40.0, 40.0)
    click(win, 40.0, 40.0)
    click(win, 40.0, 80.0)
    click(win, 40.0, 80.0)
    settle(app, win, events)
    println(state.count)
    println(win.text(at(win, 20.0, 110.0)).unwrap())
    println(win.rect(at(win, 20.0, 134.0)).unwrap().get(3) > 100.0)"#,
        ),
        "children",
    );
    assert_eq!(got, ["1", "row 0", "true"]);
}

#[test]
fn a_changed_kind_is_rebuilt_in_its_place() {
    let view = r#"fn view(s: State) -> pane.Element<State> {
    var body: pane.Element<State> = pane.text("plain")
    if s.count > 0 { body = pane.button("flip") }
    return pane.vstack([
        pane.button("go").on_click(|var s| { s.count += 1 }),
        body,
        pane.text("after"),
    ])
}"#;
    let got = run(
        &program(
            view,
            r#"    click(win, 40.0, 40.0)
    settle(app, win, events)
    println(win.text(at(win, 20.0, 80.0)).unwrap())
    println(win.text(at(win, 20.0, 120.0)).unwrap())"#,
        ),
        "kind",
    );
    assert_eq!(got, ["flip", "after"]);
}

#[test]
fn a_keyed_element_keeps_its_node_when_the_siblings_reorder() {
    let view = r#"fn view(s: State) -> pane.Element<State> {
    let a: pane.Element<State> = pane.input(s.name).key("a").on_change(|var s, t| { s.name = t })
    let b: pane.Element<State> = pane.input("fixed").key("b")
    var rows: List<pane.Element<State>> = [a, b]
    if s.count % 2 == 1 { rows = [b, a] }
    return pane.vstack([pane.button("flip").on_click(|var s| { s.count += 1 }), pane.vstack(rows)])
}"#;
    let got = run(
        &program(
            view,
            r#"    click(win, 100.0, 70.0)
    win.input(gui.Input.Text("ann")).unwrap()
    settle(app, win, events)
    let before = at(win, 100.0, 70.0).id
    click(win, 30.0, 30.0)
    settle(app, win, events)
    println(win.text(at(win, 100.0, 70.0)).unwrap())
    println(win.text(at(win, 100.0, 116.0)).unwrap())
    println(at(win, 100.0, 116.0).id == before)
    click(win, 30.0, 30.0)
    settle(app, win, events)
    println(at(win, 100.0, 70.0).id == before)"#,
        ),
        "keys",
    );
    assert_eq!(got, ["fixed", "ann", "true", "true"]);
}

#[test]
fn unkeyed_siblings_match_by_position_and_a_changed_kind_leaves_the_rest() {
    let view = r#"fn view(s: State) -> pane.Element<State> {
    var body: pane.Element<State> = pane.text("plain")
    if s.count > 0 { body = pane.button("flip") }
    return pane.vstack([pane.button("go").on_click(|var s| { s.count += 1 }), body, pane.text("after")])
}"#;
    let got = run(
        &program(
            view,
            r#"    let after = at(win, 20.0, 95.0)
    click(win, 40.0, 40.0)
    settle(app, win, events)
    println(at(win, 20.0, 110.0).id == after.id)"#,
        ),
        "positional",
    );
    assert_eq!(got, ["true"]);
}

#[test]
fn a_theme_set_once_restyles_every_element() {
    let got = run(
        r#"import std.sys.gui as gui
import pane
import pane.theme

class State {
    var count: Int
}

fn view(s: State) -> pane.Element<State> {
    return pane.vstack([pane.button("go").on_click(|var s| { s.count += 1 })])
}

fn main() {
    let a = gui.headless(200, 100).unwrap()
    pane.mount(a, State { count: 0 }, view).unwrap()
    println(a.pixel(190, 90).unwrap() == 0xf4f5f7ff)
    println(a.pixel(30, 30).unwrap() == 0x2563ebff)
    let b = gui.headless(200, 100).unwrap()
    pane.mount_themed(b, State { count: 0 }, view, theme.dark()).unwrap()
    println(b.pixel(190, 90).unwrap() == 0x111827ff)
    println(b.pixel(30, 30).unwrap() == 0x3b82f6ff)
    let c = gui.headless(200, 100).unwrap()
    pane.mount_themed(c, State { count: 0 }, view, theme.contrast().accent(0xff0000ff).page(0x00ff00ff)).unwrap()
    println(c.pixel(190, 90).unwrap() == 0x00ff00ff)
    println(c.pixel(30, 30).unwrap() == 0xff0000ff)
}
"#,
        "theme",
    );
    assert_eq!(got, ["true", "true", "true", "true", "true", "true"]);
}

/// `program` plus the component modules, and `find` that answers the x of the first button or text showing `want` on the row at `y`.
fn components(view: &str, body: &str) -> String {
    let source = program(view, body);
    let imports = "import pane\nimport pane.theme\nimport { nav } from pane.nav\nimport { tabs } from pane.tabs\nimport { pages } from pane.pages\nimport { checkbox } from pane.checkbox\nimport { select } from pane.select\nimport { image } from pane.image\n";
    let find = r#"
fn find(win: gui.Window, y: Float, want: String) -> Float {
    var x = 0.0
    while x < 300.0 {
        match win.hit(x, y).unwrap() {
            Some(n) => { if win.text(n).unwrap() == want { return x } }
            None => { }
        }
        x = x + 2.0
    }
    return -1.0
}
"#;
    source.replacen("import pane\n", imports, 1).replacen("fn main()", &format!("{find}\nfn main()"), 1)
}

#[test]
fn a_nav_picks_one_page_and_the_view_shows_that_page() {
    let view = r#"fn view(s: State) -> pane.Element<State> {
    var page: pane.Element<State> = pane.text("welcome")
    if s.count == 1 { page = pane.vstack([pane.h2("Performance"), pane.text("busy")]) }
    return pane.hstack([
        nav(["Processes", "Performance", "History"], s.count, |var s, i| { s.count = i }).style(|st| st.width(gui.Size.Px(150.0))),
        page,
    ]).style(|st| st.align_items(gui.Align.Start)).grow()
}"#;
    let got = run(
        &components(
            view,
            r#"    println(win.pixel(20, 20).unwrap() == 0x2563ebff)
    click(win, 40.0, 70.0)
    settle(app, win, events)
    println(state.count)
    win.input(gui.Input.PointerMove(290.0, 290.0)).unwrap()
    win.present().unwrap()
    println(win.pixel(20, 20).unwrap() == 0x2563ebff)
    println(win.pixel(20, 70).unwrap() == 0x2563ebff)
    println(find(win, 60.0, "busy") > 0.0)
    println(find(win, 26.0, "welcome") < 0.0)"#,
        ),
        "nav",
    );
    assert_eq!(got, ["true", "1", "false", "true", "true", "true"]);
}

#[test]
fn tabs_and_pages_report_the_one_picked() {
    let view = r#"fn view(s: State) -> pane.Element<State> {
    return pane.vstack([
        tabs(["One", "Two", "Three"], s.count, |var s, i| { s.count = i }),
        pages(10, s.count, |var s, i| { s.count = i }),
    ])
}"#;
    let got = run(
        &components(
            view,
            r#"    click(win, find(win, 26.0, "Three") + 2.0, 26.0)
    settle(app, win, events)
    println(state.count)
    click(win, find(win, 70.0, ">") + 2.0, 70.0)
    settle(app, win, events)
    println(state.count)
    click(win, find(win, 70.0, "2") + 2.0, 70.0)
    settle(app, win, events)
    println(state.count)
    click(win, find(win, 70.0, "<") + 2.0, 70.0)
    settle(app, win, events)
    println(state.count)
    println(find(win, 70.0, "10") < 0.0)"#,
        ),
        "tabs_pages",
    );
    assert_eq!(got, ["2", "3", "1", "0", "true"]);
}

#[test]
fn a_checkbox_flips_from_its_box_or_its_label() {
    let view = r#"fn view(s: State) -> pane.Element<State> {
    return pane.vstack([checkbox("Enabled", s.done, |var s, on| { s.done = on })])
}"#;
    let got = run(
        &components(
            view,
            r#"    println(win.pixel(26, 26).unwrap() == 0x2563ebff)
    click(win, 60.0, 26.0)
    settle(app, win, events)
    println(state.done)
    println(win.pixel(26, 26).unwrap() == 0x2563ebff)
    click(win, 26.0, 26.0)
    settle(app, win, events)
    println(state.done)"#,
        ),
        "checkbox",
    );
    assert_eq!(got, ["false", "true", "true", "false"]);
}

#[test]
fn a_select_opens_in_place_and_closes_on_a_pick() {
    let view = r#"fn view(s: State) -> pane.Element<State> {
    return pane.vstack([
        select(["red", "green", "blue"], s.count, s.done, |var s, i, open| { s.count = i
 s.done = open }),
        pane.text("below"),
    ])
}"#;
    let got = run(
        &components(
            view,
            r#"    click(win, 30.0, 26.0)
    settle(app, win, events)
    println(state.done)
    click(win, 30.0, 150.0)
    settle(app, win, events)
    println(state.count)
    println(state.done)"#,
        ),
        "select",
    );
    assert_eq!(got, ["true", "2", "false"]);
}

#[test]
fn an_image_shows_its_png_scaled_to_its_box() {
    let got = run(
        r#"import std.sys.gui as gui
import pane
import { image } from pane.image

class State {
    var count: Int
}

fn view(s: State, png: Bytes) -> pane.Element<State> {
    return pane.vstack([image(png).width(20.0).height(20.0)])
}

fn main() {
    let src = gui.headless(4, 4).unwrap()
    src.set_style(src.root(), src.style(gui.style().width(gui.Size.Pct(100.0)).height(gui.Size.Pct(100.0)).background(0xff0000ff)).unwrap()).unwrap()
    src.present().unwrap()
    let png = src.png().unwrap()
    let win = gui.headless(100, 100).unwrap()
    pane.mount(win, State { count: 0 }, |s| { return view(s, png) }).unwrap()
    println(win.pixel(26, 26).unwrap() == 0xff0000ff)
    println(win.pixel(60, 60).unwrap() == 0xf4f5f7ff)
}
"#,
        "image",
    );
    assert_eq!(got, ["true", "true"]);
}

#[test]
fn a_shared_cell_updated_by_another_task_redraws_and_takes_the_handlers() {
    let got = run(
        r#"import std.sys.gui as gui
import pane
import { mount_shared } from pane.shared

class State {
    var count: Int
}

fn view(s: State) -> pane.Element<State> {
    return pane.vstack([
        pane.text("count ${s.count}"),
        pane.button("+").on_click(|var s| { s.count += 1 }),
    ])
}

fn main() {
    let win = gui.headless(300, 300).unwrap()
    var cell = Shared(State { count: 0 })
    var app = mount_shared(win, cell, view).unwrap()
    let events = win.events()
    scope {
        spawn {
            var other = cell
            other.update(|v| { v.count = 40 })
            win.post(7).unwrap()
        }
    }
    for e in events {
        match e {
            gui.UiEvent.User(n) => {
                app.step(e).unwrap()
                break
            }
            _ => { app.step(e).unwrap() }
        }
    }
    println(win.text(win.hit(20.0, 26.0).unwrap().unwrap()).unwrap())
    win.input(gui.Input.PointerMove(20.0, 60.0)).unwrap()
    win.input(gui.Input.PointerDown(20.0, 60.0)).unwrap()
    win.input(gui.Input.PointerUp(20.0, 60.0)).unwrap()
    win.post(8).unwrap()
    for e in events {
        match e {
            gui.UiEvent.User(n) => { break }
            _ => { app.step(e).unwrap() }
        }
    }
    println(cell.get().count)
    println(win.text(win.hit(20.0, 26.0).unwrap().unwrap()).unwrap())
}
"#,
        "shared",
    );
    assert_eq!(got, ["count 40", "41", "count 41"]);
}

#[test]
fn a_title_bar_drawn_by_pane_has_working_caption_buttons_and_leaves_the_content_below_it() {
    let got = run(
        r#"import std.sys.gui as gui
import pane
import pane.theme
import pane.chrome

class State {
    var count: Int
}

fn view(s: State) -> pane.Element<State> {
    return pane.vstack([pane.button("+").on_click(|var s| { s.count += 1 })])
}

fn click(win: gui.Window, x: Float, y: Float) -> List<gui.UiEvent> {
    var out: List<gui.UiEvent> = []
    win.input(gui.Input.PointerMove(x, y)).unwrap()
    win.input(gui.Input.PointerDown(x, y)).unwrap()
    win.input(gui.Input.PointerUp(x, y)).unwrap()
    return out
}

fn drain(var app: pane.App<State>, win: gui.Window, events: Stream<gui.UiEvent>) -> Bool {
    win.post(1).unwrap()
    for e in events {
        match e {
            gui.UiEvent.User(n) => { return true }
            _ => {
                let open = app.step(e).unwrap()
                if !open { return false }
            }
        }
    }
    return false
}

fn main() {
    let win = gui.headless(420, 200).unwrap()
    let state = State { count: 0 }
    var app = pane.mount_chrome(win, state, view, theme.light(), chrome.windows(), "Demo").unwrap()
    let events = win.events()
    println(win.pixel(200, 10).unwrap() == 0xe5e7ebff)
    println(win.pixel(200, 100).unwrap() == 0xf4f5f7ff)
    click(win, 200.0, 18.0)
    println(drain(app, win, events))
    click(win, 30.0, 70.0)
    println(drain(app, win, events))
    println(state.count)
    click(win, 2.0, 100.0)
    println(drain(app, win, events))
    click(win, 351.0, 18.0)
    println(drain(app, win, events))
    click(win, 397.0, 18.0)
    println(drain(app, win, events))
    println(win.present().is_err())
}
"#,
        "chrome",
    );
    assert_eq!(got, ["true", "true", "true", "true", "1", "true", "true", "false", "true"]);
}

#[test]
fn the_presets_draw_their_buttons_where_their_style_puts_them() {
    let got = run(
        r#"import std.sys.gui as gui
import pane
import pane.theme
import pane.chrome

class State {
    var count: Int
}

fn view(s: State) -> pane.Element<State> {
    return pane.text("hi")
}

fn main() {
    let a = gui.headless(420, 200).unwrap()
    pane.mount_chrome(a, State { count: 0 }, view, theme.light(), chrome.mac(), "Demo").unwrap()
    println(a.pixel(20, 18).unwrap() == 0xff5f57ff)
    let b = gui.headless(420, 200).unwrap()
    pane.mount_chrome(b, State { count: 0 }, view, theme.light(), chrome.system(), "Demo").unwrap()
    println(b.pixel(20, 18).unwrap() == 0xf4f5f7ff)
    let c = gui.headless(420, 200).unwrap()
    pane.mount_chrome(c, State { count: 0 }, view, theme.dark(), chrome.flat(), "Demo").unwrap()
    println(c.pixel(5, 5).unwrap() == 0x374151ff)
}
"#,
        "presets",
    );
    assert_eq!(got, ["true", "true", "true"]);
}

#[test]
fn a_module_of_the_package_imports_by_name_beside_the_whole_package() {
    let got = run(
        r#"import std.sys.gui as gui
import pane
import pane.layout
import pane.controls as c

class State {
    var count: Int
}

fn view(s: State) -> pane.Element<State> {
    return layout.vstack([c.text("n = ${s.count}"), pane.button("go").on_click(|var s| { s.count += 1 })])
}

fn main() {
    let win = gui.headless(200, 100).unwrap()
    let app = pane.mount(win, State { count: 4 }, view).unwrap()
    println(win.text(win.hit(20.0, 30.0).unwrap().unwrap()).unwrap())
}
"#,
        "module_import",
    );
    assert_eq!(got, ["n = 4"]);
}

const DEMO: &str = include_str!("../../../packages/pane/demo.mote");

#[test]
fn the_demo_compiles() {
    let dir = std::env::temp_dir().join(format!("mote_pane_demo_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    common::install_package(&dir, "pane");
    std::fs::write(dir.join("main.mote"), DEMO).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("check").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}
