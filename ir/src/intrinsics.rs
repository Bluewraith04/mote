//! Compiler intrinsic operations backing `@name(...)` builtins.

/// What a call can do besides compute its result; the flags combine with `|`.
pub mod effect {
    /// Depends on its arguments only; may be dropped, merged or moved.
    pub const PURE: u8 = 0;
    /// Reads the state of an object it is given, or of the task world.
    pub const READS: u8 = 1;
    /// Changes the state of an object it is given.
    pub const WRITES: u8 = 2;
    /// Stops the program with a fault for some arguments.
    pub const FAULTS: u8 = 4;
    /// May suspend the calling task.
    pub const PARKS: u8 = 8;
    /// Writes output.
    pub const IO: u8 = 16;
}

use effect::{FAULTS, IO, PARKS, PURE, READS, WRITES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Intrinsic {
    pub name: &'static str,
    /// The number of arguments; a `var` argument is also a result.
    pub params: usize,
    pub effects: u8,
}

impl Intrinsic {
    pub fn parks(&self) -> bool {
        self.effects & PARKS != 0
    }

    pub fn faults(&self) -> bool {
        self.effects & FAULTS != 0
    }

    pub fn writes(&self) -> bool {
        self.effects & WRITES != 0
    }

    /// Whether a call whose result is unused can be dropped.
    pub fn removable(&self) -> bool {
        self.effects & (WRITES | FAULTS | PARKS | IO) == 0
    }
}

const fn row(name: &'static str, params: usize, effects: u8) -> Intrinsic {
    Intrinsic { name, params, effects }
}

/// The intrinsic with the name.
pub fn lookup(name: &str) -> Option<&'static Intrinsic> {
    TABLE.iter().find(|intrinsic| intrinsic.name == name)
}

