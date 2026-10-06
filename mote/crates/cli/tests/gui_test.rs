//! `std.sys.gui` over a headless window, run as real programs.

use std::process::Command;

fn run_text(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_gui_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
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

#[test]
fn a_styled_box_paints_its_pixels() {
    let got = run(
        r#"import std.sys.gui as gui

fn main() {
    let win = gui.headless(200, 100).unwrap()
    let red = win.style(gui.style().width(gui.Size.Px(100.0)).height(gui.Size.Px(50.0)).background(0xff0000ff)).unwrap()
    let box = win.add(win.root(), gui.Kind.Box).unwrap()
    win.set_style(box, red).unwrap()
    println(win.present().unwrap() > 0)
    println(win.pixel(10, 10).unwrap())
    println(win.pixel(150, 10).unwrap())
    println(win.pixel(10, 80).unwrap())
    println(win.pixel(500, 10).unwrap())
    println(win.rect(box).unwrap())
    win.close().unwrap()
}
"#,
        "paint",
    );
    assert_eq!(got, ["true", "4278190335", "4294967295", "4294967295", "-1", "[0.0, 0.0, 100.0, 50.0]"]);
}

#[test]
fn flex_grow_splits_the_free_space() {
    let got = run(
        r#"import std.sys.gui as gui

fn main() {
    let win = gui.headless(300, 100).unwrap()
    let row = win.style(gui.style().width(gui.Size.Pct(100.0)).height(gui.Size.Pct(100.0)).gap(10.0)).unwrap()
    let cell = win.style(gui.style().grow(1.0)).unwrap()
    win.set_style(win.root(), row).unwrap()
    let a = win.add(win.root(), gui.Kind.Box).unwrap()
    let b = win.add(win.root(), gui.Kind.Box).unwrap()
    win.set_style(a, cell).unwrap()
    win.set_style(b, cell).unwrap()
    win.present().unwrap()
    println(win.rect(a).unwrap())
    println(win.rect(b).unwrap())
}
"#,
        "flex",
    );
    assert_eq!(got, ["[0.0, 0.0, 145.0, 100.0]", "[155.0, 0.0, 145.0, 100.0]"]);
}

#[test]
fn a_point_hits_the_front_most_node() {
    let got = run(
        r#"import std.sys.gui as gui

fn main() {
    let win = gui.headless(100, 100).unwrap()
    let big = win.style(gui.style().width(gui.Size.Px(80.0)).height(gui.Size.Px(80.0))).unwrap()
    let small = win.style(gui.style().width(gui.Size.Px(20.0)).height(gui.Size.Px(20.0))).unwrap()
    let outer = win.add(win.root(), gui.Kind.Box).unwrap()
    win.set_style(outer, big).unwrap()
    let inner = win.add(outer, gui.Kind.Button).unwrap()
    win.set_style(inner, small).unwrap()
    win.present().unwrap()
    println(id_at(win, 5.0, 5.0) == inner.id)
    println(id_at(win, 50.0, 50.0) == outer.id)
    println(id_at(win, 95.0, 95.0) == win.root().id)
}

fn id_at(win: gui.Window, x: Float, y: Float) -> Int {
    match win.hit(x, y).unwrap() {
        Some(n) => { return n.id }
        None => { return -1 }
    }
}
"#,
        "hit",
    );
    assert_eq!(got, ["true", "true", "true"]);
}

#[test]
fn move_before_reorders_a_nodes_siblings() {
    let got = run(
        r#"import std.sys.gui as gui

fn main() {
    let win = gui.headless(100, 100).unwrap()
    let st = win.style(gui.style().width(gui.Size.Px(10.0)).height(gui.Size.Px(10.0))).unwrap()
    let a = win.add(win.root(), gui.Kind.Box).unwrap()
    let b = win.add(win.root(), gui.Kind.Box).unwrap()
    win.set_style(a, st).unwrap()
    win.set_style(b, st).unwrap()
    win.present().unwrap()
    println(win.rect(b).unwrap()[0])
    win.move_before(b, Some(a)).unwrap()
    win.present().unwrap()
    println(win.rect(b).unwrap()[0])
    win.move_before(b, None).unwrap()
    win.present().unwrap()
    println(win.rect(b).unwrap()[0])
    println(win.move_before(a, Some(win.root())).is_err())
}
"#,
        "move_before",
    );
    assert_eq!(got, ["10.0", "0.0", "10.0", "true"]);
}

#[test]
fn frame_calls_on_a_window_with_no_display_do_nothing() {
    let got = run(
        r#"import std.sys.gui as gui

fn main() {
    let win = gui.headless(100, 100).unwrap()
    win.set_frame(false).unwrap()
    win.drag().unwrap()
    win.drag_resize(gui.Edge.SouthWest).unwrap()
    win.minimize().unwrap()
    win.toggle_maximize().unwrap()
    win.set_title("x").unwrap()
    println(win.maximized().unwrap())
    win.close().unwrap()
    println(win.drag().is_err())
}
"#,
        "frame",
    );
    assert_eq!(got, ["false", "true"]);
}

#[test]
fn a_bad_window_or_node_is_an_error() {
    let got = run(
        r#"import std.sys.gui as gui

fn main() {
    println(gui.headless(0, 10).is_err())
    let win = gui.headless(10, 10).unwrap()
    let gone = win.add(win.root(), gui.Kind.Text).unwrap()
    win.remove(gone).unwrap()
    println(win.set_text(gone, "x").is_err())
    win.close().unwrap()
    println(win.present().is_err())
}
"#,
        "errors",
    );
    assert_eq!(got, ["true", "true", "true"]);
}

#[test]
fn input_raises_events_a_program_reads_in_order() {
    let got = run(
        r#"import std.sys.gui as gui

fn main() {
    let win = gui.headless(200, 100).unwrap()
    let look = win.style(gui.style().width(gui.Size.Px(100.0)).height(gui.Size.Px(50.0))).unwrap()
    let button = win.add(win.root(), gui.Kind.Button).unwrap()
    win.set_style(button, look).unwrap()
    win.present().unwrap()
    let events = win.events()
    win.input(gui.Input.PointerMove(10.0, 10.0)).unwrap()
    win.input(gui.Input.PointerDown(10.0, 10.0)).unwrap()
    println(win.input(gui.Input.PointerUp(10.0, 10.0)).unwrap())
    win.input(gui.Input.PointerLeave).unwrap()
    win.input(gui.Input.Key(gui.Key.Char(97), gui.mods(false, true, false, false), true)).unwrap()
    win.input(gui.Input.Resize(300, 120)).unwrap()
    win.post(7).unwrap()
    win.close().unwrap()
    for e in events {
        match e {
            gui.UiEvent.Enter(n) => { println("enter ${n.id == button.id}") }
            gui.UiEvent.Leave(n) => { println("leave ${n.id == button.id}") }
            gui.UiEvent.Click(n) => { println("click ${n.id == button.id}") }
            gui.UiEvent.PointerMove(n, x, y) => { println("move ${x} ${y}") }
            gui.UiEvent.PointerDown(n, x, y) => { println("down") }
            gui.UiEvent.PointerUp(n, x, y) => { println("up") }
            gui.UiEvent.Key(key, mods, down) => {
                match key {
                    gui.Key.Char(c) => { println("key ${c} ctrl=${mods.ctrl} shift=${mods.shift} ${down}") }
                    _ => { println("named key") }
                }
            }
            gui.UiEvent.Resize(w, h) => { println("resize ${w} ${h}") }
            gui.UiEvent.User(tag) => { println("user ${tag}") }
            _ => { println("other") }
        }
    }
}
"#,
        "events",
    );
    assert_eq!(
        got,
        ["2", "enter true", "move 10.0 10.0", "down", "up", "click true", "leave true", "key 97 ctrl=true shift=false true", "resize 300 120", "user 7"]
    );
}

#[test]
fn typing_into_an_input_raises_changed_and_submit() {
    let got = run(
        r#"import std.sys.gui as gui

fn main() {
    let win = gui.headless(240, 60).unwrap()
    let look = win.style(gui.style().width(gui.Size.Px(200.0)).height(gui.Size.Px(30.0))).unwrap()
    let field = win.add(win.root(), gui.Kind.Input).unwrap()
    win.set_style(field, look).unwrap()
    win.present().unwrap()
    let events = win.events()
    win.input(gui.Input.PointerDown(20.0, 15.0)).unwrap()
    win.input(gui.Input.PointerUp(20.0, 15.0)).unwrap()
    println(win.focus().unwrap().unwrap().id == field.id)
    win.input(gui.Input.Text("hello")).unwrap()
    let none = gui.no_mods()
    win.input(gui.Input.Key(gui.Key.Backspace, none, true)).unwrap()
    println(win.text(field).unwrap())
    println(win.selection(field).unwrap())
    win.input(gui.Input.Key(gui.Key.Enter, none, true)).unwrap()
    win.close().unwrap()
    for e in events {
        match e {
            gui.UiEvent.Changed(n) => { println("changed") }
            gui.UiEvent.Submit(n) => { println("submit") }
            _ => { }
        }
    }
}
"#,
        "typing",
    );
    assert_eq!(got, ["true", "hell", "[4, 4]", "changed", "changed", "submit"]);
}

#[test]
fn a_window_has_one_event_reader() {
    let (ok, text) = run_text(
        r#"import std.sys.gui as gui

fn main() {
    let win = gui.headless(10, 10).unwrap()
    let first = win.events()
    println("opened")
    let second = win.events()
    println("unreachable")
}
"#,
        "one_reader",
    );
    assert!(!ok && text.starts_with("opened\n") && text.contains("already being read") && !text.contains("unreachable"), "{text}");
}

const DEMO: &str = include_str!("../../../examples/gui_demo.mote");

#[test]
fn the_demo_compiles() {
    let dir = std::env::temp_dir().join(format!("mote_gui_demo_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), DEMO).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("check").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}

/// The demo's own code on a headless window, driven by input instead of a person.
#[test]
fn the_demo_responds_to_clicks_typing_and_posts() {
    let driven = DEMO
        .replace("    task.pin()\n", "")
        .replace("gui.open(\"Mote GUI demo\", 480, 560)", "gui.headless(480, 560)")
        .replace(
            "    for e in win.events() {",
            r#"    let events = win.events()
    let b = win.rect(button)!
    win.input(gui.Input.PointerMove(b.get(0) + 5.0, b.get(1) + 5.0))!
    win.input(gui.Input.PointerDown(b.get(0) + 5.0, b.get(1) + 5.0))!
    win.input(gui.Input.PointerUp(b.get(0) + 5.0, b.get(1) + 5.0))!
    win.input(gui.Input.PointerDown(b.get(0) + 5.0, b.get(1) + 5.0))!
    win.input(gui.Input.PointerUp(b.get(0) + 5.0, b.get(1) + 5.0))!
    let f = win.rect(field)!
    win.input(gui.Input.PointerDown(f.get(0) + 20.0, f.get(1) + 10.0))!
    win.input(gui.Input.PointerUp(f.get(0) + 20.0, f.get(1) + 10.0))!
    win.input(gui.Input.Text("hi"))!
    let l = win.rect(list)!
    win.input(gui.Input.Wheel(l.get(0) + 20.0, l.get(1) + 20.0, 0.0, 60.0))!
    win.post(5)!
    win.input(gui.Input.Close)!
    for e in events {"#,
        )
        .replace(
            "            gui.UiEvent.Close => {\n",
            "            gui.UiEvent.Close => {\n                println(win.text(button)!)\n                println(win.text(echo)!)\n                println(win.text(clock)!)\n",
        );
    let got = run(&driven, "demo_driven");
    assert_eq!(got, ["Clicked 2 times", "You typed: hi", "Seconds up: 5"]);
}

#[test]
fn a_window_on_a_display_opens_only_from_a_pinned_task() {
    let got = run(
        r#"import std.sys.gui as gui

fn main() {
    match gui.open("hello", 100, 100) {
        Ok(win) => { println("opened") }
        Err(e) => { println(e.message) }
    }
}
"#,
        "unpinned",
    );
    assert_eq!(got, ["a window opens from a pinned task: call task.pin() first"]);
}

#[test]
fn a_window_with_no_display_is_an_error_not_a_crash() {
    let got = run(
        r#"import std.sys.gui as gui
import std.task as task

fn main() {
    task.pin()
    println(gui.open("hello", 100, 100).is_err())
    println(gui.open("hello", 0, 100).is_err())
}
"#,
        "nodisplay",
    );
    assert_eq!(got, ["true", "true"]);
}

#[test]
fn a_picture_encodes_as_png() {
    let got = run(
        r#"import std.sys.gui as gui

fn main() {
    let win = gui.headless(8, 8).unwrap()
    win.present().unwrap()
    let png = win.png().unwrap()
    println(png.get(1))
    println(png.get(2))
    println(png.get(3))
}
"#,
        "png",
    );
    assert_eq!(got, ["80", "78", "71"]);
}

#[test]
fn runs_give_a_text_node_rich_text_and_links() {
    let got = run(
        r#"import std.sys.gui as gui

fn main() {
    let win = gui.headless(300, 100).unwrap()
    let row = win.style(gui.style().align_items(gui.Align.Start)).unwrap()
    win.set_style(win.root(), row).unwrap()
    let t = win.add(win.root(), gui.Kind.Text).unwrap()
    win.set_text(t, "plain").unwrap()
    win.present().unwrap()
    let plain = win.rect(t).unwrap().get(2)
    win.set_runs(t, [gui.run("0000000000").mono(), gui.run("docs").underline().link(7), gui.run(" end").bold().italic().color(0xff0000ff)]).unwrap()
    win.present().unwrap()
    println(win.text(t).unwrap())
    println(win.rect(t).unwrap().get(2) > plain)
    let events = win.events()
    win.input(gui.Input.PointerDown(110.0, 8.0)).unwrap()
    win.input(gui.Input.PointerUp(110.0, 8.0)).unwrap()
    win.input(gui.Input.PointerDown(20.0, 8.0)).unwrap()
    win.input(gui.Input.PointerUp(20.0, 8.0)).unwrap()
    win.set_text(t, "again").unwrap()
    println(win.text(t).unwrap())
    win.close().unwrap()
    for e in events {
        match e {
            gui.UiEvent.Link(n, id) => { println("link ${n.id == t.id} ${id}") }
            _ => {}
        }
    }
}
"#,
        "runs",
    );
    assert_eq!(got, ["0000000000docs end", "true", "again", "link true 7"]);
}

#[test]
fn runs_on_a_box_are_an_error() {
    let got = run(
        r#"import std.sys.gui as gui

fn main() {
    let win = gui.headless(50, 50).unwrap()
    let b = win.add(win.root(), gui.Kind.Box).unwrap()
    match win.set_runs(b, [gui.run("x")]) {
        Ok(_) => { println("ok") }
        Err(e) => { println(e.message) }
    }
}
"#,
        "runs_box",
    );
    assert_eq!(got, ["only a text or button node holds runs"]);
}
