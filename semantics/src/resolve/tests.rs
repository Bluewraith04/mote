use mote_base::{Interner, Severity};

use super::resolve;
use crate::defs::{Bound, DefId, DefKind, ModuleId, Payload, Receiver, Resolved};
use crate::program::Program;
use crate::testing::{parse_sources, path_text};
use crate::types::{TyId, TyKind};

struct Run {
    program: Program,
    interner: Interner,
    errors: Vec<String>,
}

fn run(sources: &[(&str, &str)]) -> Run {
    let mut parsed = parse_sources(sources);
    let program = resolve(parsed.input, &mut parsed.interner, &mut parsed.sink);
    let errors = parsed.sink.iter().filter(|diagnostic| diagnostic.severity == Severity::Error).map(|diagnostic| diagnostic.message.clone()).collect();
    Run { program, interner: parsed.interner, errors }
}

impl Run {
    fn module(&self, path: &str) -> ModuleId {
        let found = self.program.modules.iter().find(|(_, module)| path_text(&self.interner, &module.path) == path);
        found.unwrap_or_else(|| panic!("no module {path}")).0
    }

    fn resolved(&self, module: &str, name: &str) -> Option<Resolved> {
        let name = self.interner.get(name)?;
        self.program.modules[self.module(module)].scope.get(&name).copied()
    }

    fn def(&self, module: &str, name: &str) -> DefId {
        match self.resolved(module, name) {
            Some(Resolved::Def(def)) => def,
            other => panic!("{name} in {module} is {other:?}"),
        }
    }

    fn exports(&self, module: &str, name: &str) -> bool {
        let Some(name) = self.interner.get(name) else { return false };
        self.program.modules[self.module(module)].exports.contains_key(&name)
    }

    fn ty(&self, ty: TyId) -> String {
        self.program.render(ty, &self.interner)
    }

    /// A function as `(Int, Int) -> Int`.
    fn signature(&self, def: DefId) -> String {
        let function = self.program.defs[def].function().expect("a function");
        let params: Vec<String> = function.params.iter().map(|param| self.ty(param.ty)).collect();
        format!("({}) -> {}", params.join(", "), self.ty(function.ret))
    }

    fn method(&self, owner: DefId, name: &str) -> DefId {
        let methods = match &self.program.defs[owner].kind {
            DefKind::Record(record) => &record.methods,
            DefKind::Enum(en) => &en.methods,
            DefKind::Trait(tr) => &tr.methods,
            _ => panic!("no methods"),
        };
        *methods.iter().find(|&&method| self.interner.resolve(self.program.defs[method].name) == name).expect("a method")
    }
}

fn main(text: &str) -> Run {
    run(&[("main", text)])
}

#[test]
fn every_item_binds_its_name() {
    let run = main("fn f() {}\nstruct S {}\nenum E { A }\ntrait T { fn m(self) }\ntype Id = Int\nlet G = 1\ntest \"works\" {}");
    assert_eq!(run.errors, Vec::<String>::new());
    for name in ["f", "S", "E", "T", "Id", "G"] {
        assert!(run.resolved("main", name).is_some(), "{name}");
    }
    assert_eq!(run.program.modules[run.module("main")].scope.len(), 6);
    assert!(matches!(run.program.defs[run.def("main", "S")].kind, DefKind::Record(_)));
}

#[test]
fn a_destructured_global_binds_each_name() {
    let run = main("let (a, b) = (1, 2)");
    assert_eq!(run.errors, Vec::<String>::new());
    assert!(run.resolved("main", "a").is_some() && run.resolved("main", "b").is_some());
}

#[test]
fn a_name_defined_twice_is_an_error() {
    let run = main("fn f() {}\nstruct f {}");
    assert_eq!(run.errors, ["`f` is defined twice"]);
}

#[test]
fn duplicate_methods_fields_and_variants_are_errors() {
    let run = main("struct S {\n x: Int\n x: Int\n fn m(self) {}\n fn m(self) {}\n}\nenum E {\n A\n A\n fn A(self) {}\n}");
    assert_eq!(run.errors, ["`m` is defined twice", "`A` is defined twice", "`A` is a variant and a method", "`x` is defined twice"]);
}

