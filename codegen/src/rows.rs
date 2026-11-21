//! Which rows of `ir::intrinsics::TABLE` the code generator emits, and which it leaves to tasks and handles.

/// The lowering that emits an intrinsic row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Emitter {
    List,
    Numeric,
    Text,
    Collect,
    Print,
    System,
    Task,
    Channel,
    Lock,
    Atomic,
    Stream,
    Mem,
}

/// The lowering that emits the row, if any.
pub(crate) fn emitter(name: &str) -> Option<Emitter> {
    Some(match name {
        "list_new" | "list_len" | "list_push" => Emitter::List,
        "array_len" | "list_get" | "array_get" | "list_set" | "array_set" | "list_clear" | "list_pop" | "list_slice" | "array_slice" | "list_clone" | "array_clone" | "list_set_slice" | "list_contains" | "array_contains" | "equal_of" | "key_eq" | "hash_of" | "deep_clone" | "fault" | "index_out_of_range" | "type_tag" => Emitter::Collect,
        "print" | "println" | "eprint" | "eprintln" => Emitter::Print,
        _ if name.starts_with("sys_") => Emitter::System,
        "task_await" | "task_await_timeout" | "task_cancel" | "task_is_ready" | "task_view" | "task_value" | "spawn_call" | "sleep" => Emitter::Task,
        "channel_new" | "receiver_recv_timeout" | "sender_send" | "sender_try_send" | "sender_close" | "sender_clone" | "receiver_recv" | "receiver_try_recv" | "receiver_is_closed" | "receiver_close" | "receiver_clone" | "port_open" => Emitter::Channel,
        "mutex_new" | "mutex_take" | "mutex_put" | "once_new" | "once_begin" | "once_finish" | "once_value" => Emitter::Lock,
        _ if name.starts_with("atomic_") => Emitter::Atomic,
        _ if name.starts_with("ptr_") => Emitter::Mem,
        "stream_next" => Emitter::Stream,
        _ if name.starts_with("num_") || name.starts_with("math_") || name.starts_with("float_") => Emitter::Numeric,
        _ if name.starts_with("string_") || name.starts_with("parse_") => Emitter::Text,
        _ => return None,
    })
}

/// The rows that are lowered directly before codegen and are never called.
pub const FOLDED_ROWS: &[&str] = &[
    "mask_all",
    "mask_any",
    "mask_none",
    "array_all_by",
    "array_any_by",
    "array_none_by",
    "array_count",
    "array_count_by",
    "array_sum",
    "array_product",
    "array_mean",
    "array_min",
    "array_max",
    "array_argmin",
    "array_argmax",
    "array_dot",
    "array_norm_sq",
    "array_fold",
    "array_reduce",
    "array_scan",
    "array_find",
    "array_find_index",
    "array_index_of",
    "array_select",
    "array_map",
    "array_zip_with",
    "array_reverse",
    "array_clamp",
    "array_filter",
    "array_zip",
    "array_enumerate",
    "array_concat",
    "array_take",
    "array_drop",
    "array_flatten",
    "array_sort",
    "array_sort_by",
];

#[cfg(test)]
mod tests {
    use mote_ir::intrinsics::TABLE;

    use super::*;

    #[test]
    fn table_complete() {
        let mut emitted = Vec::new();
        let mut left = Vec::new();
        for row in TABLE {
            match (emitter(row.name), FOLDED_ROWS.contains(&row.name)) {
                (Some(_), false) => emitted.push(row.name),
                (None, true) => left.push(row.name),
                other => panic!("the row `{}` is in {other:?} of (emitted, folded)", row.name),
            }
        }
        println!("emitted: {}", emitted.len());
        println!("folded: {}", left.join(" "));
        assert_eq!(emitted.len() + left.len(), TABLE.len());
        for name in FOLDED_ROWS {
            assert!(TABLE.iter().any(|row| row.name == *name), "`{name}` is not a row of the table");
        }
    }
}
