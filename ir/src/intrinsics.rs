//! The operations the runtime provides, one row per `@intrinsic` name in `std/src`.

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
    row("typeof", 1, PURE),
    row("fault", 1, FAULTS),
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
    // reflection
    row("type_id", 0, PURE),
    row("meta_ints", 0, PURE),
    row("meta_texts", 0, PURE),
    row("dyn_of", 2, PURE),
    row("dyn_type", 1, READS),
    row("dyn_to", 2, READS),
    row("dyn_len", 1, READS),
    row("dyn_tag", 1, READS),
    row("dyn_get", 2, READS | FAULTS),
    row("dyn_set", 3, WRITES | FAULTS),
    row("dyn_show", 2, READS),
    row("require", 2, FAULTS),
    row("dyn_build", 3, READS | FAULTS),
    row("dyn_make", 3, READS | FAULTS),
];

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashSet};

    use mote_semantics::compile;

    use super::*;

    /// Each intrinsic name in `std/src` with its parameter counts and whether a declaration has a `var` parameter.
    fn declared() -> BTreeMap<String, (HashSet<usize>, bool)> {
        let compiled = compile("");
        assert_eq!(compiled.errors, Vec::<String>::new());
        let mut found: BTreeMap<String, (HashSet<usize>, bool)> = BTreeMap::new();
        for (_, def) in compiled.program.defs.iter() {
            let Some(function) = def.function() else { continue };
            let Some(name) = function.intrinsic else { continue };
            let entry = found.entry(compiled.interner.resolve(name).to_string()).or_default();
            entry.0.insert(function.params.len());
            entry.1 |= function.params.iter().any(|param| param.is_var);
        }
        found
    }

    #[test]
    fn the_rows_that_park_are_the_ones_semantics_lists() {
        let mut parks: Vec<&str> = TABLE.iter().filter(|row| row.parks()).map(|row| row.name).collect();
        let mut listed = mote_semantics::PARKING_INTRINSICS.to_vec();
        parks.sort_unstable();
        listed.sort_unstable();
        assert_eq!(parks, listed);
    }

    #[test]
    fn every_intrinsic_in_std_is_in_the_table_and_every_row_is_used() {
        let declared = declared();
        let names: Vec<&str> = declared.keys().map(String::as_str).collect();
        let rows: Vec<&str> = TABLE.iter().map(|row| row.name).collect();
        let mut sorted = rows.clone();
        sorted.sort_unstable();
        let mut expected = names.clone();
        expected.sort_unstable();
        assert_eq!(sorted, expected);
    }

    #[test]
    fn a_row_names_its_parameter_count_and_a_var_parameter_writes() {
        for (name, (counts, has_var)) in declared() {
            let row = lookup(&name).unwrap_or_else(|| panic!("no row for {name}"));
            assert_eq!(counts, HashSet::from([row.params]), "{name}");
            assert!(!has_var || row.writes(), "{name} has a var parameter and does not write");
        }
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