#[test]
fn a_module_import_binds_the_last_segment_or_the_alias() {
    let run = run(&[("util", "pub fn helper() {}"), ("main", "import .util\nimport .util as u")]);
    assert_eq!(run.errors, Vec::<String>::new());
    let util = run.module("util");
    assert_eq!(run.resolved("main", "util"), Some(Resolved::Module(util)));
    assert_eq!(run.resolved("main", "u"), Some(Resolved::Module(util)));
}

#[test]
fn a_named_import_takes_one_name_under_an_alias() {
    let run = run(&[("util", "pub fn helper() {}\nfn secret() {}\npub let MAX = 1"), ("main", "import { helper as h, MAX } from .util")]);
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(run.resolved("main", "h"), run.resolved("util", "helper"));
    assert_eq!(run.resolved("main", "MAX"), run.resolved("util", "MAX"));
    assert!(run.resolved("main", "helper").is_none());
}

#[test]
fn importing_a_private_or_missing_name_is_an_error() {
    let run = run(&[("util", "fn secret() {}"), ("main", "import { secret, nothing } from .util")]);
    assert_eq!(run.errors, ["`secret` is private in util", "no `nothing` in util"]);
}

#[test]
fn a_glob_brings_every_pub_name() {
    let run = run(&[("util", "pub fn a() {}\npub fn b() {}\nfn c() {}"), ("main", "import { * } from .util")]);
    assert_eq!(run.errors, Vec::<String>::new());
    assert!(run.resolved("main", "a").is_some() && run.resolved("main", "b").is_some());
    assert!(run.resolved("main", "c").is_none());
}

#[test]
fn two_globs_of_one_name_are_an_error_unless_it_is_the_same_item() {
    let clash = run(&[("a", "pub fn x() {}"), ("b", "pub fn x() {}"), ("main", "import { * } from .a\nimport { * } from .b")]);
    assert_eq!(clash.errors, ["`x` comes from two glob imports"]);
    let same = run(&[("origin", "pub fn x() {}"), ("a", "pub import { * } from .origin"), ("b", "pub import { * } from .origin"), ("main", "import { * } from .a\nimport { * } from .b")]);
    assert_eq!(same.errors, Vec::<String>::new());
    assert_eq!(same.resolved("main", "x"), same.resolved("origin", "x"));
}

#[test]
fn a_modules_own_and_explicit_names_win_over_a_glob() {
    let run = run(&[("a", "pub fn x() {}\npub fn y() {}"), ("b", "pub fn y() {}"), ("main", "fn x() {}\nimport { y } from .b\nimport { * } from .a")]);
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(run.program.defs[run.def("main", "x")].module, run.module("main"));
    assert_eq!(run.resolved("main", "y"), run.resolved("b", "y"));
}

#[test]
fn an_import_clashing_with_an_own_name_is_an_error() {
    let run = run(&[("util", "pub fn helper() {}"), ("main", "fn helper() {}\nimport { helper } from .util")]);
    assert_eq!(run.errors, ["`helper` is defined twice"]);
}

#[test]
fn pub_import_re_exports_and_a_plain_import_does_not() {
    let run = run(&[
        ("util", "pub fn helper() {}\npub fn other() {}"),
        ("facade", "pub import { helper } from .util\nimport { other } from .util"),
        ("main", "import { helper } from .facade"),
        ("late", "import { other } from .facade"),
    ]);
    assert_eq!(run.errors, ["`other` is private in facade"]);
    assert!(run.exports("facade", "helper") && !run.exports("facade", "other"));
    assert_eq!(run.resolved("main", "helper"), run.resolved("util", "helper"));
}

#[test]
fn pub_glob_and_pub_module_import_re_export() {
    let run = run(&[("util", "pub fn a() {}"), ("facade", "pub import { * } from .util\npub import .util as inner"), ("main", "import { a, inner } from .facade")]);
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(run.resolved("main", "inner"), Some(Resolved::Module(run.module("util"))));
}

#[test]
fn a_circular_import_names_its_modules_and_is_skipped() {
    let run = run(&[("a", "import .b\npub fn fa() {}"), ("b", "import .a\npub fn fb() {}")]);
    assert_eq!(run.errors, ["circular import a -> b -> a"]);
    assert!(run.resolved("a", "b").is_some());
    assert!(run.resolved("b", "a").is_none());
}

