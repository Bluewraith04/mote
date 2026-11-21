use mote::{Options, Pipeline, Target};
use mote_abi::{passing, HandleKind, Kind, PairPayload, Passing, Scalar, TypeId, TypeInfo};
use mote_packages::testing::Scratch;

use super::*;

/// The program built and its table.
fn mapped(source: &str) -> (Pipeline, TypeMap) {
    let scratch = Scratch::new();
    let dir = scratch.package("app", "app", &[], &[("main.mote", source)]);
    let mut pipeline = Pipeline::build(&dir, Target::Run, &Options::default()).unwrap_or_else(|failure| panic!("{failure}"));
    let map = build(&pipeline.module, &mut pipeline.program, &pipeline.interner);
    (pipeline, map)
}

/// The entry with the name.
fn entry<'a>(map: &'a TypeMap, name: &str) -> (TypeId, &'a TypeInfo) {
    let mut found = map.table.iter().filter(|(_, info)| info.name == name);
    let first = found.next().unwrap_or_else(|| panic!("no type {name}: {:?}", map.table.iter().map(|(_, info)| info.name.as_str()).collect::<Vec<_>>()));
    assert!(found.next().is_none(), "two types are called {name}");
    first
}

const POINT: &str = "struct Point {\n    x: Int\n    y: Int\n}\n\nfn main() {\n    let p = Point { x: 1, y: 2 }\n    println(p.x + p.y)\n}\n";

#[test]
fn typetable_numbers_and_bool_are_scalars_and_each_type_keeps_its_own_entry() {
    let source = "fn main() {\n    let a: Int8 = 1\n    let b: UInt8 = 2\n    let c: Int32 = 3\n    let d: UInt64 = 4\n    let e: Float32 = 1.5\n    let f = true\n    let g = 7\n    let h = 2.5\n    println(a)\n    println(b)\n    println(c)\n    println(d)\n    println(e)\n    println(f)\n    println(g)\n    println(h)\n}\n";
    let (_pipeline, map) = mapped(source);
    let scalar = |name: &str| match entry(&map, name).1.kind {
        Kind::Scalar(scalar) => scalar,
        ref other => panic!("{name} is {other:?}"),
    };
    let found: Vec<Scalar> = ["Int8", "UInt8", "Int32", "UInt64", "Float32", "Bool", "Int", "Float"].iter().map(|name| scalar(name)).collect();
    assert_eq!(found, [Scalar::I8, Scalar::I8, Scalar::I32, Scalar::I64, Scalar::F32, Scalar::I8, Scalar::I64, Scalar::F64]);
    assert_ne!(entry(&map, "Int").0, entry(&map, "UInt64").0);
}

#[test]
fn typetable_a_struct_is_its_fields_inline_and_travels_in_registers() {
    let (_pipeline, map) = mapped(POINT);
    let (id, info) = entry(&map, "app.main.Point");
    assert!(matches!(info.kind, Kind::Struct(_)));
    assert_eq!((info.layout.size, info.layout.align), (16, 8));
    assert_eq!(passing(&map.table, id), Passing::Registers(vec![Scalar::I64, Scalar::I64]));
}

#[test]
fn typetable_a_tuple_is_a_struct_of_its_elements() {
    let source = "fn main() {\n    let t = (1, \"a\", true)\n    println(t.0)\n}\n";
    let (_pipeline, map) = mapped(source);
    let (_, info) = entry(&map, "(Int, String, Bool)");
    assert!(matches!(info.kind, Kind::Struct(_)));
    assert_eq!(info.fields.iter().map(|field| field.offset).collect::<Vec<_>>(), [0, 8, 16]);
    assert_eq!(info.slots.len(), 1);
}

#[test]
fn typetable_a_class_is_an_object_with_a_reference_and_may_hold_itself() {
    let source = "class Node {\n    var next: Node? = null\n    var value: Int = 0\n}\n\nfn main() {\n    let n = Node()\n    println(n.value)\n}\n";
    let (_pipeline, map) = mapped(source);
    let (id, info) = entry(&map, "app.main.Node");
    let Kind::Class(fields) = &info.kind else { panic!("{:?}", info.kind) };
    assert_eq!(fields.len(), 2);
    assert!(info.is_reference());
    assert_eq!(passing(&map.table, id), Passing::Registers(vec![Scalar::Ref]));
    assert_eq!(map.table.get(fields[0]).layout.size, 8);
}

#[test]
fn typetable_an_enum_has_a_tag_and_an_optional_of_an_object_is_the_reference() {
    let source = "enum Shape {\n    Circle(Int)\n    Rect(Int, Int)\n    Empty\n}\n\nfn main() {\n    let s = Shape.Rect(1, 2)\n    let name: String? = null\n    println(name == null)\n    match s {\n        Circle(r) => println(r)\n        Rect(w, h) => println(w * h)\n        Empty => println(0)\n    }\n}\n";
    let (_pipeline, map) = mapped(source);
    let (_, shape) = entry(&map, "app.main.Shape");
    assert_eq!((shape.layout.size, shape.tag_size, shape.variants.len()), (24, 1, 3));
    let (_, optional) = entry(&map, "String?");
    assert!(optional.is_reference());
}

