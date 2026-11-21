use crate::kind::{Kind, TypeId};
use crate::scalar::Scalar;
use crate::table::TypeTable;

/// Most scalars a value may flatten to and still travel in registers.
pub const MAX_PASSED_SCALARS: usize = 4;

/// How a value crosses a call, as an argument or a result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Passing {
    /// One scalar per register, in order; empty for a value with no room.
    Registers(Vec<Scalar>),
    /// Through a caller-provided stack slot.
    Memory,
}

/// Flattens structs and fixed arrays leaf by leaf, and enums and pairs into 8-byte words.
pub fn passing(table: &TypeTable, id: TypeId) -> Passing {
    let mut scalars = Vec::new();
    if collect(table, id, &mut scalars) { Passing::Registers(scalars) } else { Passing::Memory }
}

fn collect(table: &TypeTable, id: TypeId, out: &mut Vec<Scalar>) -> bool {
    let info = table.get(id);
    match &info.kind {
        Kind::Unit => true,
        Kind::Scalar(scalar) => push(out, *scalar),
        Kind::Struct(_) => info.fields.iter().all(|field| collect(table, field.ty, out)),
        Kind::FixedArray(element, count) => {
            if table.layout(*element).size == 0 {
                return true;
            }
            (0..*count).all(|_| collect(table, *element, out))
        }
        _ if info.is_reference() => push(out, Scalar::Ref),
        _ => (0..info.layout.size.div_ceil(8)).all(|_| push(out, Scalar::I64)),
    }
}

fn push(out: &mut Vec<Scalar>, scalar: Scalar) -> bool {
    out.push(scalar);
    out.len() <= MAX_PASSED_SCALARS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(table: &mut TypeTable) -> TypeId {
        table.add("Int", Kind::Scalar(Scalar::I64))
    }

    #[test]
    fn scalars_and_references_take_one_register() {
        let mut table = TypeTable::new();
        let int = int(&mut table);
        let string = table.add("String", Kind::String);
        assert_eq!(passing(&table, int), Passing::Registers(vec![Scalar::I64]));
        assert_eq!(passing(&table, string), Passing::Registers(vec![Scalar::Ref]));
    }

    #[test]
    fn a_fixed_array_flattens_to_the_leaves_of_its_elements() {
        let mut table = TypeTable::new();
        let small = table.add("Int32", Kind::Scalar(Scalar::I32));
        let float = table.add("Float", Kind::Scalar(Scalar::F64));
        let point = table.add("Point", Kind::Struct(vec![small, float]));
        let numbers = table.add("[Int32; 3]", Kind::FixedArray(small, 3));
        let points = table.add("[Point; 2]", Kind::FixedArray(point, 2));
        assert_eq!(passing(&table, numbers), Passing::Registers(vec![Scalar::I32; 3]));
        assert_eq!(passing(&table, points), Passing::Registers(vec![Scalar::I32, Scalar::F64, Scalar::I32, Scalar::F64]));
    }

    #[test]
    fn a_value_with_no_room_takes_no_register() {
        let mut table = TypeTable::new();
        let unit = table.add("Null", Kind::Unit);
        assert_eq!(passing(&table, unit), Passing::Registers(vec![]));
    }

    #[test]
    fn a_struct_flattens_to_its_leaves() {
        let mut table = TypeTable::new();
        let float = table.add("Float", Kind::Scalar(Scalar::F64));
        let byte = table.add("Int8", Kind::Scalar(Scalar::I8));
        let point = table.add("Point", Kind::Struct(vec![float, float]));
        let mixed = table.add("Mixed", Kind::Struct(vec![byte, point]));
        assert_eq!(passing(&table, point), Passing::Registers(vec![Scalar::F64, Scalar::F64]));
        assert_eq!(passing(&table, mixed), Passing::Registers(vec![Scalar::I8, Scalar::F64, Scalar::F64]));
    }

    #[test]
    fn enums_optionals_and_pairs_travel_as_words() {
        let mut table = TypeTable::new();
        let int = int(&mut table);
        let maybe = table.add("Int?", Kind::Optional(int));
        let union = table.add("Int | String", Kind::Pair);
        let shape = table.add("Shape", Kind::Enum(vec![vec![int], vec![int, int]]));
        assert_eq!(passing(&table, maybe), Passing::Registers(vec![Scalar::I64; 2]));
        assert_eq!(passing(&table, union), Passing::Registers(vec![Scalar::I64; 2]));
        assert_eq!(passing(&table, shape), Passing::Registers(vec![Scalar::I64; 3]));
    }

    #[test]
    fn an_optional_object_is_one_reference() {
        let mut table = TypeTable::new();
        let string = table.add("String", Kind::String);
        let maybe = table.add("String?", Kind::Optional(string));
        assert_eq!(passing(&table, maybe), Passing::Registers(vec![Scalar::Ref]));
    }

    #[test]
    fn past_the_limit_a_value_goes_through_memory() {
        let mut table = TypeTable::new();
        let int = int(&mut table);
        let four = table.add("Four", Kind::Struct(vec![int; 4]));
        let five = table.add("Five", Kind::Struct(vec![int; 5]));
        let big = table.add("[Int; 100]", Kind::FixedArray(int, 100));
        assert_eq!(passing(&table, four), Passing::Registers(vec![Scalar::I64; 4]));
        assert_eq!(passing(&table, five), Passing::Memory);
        assert_eq!(passing(&table, big), Passing::Memory);
    }

    #[test]
    fn a_fixed_array_of_nothing_is_free() {
        let mut table = TypeTable::new();
        let unit = table.add("Null", Kind::Unit);
        let empty = table.add("[Null; 1000000]", Kind::FixedArray(unit, 1_000_000));
        assert_eq!(passing(&table, empty), Passing::Registers(vec![]));
    }
}
