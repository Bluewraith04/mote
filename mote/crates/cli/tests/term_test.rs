//! `std.term` on the fake platform: a scripted size and events, no real terminal.

use std::sync::Arc;

use isa::value::TypeRegistry;
use modules::MultiFileCompiler;
use platform::FakePlatform;

fn run(name: &str, source: &str, fake: FakePlatform) -> Arc<FakePlatform> {
    let dir = std::env::temp_dir().join(format!("mote_term_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let main_file = dir.join("main.mote");
    std::fs::write(&main_file, source).unwrap();
    let compiled = MultiFileCompiler::new(dir.clone()).compile_program(&main_file).unwrap();
    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let fake = Arc::new(fake);
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    rt.set_platform(fake.clone());
    rt.run_entry_on(1).unwrap();
    std::fs::remove_dir_all(dir).ok();
    fake
}

const PROGRAM: &str = "import std.term as term\n\
    fn show(e: term.Event) -> String {\n\
        match e {\n\
            Key(k, m) => {\n\
                match k {\n\
                    Char(c) => { return \"char ${c} ctrl=${m.ctrl}\" }\n\
                    Up => { return \"up shift=${m.shift}\" }\n\
                    F(n) => { return \"f${n}\" }\n\
                    _ => { return \"other key\" }\n\
                }\n\
            }\n\
            Mouse(m, col, row) => {\n\
                match m {\n\
                    Down(b) => { return \"down ${col},${row}\" }\n\
                    ScrollUp => { return \"scroll ${col},${row}\" }\n\
                    _ => { return \"mouse\" }\n\
                }\n\
            }\n\
            Resize(c, r) => { return \"resize ${c}x${r}\" }\n\
        }\n\
    }\n\
    fn main() {\n\
        let t = term.open(true).unwrap()\n\
        let size = t.size().unwrap()\n\
        println(\"size ${size.cols}x${size.rows}\")\n\
        for e in t.events() {\n\
            println(show(e))\n\
        }\n\
        t.close()\n\
    }\n";

#[test]
fn size_and_scripted_events_decode() {
    let fake = FakePlatform::new(1)
        .with_terminal(120, 40)
        .with_term_event(&[0, 233, 2])
        .with_term_event(&[0, -7, 1])
        .with_term_event(&[0, -105, 0])
        .with_term_event(&[1, 0, 0, 4, 9])
        .with_term_event(&[1, 4, 0, 1, 2])
        .with_term_event(&[2, 100, 30]);
    let fake = run("events", PROGRAM, fake);
    assert_eq!(fake.stdout(), "size 120x40\nchar é ctrl=true\nup shift=true\nf5\ndown 4,9\nscroll 1,2\nresize 100x30\n");
    assert!(!fake.term_is_open());
}

#[test]
fn a_program_without_a_terminal_gets_an_error() {
    let source = "import std.term as term\n\
        fn main() {\n\
            match term.open(false) {\n\
                Ok(t) => { println(\"opened\") }\n\
                Err(e) => { println(\"no terminal\") }\n\
            }\n\
        }\n";
    let fake = run("none", source, FakePlatform::new(1));
    assert_eq!(fake.stdout(), "no terminal\n");
}