#[test]
fn a_module_importing_itself_is_a_cycle() {
    let run = run(&[("a", "import .a")]);
    assert_eq!(run.errors, ["circular import a -> a"]);
}

#[test]
fn the_prelude_is_visible_without_an_import_and_not_in_the_scope() {
    let run = run(&[("prelude", "pub struct List<T> {}"), ("main", "fn f(xs: List<Int>) {}")]);
    assert_eq!(run.errors, Vec::<String>::new());
    assert!(run.resolved("main", "List").is_none());
    assert_eq!(run.signature(run.def("main", "f")), "(List<Int>) -> Null");
}

#[test]
fn function_signatures_resolve_to_types() {
    let run = main("fn add(a: Int, b: Float32) -> Int { return a }\nfn none(x: Int?, y: (Int, String), z: [Int], w: [UInt8; 4]) {}\nfn pick(f: (Int) -> Int | String, u: Int | String) -> Bool { return true }");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(run.signature(run.def("main", "add")), "(Int, Float32) -> Int");
    assert_eq!(run.signature(run.def("main", "none")), "(Int?, (Int, String), [Int], [UInt8; 4]) -> Null");
    assert_eq!(run.signature(run.def("main", "pick")), "((Int) -> String | Int, String | Int) -> Bool");
}

#[test]
fn a_parameter_without_a_type_is_an_error() {
    let run = main("fn f(x) {}");
    assert_eq!(run.errors, ["parameter `x` needs a type"]);
}

#[test]
fn records_collect_fields_traits_and_methods() {
    let run = main(
        "trait Eq { fn eq(self, other: Self) -> Bool }\nclass Point: (Eq) {\n pub x: Int\n pub var y: Int = 2\n fn new(x: Int) -> Point { return Point { x } }\n fn grow(var self) {}\n fn eq(self, other: Point) -> Bool { return true }\n}",
    );
    assert_eq!(run.errors, Vec::<String>::new());
    let point = run.def("main", "Point");
    let record = run.program.defs[point].record().expect("a record");
    assert!(record.is_class);
    assert_eq!(record.fields.len(), 2);
    assert!(record.fields[0].is_pub && !record.fields[0].is_var);
    assert!(record.fields[1].is_var && record.fields[1].default.is_some());
    assert_eq!(record.traits, [Bound { def: run.def("main", "Eq"), args: Vec::new() }]);
    let grow = run.program.defs[run.method(point, "grow")].function().expect("a function");
    assert_eq!(grow.receiver, Receiver::Write);
    let new = run.program.defs[run.method(point, "new")].function().expect("a function");
    assert_eq!(new.receiver, Receiver::None);
    assert_eq!(run.signature(run.method(point, "eq")), "(Point) -> Bool");
}

#[test]
fn generics_number_the_owner_first_and_keep_bounds() {
    let run = main("trait Ord { fn lt(self, other: Self) -> Bool }\nstruct Box<T: Ord> {\n v: T\n fn map<U>(self, f: (T) -> U) -> Box<U> { return Box { v: f(self.v) } }\n}\nfn largest<T: Ord>(xs: List<T>) -> T? { return null }");
    assert_eq!(run.errors, ["unknown name `List`"]);
    let boxed = run.def("main", "Box");
    let record = run.program.defs[boxed].record().expect("a record");
    assert_eq!(record.generics[0].bounds, [Bound { def: run.def("main", "Ord"), args: Vec::new() }]);
    assert_eq!(run.signature(run.method(boxed, "map")), "((T) -> U) -> Box<U>");
    let map = run.program.defs[run.method(boxed, "map")].function().expect("a function");
    assert_eq!(map.generics.len(), 1);
    let TyKind::Adt { args, .. } = run.program.types.kind(map.ret) else { panic!("an adt") };
    assert!(matches!(run.program.types.kind(args[0]), TyKind::Param { index: 1, .. }));
}

#[test]
fn a_trait_method_sees_self_as_its_first_parameter() {
    let run = main("trait Eq { fn eq(self, other: Self) -> Bool }");
    let eq = run.method(run.def("main", "Eq"), "eq");
    assert_eq!(run.signature(eq), "(Self) -> Bool");
}

#[test]
fn a_bound_must_be_a_trait() {
    let run = main("struct S {}\nfn f<T: S>(x: T) {}\nfn g<T: Int>(x: T) {}");
    assert_eq!(run.errors, ["`S` is not a trait", "unknown name `Int`"]);
}

