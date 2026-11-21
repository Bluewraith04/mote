use std::mem::{offset_of, size_of};

use crate::kind::TypeId;
use crate::passing::{passing, Passing};
use crate::scalar::Scalar;
use crate::table::TypeTable;

/// The machine type of the task context, the first parameter of every function and closure code pointer.
pub const CONTEXT: Scalar = Scalar::I64;

/// The block the runtime keeps for each task and compiled code reads through the context pointer.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TaskContext {
    /// Where the next inline allocation starts.
    pub region_cursor: u64,
    /// The first address past the allocation region.
    pub region_limit: u64,
    /// Not zero when the task must enter the runtime at its next poll.
    pub poll: u64,
    /// The lowest address the stack may reach before the prologue check enters the runtime.
    pub stack_limit: u64,
    /// The raw id of the fault site that stopped the task, plus one; zero while none has.
    pub fault_site: u64,
    /// A word only the runtime reads: its record of the task.
    pub runtime: u64,
}

/// Where each field of the context sits.
pub const CONTEXT_REGION_CURSOR: u32 = offset_of!(TaskContext, region_cursor) as u32;
pub const CONTEXT_REGION_LIMIT: u32 = offset_of!(TaskContext, region_limit) as u32;
pub const CONTEXT_POLL: u32 = offset_of!(TaskContext, poll) as u32;
pub const CONTEXT_STACK_LIMIT: u32 = offset_of!(TaskContext, stack_limit) as u32;
pub const CONTEXT_FAULT_SITE: u32 = offset_of!(TaskContext, fault_site) as u32;
pub const CONTEXT_RUNTIME: u32 = offset_of!(TaskContext, runtime) as u32;

/// Bytes of the context.
pub const CONTEXT_SIZE: u32 = size_of::<TaskContext>() as u32;

/// The fields of the context by name and offset, in order.
pub const CONTEXT_FIELDS: [(&str, u32); 6] = [
    ("region_cursor", CONTEXT_REGION_CURSOR),
    ("region_limit", CONTEXT_REGION_LIMIT),
    ("poll", CONTEXT_POLL),
    ("stack_limit", CONTEXT_STACK_LIMIT),
    ("fault_site", CONTEXT_FAULT_SITE),
    ("runtime", CONTEXT_RUNTIME),
];

/// The machine values of one function: parameters in order, then results.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub params: Vec<Scalar>,
    pub results: Vec<Scalar>,
    /// The result goes through a slot the caller provides, whose address is the parameter after the context.
    pub result_in_memory: bool,
    /// For each declared argument, whether it goes through a slot the caller provides.
    pub args_in_memory: Vec<bool>,
}

/// A direct call: the context, the result slot when the result is in memory, then the arguments.
pub fn signature(table: &TypeTable, args: &[TypeId], result: TypeId) -> Signature {
    lower(table, None, args, result)
}

/// A call through a closure: the closure reference comes after the context and the result slot, then the arguments.
pub fn closure_signature(table: &TypeTable, args: &[TypeId], result: TypeId) -> Signature {
    lower(table, Some(Scalar::Ref), args, result)
}

