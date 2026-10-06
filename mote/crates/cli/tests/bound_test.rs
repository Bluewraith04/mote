//! Trait bounds are accepted, checked at calls and used in generic bodies (`mote check`).

use std::process::Command;

fn check(source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_bound_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("check").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn rejected(source: &str, tag: &str, message: &str) {
    let (ok, text) = check(source, tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

const SHAPES: &str = "trait Shape {\n    fn area(self) -> Int\n}\nclass Sq {\n    side: Int\n    pub fn area(self) -> Int { return self.side * self.side }\n}\nclass Blob {\n    n: Int\n}\n";

#[test]
fn a_type_that_satisfies_the_bound_is_accepted() {
    let src = format!("{SHAPES}fn one<T: Shape>(a: T) -> T {{ return a }}\nfn main() {{\n    one(Sq {{ side: 1 }})\n}}\n");
    let (ok, text) = check(&src, "ok");
    assert!(ok, "{text}");
}

#[test]
fn a_type_missing_the_method_is_rejected_at_the_call() {
    let src = format!("{SHAPES}fn one<T: Shape>(a: T) -> T {{ return a }}\nfn main() {{\n    one(Blob {{ n: 1 }})\n}}\n");
    rejected(&src, "missing", "`Blob` cannot be used for `T`: missing method `area`");
}

#[test]
fn a_primitive_does_not_satisfy_a_user_trait() {
    let src = format!("{SHAPES}fn one<T: Shape>(a: T) -> T {{ return a }}\nfn main() {{\n    one(3)\n}}\n");
    rejected(&src, "prim", "`Int` cannot be used for `T`");
}

#[test]
fn an_unbounded_parameter_still_has_no_methods() {
    rejected("fn f<T>(a: T) -> Int {\n    return a.area()\n}\nfn main() { }\n", "unbounded", "type parameter with no bound");
}

#[test]
fn a_method_outside_the_bound_is_rejected() {
    let src = format!("{SHAPES}fn f<T: Shape>(a: T) -> Int {{\n    return a.side\n}}\nfn main() {{ }}\n");
    let (ok, _) = check(&src, "field");
    assert!(!ok);
}

#[test]
fn a_bound_must_name_a_trait() {
    rejected("fn f<T: Missing>(a: T) -> T { return a }\nfn main() { }\n", "notrait", "`Missing` is not a trait");
}

#[test]
fn a_bound_on_a_type_parameter_is_still_refused() {
    rejected("trait Shape {\n    fn area(self) -> Int\n}\nclass Box<T: Shape> {\n    v: T\n}\nfn main() { }\n", "onclass", "not supported yet");
}

#[test]
fn a_wrong_argument_to_a_bound_method_is_rejected() {
    let src = "trait Scale {\n    fn scaled(self, k: Int) -> Int\n}\nfn f<T: Scale>(a: T) -> Int {\n    return a.scaled(\"x\")\n}\nfn main() { }\n";
    rejected(src, "arg", "argument 1 expects");
}