#[test]
fn enums_lower_payloads_and_discriminants() {
    let run = main("enum Shape {\n Circle(Int)\n Rect(Int, Int)\n Named { name: String, sides: Int }\n Empty\n}\nenum Level {\n Low = 1\n High = 10\n Min = -5\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    let shape = run.program.defs[run.def("main", "Shape")].enum_def().expect("an enum");
    assert_eq!(shape.variants.len(), 4);
    assert!(matches!(&shape.variants[1].payload, Payload::Tuple(types) if types == &[TyId::INT, TyId::INT]));
    assert!(matches!(&shape.variants[2].payload, Payload::Struct(fields) if fields.len() == 2));
    let level = run.program.defs[run.def("main", "Level")].enum_def().expect("an enum");
    assert_eq!(level.variants.iter().map(|variant| variant.discriminant).collect::<Vec<_>>(), [Some(1), Some(10), Some(-5)]);
}

#[test]
fn an_enum_method_cannot_take_var_self() {
    let run = main("enum E {\n A\n fn go(var self) {}\n}");
    assert_eq!(run.errors, ["an enum method takes `self`, not `var self`"]);
}

#[test]
fn aliases_substitute_their_arguments_in_any_order() {
    let run = main("fn f(p: Pair<Int>, i: Id) {}\ntype Pair<T> = (T, T)\ntype Id = Int | String");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(run.signature(run.def("main", "f")), "((Int, Int), String | Int) -> Null");
}

#[test]
fn an_alias_that_refers_to_itself_is_an_error() {
    let run = main("type A = B\ntype B = (A, Int)");
    assert_eq!(run.errors, ["type alias `A` refers to itself"]);
}

#[test]
fn type_argument_counts_are_checked() {
    let run = main("struct Box<T> {}\nstruct Plain {}\nfn f(a: Box, b: Plain<Int>, c: Box<Int, Int>) {}");
    assert_eq!(run.errors, ["write the type arguments of `Box`", "`Plain` takes no type arguments", "`Box` takes 1 type argument"]);
}

#[test]
fn unknown_modules_traits_and_values_are_not_types() {
    let run = run(&[("util", "pub fn helper() {}"), ("main", "import .util\nfn f(a: Missing, b: util, c: util.helper, d: util.nothing) {}")]);
    assert_eq!(run.errors, ["unknown name `Missing`", "`util` is a module, not a type", "`helper` is not a type", "no `nothing` in util"]);
}

#[test]
fn a_type_is_reached_through_a_module_name() {
    let run = run(&[("shapes", "pub struct Point {}"), ("main", "import .shapes\nfn f(p: shapes.Point) {}")]);
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(run.signature(run.def("main", "f")), "(Point) -> Null");
}

#[test]
fn self_outside_a_type_and_array_lengths_are_checked() {
    let run = main("fn f(a: Self, b: [Int; n]) {}");
    assert_eq!(run.errors, ["`Self` is only inside a type", "an array length is an integer literal"]);
}

#[test]
fn globals_take_their_annotation() {
    let run = main("let a: Int = 1\nvar b = 2");
    let a = run.program.defs[run.def("main", "a")].kind_global();
    assert_eq!(a, (TyId::INT, false));
    assert_eq!(run.program.defs[run.def("main", "b")].kind_global(), (TyId::ERROR, true));
}

impl crate::defs::Def {
    fn kind_global(&self) -> (TyId, bool) {
        match &self.kind {
            DefKind::Global(global) => (global.ty, global.is_var),
            _ => panic!("a global"),
        }
    }
}

#[test]
fn only_from_may_repeat_and_once_per_source_type() {
    let run = main("struct Meters {\n v: Float\n fn from(x: Int) -> Meters { return Meters { v: 0.0 } }\n fn from(x: String) -> Meters { return Meters { v: 0.0 } }\n fn from(x: Int) -> Meters { return Meters { v: 1.0 } }\n}");
    assert_eq!(run.errors, ["`from` for Int is defined twice"]);
}

#[test]
fn native_functions_record_their_link() {
    let run = main("@link(\"m\")\nnative fn pow(x: Float, y: Float) -> Float\n@link(\"sqlite3\", symbol = \"sqlite3_libversion_number\")\n@blocking\nnative fn version() -> Int32\nnative fn lost()");
    assert_eq!(run.errors, ["a native fn needs `@link`"]);
    let pow = run.program.defs[run.def("main", "pow")].function().expect("a function").link.expect("a link");
    assert_eq!(run.interner.resolve(pow.library), "m");
    assert_eq!(run.interner.resolve(pow.symbol), "pow");
    assert!(!pow.blocking);
    let version = run.program.defs[run.def("main", "version")].function().expect("a function").link.expect("a link");
    assert_eq!(run.interner.resolve(version.symbol), "sqlite3_libversion_number");
    assert!(version.blocking);
}

#[test]
fn an_intrinsic_is_a_generic_native_function_in_std() {
    let run = run(&[(
        "std.core",
        "@intrinsic(\"alloc_array\")\nnative fn alloc<T>(n: Int) -> [T]\n@intrinsic(\"x\")\n@link(\"m\")\nnative fn both()\n@intrinsic(1)\nnative fn named()\n@intrinsic\nnative fn bare()\n@intrinsic(\"x\")\nfn plain() {}",
    )]);
    assert_eq!(run.errors, ["a native fn takes `@link` or `@intrinsic`, not both", "`@intrinsic` takes a name in quotes", "`@intrinsic` takes one name", "`@intrinsic` goes on a native fn"]);
    let alloc = run.program.defs[run.def("std.core", "alloc")].function().expect("a function");
    assert_eq!(run.interner.resolve(alloc.intrinsic.expect("an intrinsic")), "alloc_array");
    assert_eq!(alloc.generics.len(), 1);
    assert!(alloc.link.is_none());
    let outside = main("@intrinsic(\"x\")\nnative fn f()");
    assert_eq!(outside.errors, ["`@intrinsic` is for std only"]);
}

#[test]
fn link_and_blocking_belong_on_native_functions() {
    let run = main("@link(\"m\")\nfn f() {}\n@blocking\nfn g() {}\n@link(1)\nnative fn h()");
    assert_eq!(run.errors, ["`@link` goes on a native fn", "`@blocking` goes on a native fn", "`@link` takes a library name and an optional `symbol`", "a native fn needs `@link`"]);
}

#[test]
fn attributes_are_a_closed_set() {
    let run = main("@nothing\nfn f() {}\n@stable\nfn g() {}\n@test\nfn h() {}\n@ignore\nfn i() {}\n@ignore\ntest \"slow\" {}");
    assert_eq!(run.errors, ["unknown attribute `@nothing`", "`@stable` is for std only", "unknown attribute `@test`", "`@ignore` goes on a test"]);
}

#[test]
fn lang_items_are_std_only_and_register_once() {
    let outside = main("@lang(\"list\")\nstruct List<T> {}");
    assert_eq!(outside.errors, ["`@lang` is for std only"]);
    let std = run(&[("std.core", "@lang(\"list\")\npub struct List<T> {}\n@lang(\"pointer\")\npub struct Pointer<T> { v: UInt64 }\n@lang(\"int8\")\npub struct Byte {}\n@lang(\"nope\")\nstruct X {}\n@lang(\"list\")\nstruct Y {}\n@lang(\"map\")\nfn z() {}")]);
    assert_eq!(std.errors, ["unknown lang item `nope`", "lang item `list` is declared twice", "`@lang` goes on a type or trait"]);
    let list = std.def("std.core", "List");
    assert_eq!(std.program.lang_item(crate::defs::LangItem::List), Some(list));
    assert_eq!(std.program.defs[list].lang, Some(crate::defs::LangItem::List));
}

#[test]
fn a_lang_declaration_stands_for_its_built_in_type() {
    let run = run(&[("std.core", "@lang(\"int8\")\npub struct Byte {}\n@lang(\"option\")\npub enum Option<T> { Some(T)\n None }"), ("main", "import { Byte, Option } from .std.core\nfn f(a: Byte, b: Option<Int>) {}")]);
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(run.signature(run.def("main", "f")), "(Int8, Int?) -> Null");
}

#[test]
fn an_import_that_names_no_module_is_skipped_quietly() {
    let run = main("import .elsewhere\nfn f() {}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert!(run.resolved("main", "elsewhere").is_none());
}