#[test]
fn typetable_a_recursive_enum_holds_its_children_in_boxes() {
    let source = "enum Tree {\n    Leaf(Int)\n    Node(Tree, Tree)\n}\n\nfn sum(t: Tree) -> Int {\n    return match t {\n        Leaf(n) => n\n        Node(a, b) => sum(a) + sum(b)\n    }\n}\n\nfn main() {\n    println(sum(Tree.Node(Tree.Leaf(1), Tree.Leaf(2))))\n}\n";
    let (pipeline, map) = mapped(source);
    let (tree, info) = entry(&map, "app.main.Tree");
    assert_eq!(info.layout.size, 24);
    let node = &info.variants[1];
    for field in &node.fields {
        assert!(matches!(map.table.get(field.ty).kind, Kind::Box(inner) if inner == tree), "{:?}", map.table.get(field.ty).kind);
    }
    let tree_ty = pipeline.module.functions.iter().find_map(|(_, function)| {
        matches!(&function.kind, FunctionKind::Instance(instance) if pipeline.interner.resolve(pipeline.program.defs[instance.def].name) == "sum")
            .then(|| function.params.first().map(|&param| function.regs[param].ty))
            .flatten()
    }).unwrap();
    assert_eq!(map.id(tree_ty), tree);
    assert_eq!(map.boxed(tree_ty), node.fields[0].ty);
}

#[test]
fn typetable_a_value_type_that_holds_itself_through_an_optional_is_boxed_too() {
    let source = "enum Chain {\n    End\n    Link(Int, Chain?)\n}\n\nfn main() {\n    let c = Chain.Link(1, null)\n    match c {\n        End => println(0)\n        Link(n, _) => println(n)\n    }\n}\n";
    let (_pipeline, map) = mapped(source);
    let (chain, info) = entry(&map, "app.main.Chain");
    assert_eq!(info.layout.size, 24);
    let Kind::Box(inner) = map.table.get(info.variants[1].fields[1].ty).kind else { panic!("the link is not boxed") };
    assert_eq!(map.table.get(inner).name, "app.main.Chain?");
    assert_ne!(inner, chain);
}

#[test]
fn typetable_a_union_is_a_pair_and_a_member_that_is_a_value_has_a_box() {
    let source = "struct Point {\n    x: Int\n    y: Int\n}\n\nfn show(v: Int | Point | String) -> Int {\n    return 1\n}\n\nfn main() {\n    let p = Point { x: 1, y: 2 }\n    println(show(3) + show(p))\n}\n";
    let (pipeline, map) = mapped(source);
    let (pair, info) = entry(&map, "String | Int | app.main.Point");
    assert!(matches!(info.kind, Kind::Pair));
    assert_eq!(info.layout.size, 16);
    let (point, _) = entry(&map, "app.main.Point");
    assert_eq!(map.table.pair_payload(point), PairPayload::Boxed);
    assert_eq!(map.id(a_reg_of(&pipeline, &map, pair)), pair);
    let member = a_reg_of(&pipeline, &map, point);
    assert!(matches!(map.table.get(map.boxed(member)).kind, Kind::Box(inner) if inner == point));
}

/// The type of some register of the program that has the entry.
fn a_reg_of(pipeline: &Pipeline, map: &TypeMap, id: TypeId) -> TyId {
    pipeline.module.functions.iter().flat_map(|(_, function)| function.regs.iter().map(|(_, reg)| reg.ty)).find(|&ty| map.id(ty) == id).expect("a register has the type")
}

#[test]
fn typetable_a_list_has_a_buffer_array_and_an_array_strides_by_its_element() {
    let source = "struct Point {\n    x: Int\n    y: Int\n}\n\nfn main() {\n    var ps: List<Point> = []\n    ps.push(Point { x: 1, y: 2 })\n    println(ps.len())\n}\n";
    let (_pipeline, map) = mapped(source);
    let (_, list) = entry(&map, "List<app.main.Point>");
    assert!(matches!(list.kind, Kind::List(_)));
    let (_, buffer) = entry(&map, "[app.main.Point]");
    assert_eq!(buffer.object.as_ref().and_then(|object| object.element).map(|element| element.stride), Some(16));
}

