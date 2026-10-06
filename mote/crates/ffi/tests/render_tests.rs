//! `builtins::render` — the `print` / `println` display form.

use std::ptr::NonNull;

use ffi::builtins::render;
use isa::value::{ObjectHeader, TypeDescriptor, Value};

#[repr(C)]
struct FakeObject<const N: usize> {
    header: ObjectHeader,
    fields: [Value; N],
}

fn td(name: &str, field_names: &[Option<&str>]) -> Box<TypeDescriptor> {
    let mut desc = TypeDescriptor::new_value_type(1, field_names.iter().map(|n| n.map(String::from)).collect(), true);
    desc.name = Some(name.to_string());
    Box::new(desc)
}

fn boxed<const N: usize>(desc: &mut TypeDescriptor, fields: [Value; N]) -> (Box<FakeObject<N>>, Value) {
    let mut obj = Box::new(FakeObject {
        header: ObjectHeader {
            type_ptr: NonNull::from(&*desc),
            gc_state: std::sync::atomic::AtomicUsize::new(0),
        },
        fields,
    });
    let v = Value::boxed(NonNull::from(&mut obj.header));
    (obj, v)
}

#[test]
fn struct_with_named_fields() {
    let mut d = td("Point", &[Some("x"), Some("y")]);
    let (_keep, v) = boxed(&mut d, [Value::int(3), Value::int(7)]);
    assert_eq!(render(&v), "Point { x: 3, y: 7 }");
}

#[test]
fn tuple_variant_with_positional_fields() {
    let mut d = td("Circle", &[None]);
    let (_keep, v) = boxed(&mut d, [Value::int(5)]);
    assert_eq!(render(&v), "Circle(5)");
}

#[test]
fn unit_variant_is_just_the_name() {
    let mut d = td("None", &[]);
    let (_keep, v) = boxed(&mut d, []);
    assert_eq!(render(&v), "None");
}

#[test]
fn nested_objects_recurse() {
    let mut inner = td("Inner", &[Some("n")]);
    let (_i, iv) = boxed(&mut inner, [Value::int(9)]);
    let mut outer = td("Outer", &[Some("inner"), Some("flag")]);
    let (_o, ov) = boxed(&mut outer, [iv, Value::true_()]);
    assert_eq!(render(&ov), "Outer { inner: Inner { n: 9 }, flag: true }");
}

#[test]
fn primitives_are_unchanged() {
    assert_eq!(render(&Value::int(42)), "42");
    assert_eq!(render(&Value::true_()), "true");
    assert_eq!(render(&Value::null()), "null");
}