pub static TABLE: &[Intrinsic] = &[
    // io, text conversion, faults
    row("print", 1, IO),
    row("println", 1, IO),
    row("eprint", 1, IO),
    row("eprintln", 1, IO),
    row("sys_read_line", 0, IO),
    row("sys_args", 0, READS),
    row("sys_exit", 1, IO),
    row("sys_now", 0, IO),
    row("type_tag", 1, PURE),
    row("fault", 1, FAULTS),
    row("index_out_of_range", 0, FAULTS),
    row("port_open", 1, WRITES),
    // numbers
    row("float_is_nan", 1, PURE),
    row("float_is_infinite", 1, PURE),
    row("float_to_int", 1, PURE),
    row("num_to_string", 1, PURE),
    row("num_wrapping_add", 2, PURE),
    row("num_wrapping_sub", 2, PURE),
    row("num_wrapping_mul", 2, PURE),
    row("num_wrapping_neg", 1, PURE),
    row("num_wrapping_shl", 2, PURE),
    row("num_wrapping_shr", 2, PURE),
    row("num_checked_add", 2, PURE),
    row("num_checked_sub", 2, PURE),
    row("num_checked_mul", 2, PURE),
    row("num_checked_neg", 1, PURE),
    row("num_checked_shl", 2, PURE),
    row("num_checked_shr", 2, PURE),
    row("num_saturating_add", 2, PURE),
    row("num_saturating_sub", 2, PURE),
    row("num_saturating_mul", 2, PURE),
    row("num_saturating_neg", 1, PURE),
    row("num_saturating_shl", 2, PURE),
    row("num_saturating_shr", 2, PURE),
    row("num_overflowing_add", 2, PURE),
    row("num_overflowing_sub", 2, PURE),
    row("num_overflowing_mul", 2, PURE),
    row("num_overflowing_neg", 1, PURE),
    row("num_overflowing_shl", 2, PURE),
    row("num_overflowing_shr", 2, PURE),
    row("math_sqrt", 1, PURE),
    row("math_floor", 1, PURE),
    row("math_ceil", 1, PURE),
    row("math_round", 1, PURE),
    row("math_pow", 2, PURE),
    row("math_exp", 1, PURE),
    row("math_ln", 1, PURE),
    row("math_log10", 1, PURE),
    row("math_sin", 1, PURE),
    row("math_cos", 1, PURE),
    row("math_tan", 1, PURE),
    row("math_atan2", 2, PURE),
    // text
    row("parse_int", 1, PURE),
    row("parse_float", 1, PURE),
    row("string_len", 1, PURE),
    row("string_concat", 2, PURE),
    row("string_replace", 3, PURE),
    row("string_slice", 3, FAULTS),
    row("string_byte_at", 2, FAULTS),
    row("string_is_boundary", 2, PURE),
    row("string_to_upper", 1, PURE),
    row("string_to_lower", 1, PURE),
    row("string_trim", 1, PURE),
    row("string_repeat", 2, FAULTS),
    row("string_find", 2, PURE),
    row("string_rfind", 2, PURE),
    row("string_split", 2, PURE),
    row("string_bytes", 1, PURE),
    row("string_from_bytes", 1, PURE),
    // lists
    row("list_new", 0, PURE),
    row("list_len", 1, READS),
    row("list_push", 2, WRITES),
    row("list_pop", 1, WRITES),
    row("list_get", 2, READS | FAULTS),
    row("list_set", 3, WRITES | FAULTS),
    row("list_clear", 1, WRITES),
    row("list_contains", 2, READS),
    row("list_slice", 3, READS | FAULTS),
    row("list_set_slice", 4, WRITES | FAULTS),
    row("list_clone", 1, READS),
    row("deep_clone", 1, READS),
    row("hash_of", 1, PURE),
    row("key_eq", 2, PURE),
    row("equal_of", 2, READS),
    // arrays
    row("array_len", 1, PURE),
    row("array_get", 2, READS | FAULTS),
    row("array_set", 3, WRITES | FAULTS),
    row("array_contains", 2, READS),
    row("array_slice", 3, READS | FAULTS),
    row("array_clone", 1, READS),
    row("mask_all", 1, PURE),
    row("mask_any", 1, PURE),
    row("mask_none", 1, PURE),
    row("array_all_by", 2, READS),
    row("array_any_by", 2, READS),
    row("array_none_by", 2, READS),
    row("array_count", 2, READS),
    row("array_count_by", 2, READS),
    row("array_sum", 1, PURE),
    row("array_product", 1, PURE),
    row("array_mean", 1, PURE),
    row("array_min", 1, READS),
    row("array_max", 1, READS),
    row("array_argmin", 1, READS),
    row("array_argmax", 1, READS),
    row("array_dot", 2, READS | FAULTS),
    row("array_norm_sq", 1, PURE),
    row("array_fold", 3, READS),
    row("array_reduce", 2, READS),
    row("array_scan", 3, READS),
    row("array_find", 2, READS),
    row("array_find_index", 2, READS),
    row("array_index_of", 2, READS),
    row("array_select", 3, READS | FAULTS),
    row("array_map", 2, READS),
    row("array_zip_with", 3, READS),
    row("array_reverse", 1, READS),
    row("array_clamp", 3, READS),
    row("array_filter", 2, READS),
    row("array_zip", 2, READS),
    row("array_enumerate", 1, READS),
    row("array_concat", 2, READS),
    row("array_take", 2, READS),
    row("array_drop", 2, READS),
    row("array_flatten", 1, READS),
    row("array_sort", 1, READS),
    row("array_sort_by", 2, READS),
    // streams and tasks
    row("stream_next", 1, WRITES | PARKS),
    row("task_await", 1, PARKS),
    row("task_await_timeout", 2, PARKS),
    row("task_cancel", 1, WRITES),
    row("task_is_ready", 1, READS),
    row("spawn_call", 1, WRITES),
    row("task_view", 2, READS),
    row("task_value", 1, READS),
    row("sleep", 1, PARKS),
    // channels
    row("channel_new", 1, FAULTS),
    row("sender_send", 2, WRITES | PARKS),
    row("sender_try_send", 2, WRITES),
    row("sender_close", 1, WRITES),
    row("sender_clone", 1, WRITES),
    row("receiver_recv", 1, WRITES | PARKS),
    row("receiver_recv_timeout", 2, WRITES | PARKS),
    row("receiver_try_recv", 1, WRITES),
    row("receiver_is_closed", 1, READS),
    row("receiver_close", 1, WRITES),
    row("receiver_clone", 1, WRITES),
    // mutexes, onces, atomics
    row("mutex_new", 1, PURE),
    row("mutex_take", 1, WRITES | FAULTS | PARKS),
    row("mutex_put", 2, WRITES | FAULTS),
    row("once_new", 0, PURE),
    row("once_begin", 1, WRITES | FAULTS | PARKS),
    row("once_finish", 2, WRITES),
    row("once_value", 1, READS | FAULTS),
    row("atomic_int_new", 1, PURE),
    row("atomic_int_load", 1, READS),
    row("atomic_int_store", 2, WRITES),
    row("atomic_int_swap", 2, WRITES),
    row("atomic_int_compare_exchange", 3, WRITES),
    row("atomic_int_fetch_add", 2, WRITES | FAULTS),
    row("atomic_int_fetch_sub", 2, WRITES | FAULTS),
    row("atomic_bool_new", 1, PURE),
    row("atomic_bool_load", 1, READS),
    row("atomic_bool_store", 2, WRITES),
    row("atomic_bool_swap", 2, WRITES),
    row("atomic_bool_compare_exchange", 3, WRITES),
    // pointers / off-heap memory
    row("ptr_null", 0, PURE),
    row("ptr_from_addr", 1, PURE),
    row("ptr_alloc", 1, WRITES | FAULTS),
    row("ptr_realloc", 2, WRITES | FAULTS),
    row("ptr_free", 1, WRITES),
    row("ptr_offset", 2, FAULTS),
    row("ptr_load", 2, READS | FAULTS),
    row("ptr_store", 3, WRITES | FAULTS),
    row("ptr_copy", 3, WRITES | FAULTS),
    row("ptr_zero", 2, WRITES | FAULTS),
    row("ptr_cast", 1, PURE),
];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn the_rows_that_park_are_the_ones_semantics_lists() {
        let mut parks: Vec<&str> = TABLE.iter().filter(|row| row.parks()).map(|row| row.name).collect();
        let mut listed: Vec<&str> = mote_semantics::PARKING_BUILTINS.iter().map(|s| s.strip_prefix("__").unwrap_or(s)).collect();
        parks.sort_unstable();
        listed.sort_unstable();
        assert_eq!(parks, listed);
    }

    #[test]
    fn every_runtime_builtin_in_semantics_is_in_the_table_and_every_row_is_used() {
        let rows: Vec<&str> = TABLE.iter().map(|row| row.name).collect();
        let mut sorted = rows.clone();
        sorted.sort_unstable();
        let mut expected = mote_semantics::builtins::RUNTIME_INTRINSIC_NAMES.to_vec();
        expected.sort_unstable();
        assert_eq!(sorted, expected);
    }

    #[test]
    fn no_name_is_listed_twice() {
        let names: HashSet<&str> = TABLE.iter().map(|row| row.name).collect();
        assert_eq!(names.len(), TABLE.len());
    }

    #[test]
    fn a_call_that_parks_or_faults_cannot_be_dropped() {
        assert!(lookup("math_sqrt").is_some_and(Intrinsic::removable));
        assert!(!lookup("receiver_recv").is_some_and(Intrinsic::removable));
        assert!(!lookup("list_get").is_some_and(Intrinsic::removable));
        assert!(lookup("array_len").is_some_and(|row| !row.parks() && !row.faults()));
    }
}