#[test]
fn typetable_handles_have_their_kinds_and_a_channel_is_behind_its_ends() {
    let source = "import Channel from std.sync\n\nfn main() {\n    let (tx, rx) = Channel<Int>(1)\n    let m = Mutex(5)\n    scope {\n        let t = spawn { return 1 }\n        tx.send(2)\n        println(rx.recv())\n        println(t.await())\n    }\n    println(m.get())\n}\n";
    let (_pipeline, map) = mapped(source);
    let kind = |name: &str| match &entry(&map, name).1.kind {
        Kind::Handle(handle, payload) => (*handle, payload.len()),
        other => panic!("{name} is {other:?}"),
    };
    assert_eq!(kind("Sender<Int>"), (HandleKind::Sender, 0));
    assert_eq!(kind("Receiver<Int>"), (HandleKind::Receiver, 0));
    assert_eq!(kind("Mutex<Int>"), (HandleKind::Mutex, 1));
    assert_eq!(kind("Task<Int>"), (HandleKind::Task, 1));
    assert_eq!(kind("Channel<Int>"), (HandleKind::Channel, 1));
}

#[test]
fn typetable_a_closure_keeps_its_captures_and_a_shared_var_is_a_cell() {
    let source = "fn main() {\n    var count = 0\n    let name = \"n\"\n    let bump = || {\n        count += 1\n        println(name)\n    }\n    bump()\n    println(count)\n}\n";
    let (pipeline, map) = mapped(source);
    let (func, _) = pipeline.module.functions.iter().find(|(_, function)| matches!(function.kind, FunctionKind::Closure { .. })).unwrap();
    let closure = map.table.get(map.closure(func));
    let Kind::Closure(captures) = &closure.kind else { panic!("{:?}", closure.kind) };
    assert_eq!(captures.len(), 2);
    assert!(captures.iter().any(|&capture| matches!(map.table.get(capture).kind, Kind::Box(_))));
    assert_eq!(closure.object.as_ref().map(|object| object.fields.len()), Some(2));
}

#[test]
fn typetable_a_named_function_used_as_a_value_is_a_closure_with_no_captures() {
    let source = "fn double(x: Int) -> Int {\n    return x * 2\n}\n\nfn apply(f: (Int) -> Int, n: Int) -> Int {\n    if n == 0 {\n        return f(4)\n    }\n    return apply(f, n - 1)\n}\n\nfn main() {\n    println(apply(double, 3))\n}\n";
    let (pipeline, map) = mapped(source);
    let (func, _) = pipeline.module.functions.iter().find(|(_, function)| matches!(&function.kind, FunctionKind::Instance(instance) if pipeline.interner.resolve(pipeline.program.defs[instance.def].name) == "double")).unwrap();
    assert!(matches!(&map.table.get(map.closure(func)).kind, Kind::Closure(captures) if captures.is_empty()));
}

#[test]
fn typetable_every_std_test_file_gets_a_table_with_an_entry_for_each_register() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../mote_std/std/tests");
    let mut paths: Vec<_> = std::fs::read_dir(&dir).expect("the std/tests directory").map(|entry| entry.expect("a directory entry").path()).collect();
    paths.sort();
    for path in paths.iter().filter(|path| path.extension().is_some_and(|extension| extension == "mote")) {
        let mut compiled = mote_semantics::compile(&std::fs::read_to_string(path).expect("a readable file"));
        assert_eq!(compiled.errors, Vec::<String>::new(), "{}", path.display());
        let lowered = mote_ir::lower(&mut compiled.program, &mut compiled.interner);
        let map = build(&lowered.module, &mut compiled.program, &compiled.interner);
        for (_, function) in lowered.module.functions.iter() {
            for (_, reg) in function.regs.iter() {
                assert!(map.id(reg.ty).index() > 0, "{}", path.display());
            }
        }
    }
}

#[test]
fn typetable_every_type_of_a_register_has_an_entry_and_a_second_build_gives_the_same_bytes() {
    let source = "import Channel from std.sync\n\nenum Tree {\n    Leaf(Int)\n    Node(Tree, Tree)\n}\n\nclass Cell {\n    var items: List<(Int, String)> = []\n    var by_name: Map<String, Int> = {}\n    var next: Cell? = null\n}\n\nfn gen() -> Stream<Int> {\n    yield 1\n    yield 2\n}\n\nfn main() {\n    var c = Cell()\n    c.items.push((1, \"a\"))\n    c.by_name[\"a\"] = 1\n    let t = Tree.Node(Tree.Leaf(1), Tree.Leaf(2))\n    let u: Int | String | Float = 1\n    let fixed: [Int; 3] = [1, 2, 3]\n    let (tx, rx) = Channel<String>(1)\n    for v in gen() { println(v) }\n    println(fixed[0])\n    println(c.by_name.len())\n}\n";
    let (pipeline, map) = mapped(source);
    for (_, function) in pipeline.module.functions.iter() {
        for (_, reg) in function.regs.iter() {
            assert!(map.id(reg.ty).index() > 0);
        }
    }
    let (_again_pipeline, again) = mapped(source);
    assert_eq!(map.table.len(), again.table.len());
    assert!(map.table.iter().zip(again.table.iter()).all(|(left, right)| left == right));
}
