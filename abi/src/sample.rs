use crate::handle::HandleKind;
use crate::kind::Kind;
use crate::reflect::Reflection;
use crate::scalar::Scalar;
use crate::table::TypeTable;
use crate::table_bytes::TypeImage;

/// A table with every kind, a recursive class, a recursive enum and a reflection section.
pub(crate) fn everything() -> TypeImage {
    let mut table = TypeTable::new();
    table.add("Null", Kind::Unit);
    let byte = table.add("Int8", Kind::Scalar(Scalar::I8));
    let int = table.add("Int", Kind::Scalar(Scalar::I64));
    let float = table.add("Float", Kind::Scalar(Scalar::F64));
    let string = table.add("String", Kind::String);
    let point = table.add("Point", Kind::Struct(vec![float, float]));
    table.add("Int?", Kind::Optional(int));
    table.add("String?", Kind::Optional(string));
    let union = table.add("Int | String", Kind::Pair);
    table.add("[Point; 3]", Kind::FixedArray(point, 3));
    let shape = table.add("Shape", Kind::Enum(vec![vec![int], vec![point, byte], vec![]]));
    let node = table.reserve_object("Node");
    let next = table.add("Node?", Kind::Optional(node));
    table.define(node, Kind::Class(vec![int, next, union]));
    let child = table.reserve_object("Box<Tree>");
    let tree = table.add("Tree", Kind::Enum(vec![vec![int], vec![child, child]]));
    table.define(child, Kind::Box(tree));
    table.add("[Int]", Kind::Array(int));
    table.add("List<String>", Kind::List(string));
    table.add("fn(Int) -> Int", Kind::Closure(vec![int, string]));
    let buffer = table.add("[String]", Kind::Array(string));
    table.add("Channel<String>", Kind::Handle(HandleKind::Channel, vec![buffer]));
    table.add("Sender<String>", Kind::Handle(HandleKind::Sender, vec![]));
    table.add("Mutex<Shape>", Kind::Handle(HandleKind::Mutex, vec![shape]));
    table.add("Atomic", Kind::Handle(HandleKind::AtomicInt, vec![]));
    let reflection = Reflection { ints: vec![3, 0, -7], texts: vec![String::new(), "Point".into()] };
    TypeImage { table, reflection }
}
