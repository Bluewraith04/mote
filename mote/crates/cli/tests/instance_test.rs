//! Bounded generics are instantiated per concrete type and dispatch statically.

use std::process::Command;

fn run_files(files: &[(&str, &str)], tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_inst_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, src) in files {
        std::fs::write(dir.join(name), src).unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn run(source: &str, tag: &str) -> (bool, String) {
    run_files(&[("main.mote", source)], tag)
}

const SHAPES: &str = "trait Shape {\n    fn area(self) -> Int\n}\nclass Sq {\n    side: Int\n    pub fn area(self) -> Int { return self.side * self.side }\n}\nclass Rect {\n    w: Int\n    h: Int\n    pub fn area(self) -> Int { return self.w * self.h }\n}\n";

#[test]
fn a_bounded_body_calls_the_bound_method() {
    let src = format!("{SHAPES}fn total<T: Shape>(a: T, b: T) -> Int {{\n    return a.area() + b.area()\n}}\nfn main() {{\n    println(total(Sq {{ side: 1 }}, Sq {{ side: 2 }}))\n}}\n");
    let (ok, text) = run(&src, "body");
    assert!(ok && text.trim() == "5", "{text}");
}

#[test]
fn each_concrete_type_gets_its_own_instance() {
    let src = format!("{SHAPES}fn area_of<T: Shape>(a: T) -> Int {{\n    return a.area()\n}}\nfn main() {{\n    println(area_of(Sq {{ side: 3 }}))\n    println(area_of(Rect {{ w: 2, h: 5 }}))\n}}\n");
    let (ok, text) = run(&src, "two");
    assert!(ok && text.trim() == "9\n10", "{text}");
}

#[test]
fn a_bounded_function_may_call_another() {
    let src = format!("{SHAPES}fn area_of<T: Shape>(a: T) -> Int {{\n    return a.area()\n}}\nfn twice<T: Shape>(a: T) -> Int {{\n    return area_of(a) * 2\n}}\nfn main() {{\n    println(twice(Rect {{ w: 2, h: 5 }}))\n}}\n");
    let (ok, text) = run(&src, "chain");
    assert!(ok && text.trim() == "20", "{text}");
}

#[test]
fn a_default_method_is_reachable_through_the_bound() {
    let src = "trait Shape {\n    fn area(self) -> Int\n    fn double(self) -> Int { return self.area() * 2 }\n}\nclass Sq: (Shape) {\n    side: Int\n    pub fn area(self) -> Int { return self.side * self.side }\n}\nfn dbl<T: Shape>(a: T) -> Int {\n    return a.double()\n}\nfn main() {\n    println(dbl(Sq { side: 3 }))\n}\n";
    let (ok, text) = run(src, "default");
    assert!(ok && text.trim() == "18", "{text}");
}

#[test]
fn a_bounded_function_works_across_modules() {
    let shapes = "pub trait Shape {\n    fn area(self) -> Int\n}\npub class Sq {\n    side: Int\n    pub fn new(side: Int) -> Sq { return Sq { side: side } }\n    pub fn area(self) -> Int { return self.side * self.side }\n}\npub fn area_of<T: Shape>(a: T) -> Int {\n    return a.area()\n}\n";
    let main = "import { Sq, area_of } from .shapes\nfn main() {\n    println(area_of(Sq.new(4)))\n}\n";
    let (ok, text) = run_files(&[("main.mote", main), ("shapes.mote", shapes)], "modules");
    assert!(ok && text.trim() == "16", "{text}");
}

#[test]
fn a_bounded_generic_returning_t_keeps_the_type() {
    let src = format!("{SHAPES}fn biggest<T: Shape>(a: T, b: T) -> T {{\n    if a.area() >= b.area() {{ return a }}\n    return b\n}}\nfn main() {{\n    let r = biggest(Sq {{ side: 1 }}, Sq {{ side: 3 }})\n    println(r.side)\n}}\n");
    let (ok, text) = run(&src, "ret");
    assert!(ok && text.trim() == "3", "{text}");
}
