//! Which struct fields are embedded, and the layout that gives.

use compiler::Compiler;
use isa::value::TypeDescriptor;

fn layout(source: &str, name: &str) -> TypeDescriptor {
    let program = Compiler::compile(&format!("{source}\nfn main() {{\n}}\n"), "main.mote").unwrap();
    program.type_descriptors.iter().find(|t| t.name.as_deref() == Some(name)).unwrap_or_else(|| panic!("no type {name}")).clone()
}

const V2: &str = "struct V2 {\n    var x: Int\n    var y: Int\n}\n";

#[test]
fn a_struct_field_of_struct_type_is_embedded() {
    let seg = layout(&format!("{V2}struct Seg {{\n    var a: V2\n    var b: V2\n    var n: Int\n}}\n"), "Seg");
    assert_eq!(seg.slots, 5);
    assert_eq!(seg.fields.iter().map(|f| f.slot).collect::<Vec<_>>(), [0, 2, 4]);
    assert!(seg.fields[0].inline.is_some() && seg.fields[1].inline.is_some() && seg.fields[2].inline.is_none());
}

#[test]
fn embedding_nests() {
    let src = format!("{V2}struct Seg {{\n    var a: V2\n    var b: V2\n}}\nstruct Poly {{\n    var tag: String\n    var s: Seg\n    var c: V2\n}}\n");
    let poly = layout(&src, "Poly");
    assert_eq!(poly.slots, 7);
    assert_eq!(poly.fields.iter().map(|f| f.slot).collect::<Vec<_>>(), [0, 1, 5]);
}

#[test]
fn the_order_of_declaration_does_not_matter() {
    let src = format!("struct Seg {{\n    var a: V2\n}}\n{V2}");
    assert_eq!(layout(&src, "Seg").slots, 2);
}

#[test]
fn a_class_can_embed_a_struct_but_is_never_embedded() {
    let src = format!("{V2}class Node {{\n    var p: V2\n}}\nclass Holder {{\n    var n: Node\n}}\n");
    assert_eq!(layout(&src, "Node").slots, 2);
    let holder = layout(&src, "Holder");
    assert_eq!(holder.slots, 1);
    assert!(holder.fields[0].inline.is_none());
}

#[test]
fn optional_generic_and_collection_fields_are_not_embedded() {
    let src = format!("{V2}class Mixed {{\n    var a: V2?\n    var b: List<V2>\n}}\nstruct Wrap<T> {{\n    var t: T\n    var v: V2\n}}\n");
    let mixed = layout(&src, "Mixed");
    assert_eq!(mixed.slots, 2);
    assert!(mixed.fields.iter().all(|f| f.inline.is_none()));
    let wrap = layout(&src, "Wrap");
    assert_eq!((wrap.slots, wrap.fields[0].inline.is_none(), wrap.fields[1].inline.is_some()), (3, true, true));
}

#[test]
fn a_parameter_named_like_a_struct_is_not_embedded() {
    let src = format!("{V2}struct Wrap<V2> {{\n    var t: V2\n}}\n");
    assert!(layout(&src, "Wrap").fields[0].inline.is_none());
}

#[test]
fn an_object_stays_within_the_slot_operand() {
    let fields: String = (0..130).map(|i| format!("    var f{i}: Int\n")).collect();
    let src = format!("struct Wide {{\n{fields}}}\nstruct Pair {{\n    var a: Wide\n    var b: Wide\n}}\n");
    let pair = layout(&src, "Pair");
    assert_eq!(pair.slots, 131);
    assert!(pair.fields[0].inline.is_some() && pair.fields[1].inline.is_none());
}

#[test]
fn the_layout_survives_the_bytes() {
    let src = format!("{V2}struct Seg {{\n    var a: V2\n    var b: V2\n}}\n\nfn main() {{\n}}\n");
    let program = Compiler::compile(&src, "main.mote").unwrap();
    let back = compiler::CompiledProgram::from_bytes(&program.to_bytes()).unwrap();
    let seg = back.type_descriptors.iter().find(|t| t.name.as_deref() == Some("Seg")).unwrap();
    assert_eq!(seg.slots, 4);
    assert_eq!(seg.fields[1].slot, 2);
    assert_eq!(seg.fields[1].inline.as_ref().map(|d| d.slots), Some(2));
}