fn lower(table: &TypeTable, environment: Option<Scalar>, args: &[TypeId], result: TypeId) -> Signature {
    let mut params = vec![CONTEXT];
    let (results, result_in_memory) = match passing(table, result) {
        Passing::Registers(scalars) => (scalars, false),
        Passing::Memory => {
            params.push(Scalar::I64);
            (Vec::new(), true)
        }
    };
    params.extend(environment);
    let mut args_in_memory = Vec::new();
    for &arg in args {
        match passing(table, arg) {
            Passing::Registers(scalars) => {
                params.extend(scalars);
                args_in_memory.push(false);
            }
            Passing::Memory => {
                params.push(Scalar::I64);
                args_in_memory.push(true);
            }
        }
    }
    Signature { params, results, result_in_memory, args_in_memory }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::Kind;

    fn table() -> (TypeTable, TypeId, TypeId, TypeId) {
        let mut table = TypeTable::new();
        let int = table.add("Int", Kind::Scalar(Scalar::I64));
        let float = table.add("Float", Kind::Scalar(Scalar::F64));
        let unit = table.add("Null", Kind::Unit);
        (table, int, float, unit)
    }

    #[test]
    fn callconv_the_context_is_six_words_with_the_region_first() {
        assert_eq!(CONTEXT_SIZE, 48);
        assert_eq!(
            [CONTEXT_REGION_CURSOR, CONTEXT_REGION_LIMIT, CONTEXT_POLL, CONTEXT_STACK_LIMIT, CONTEXT_FAULT_SITE, CONTEXT_RUNTIME],
            [0, 8, 16, 24, 32, 40]
        );
        assert_eq!(TaskContext::default(), TaskContext { region_cursor: 0, region_limit: 0, poll: 0, stack_limit: 0, fault_site: 0, runtime: 0 });
    }

    #[test]
    fn callconv_the_field_list_names_each_field_at_its_offset_in_order() {
        let offsets: Vec<u32> = CONTEXT_FIELDS.iter().map(|(_, offset)| *offset).collect();
        assert_eq!(offsets, [0, 8, 16, 24, 32, 40]);
        let names: Vec<&str> = CONTEXT_FIELDS.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["region_cursor", "region_limit", "poll", "stack_limit", "fault_site", "runtime"]);
        assert!(offsets.windows(2).all(|pair| pair[1] - pair[0] == 8));
        assert_eq!(offsets.last().unwrap() + 8, CONTEXT_SIZE);
    }

    #[test]
    fn callconv_the_context_is_the_first_parameter_and_is_not_a_reference() {
        let (table, _, _, unit) = table();
        let sig = signature(&table, &[], unit);
        assert_eq!(sig.params, [CONTEXT]);
        assert!(!CONTEXT.is_reference());
        assert!(sig.results.is_empty());
    }

    #[test]
    fn callconv_arguments_follow_the_context_leaf_by_leaf() {
        let (mut table, int, float, _) = table();
        let point = table.add("Point", Kind::Struct(vec![float, float]));
        let sig = signature(&table, &[int, point], float);
        assert_eq!(sig.params, [CONTEXT, Scalar::I64, Scalar::F64, Scalar::F64]);
        assert_eq!(sig.results, [Scalar::F64]);
        assert_eq!(sig.args_in_memory, [false, false]);
    }

    #[test]
    fn callconv_a_result_in_registers_returns_each_leaf() {
        let (mut table, int, float, _) = table();
        let pair = table.add("Pair", Kind::Struct(vec![int, float]));
        let sig = signature(&table, &[], pair);
        assert_eq!(sig.results, [Scalar::I64, Scalar::F64]);
        assert!(!sig.result_in_memory);
    }

    #[test]
    fn callconv_past_four_leaves_an_argument_goes_through_a_slot() {
        let (mut table, int, _, unit) = table();
        let five = table.add("Five", Kind::Struct(vec![int; 5]));
        let sig = signature(&table, &[five, int], unit);
        assert_eq!(sig.params, [CONTEXT, Scalar::I64, Scalar::I64]);
        assert_eq!(sig.args_in_memory, [true, false]);
    }

    #[test]
    fn callconv_past_four_leaves_a_result_goes_through_a_slot_after_the_context() {
        let (mut table, int, _, _) = table();
        let five = table.add("Five", Kind::Struct(vec![int; 5]));
        let sig = signature(&table, &[int], five);
        assert_eq!(sig.params, [CONTEXT, Scalar::I64, Scalar::I64]);
        assert!(sig.results.is_empty());
        assert!(sig.result_in_memory);
    }

    #[test]
    fn callconv_a_closure_call_passes_the_closure_after_the_result_slot() {
        let (mut table, int, _, _) = table();
        let five = table.add("Five", Kind::Struct(vec![int; 5]));
        let direct = signature(&table, &[int], five);
        let through = closure_signature(&table, &[int], five);
        assert_eq!(direct.params, [CONTEXT, Scalar::I64, Scalar::I64]);
        assert_eq!(through.params, [CONTEXT, Scalar::I64, Scalar::Ref, Scalar::I64]);
        let plain = closure_signature(&table, &[int], int);
        assert_eq!(plain.params, [CONTEXT, Scalar::Ref, Scalar::I64]);
    }

    #[test]
    fn callconv_an_enum_travels_as_words_and_an_optional_object_as_one_reference() {
        let (mut table, int, _, _) = table();
        let string = table.add("String", Kind::String);
        let maybe = table.add("String?", Kind::Optional(string));
        let shape = table.add("Shape", Kind::Enum(vec![vec![int], vec![int, int]]));
        let sig = signature(&table, &[maybe, shape], int);
        assert_eq!(sig.params, [CONTEXT, Scalar::Ref, Scalar::I64, Scalar::I64, Scalar::I64]);
    }
}
