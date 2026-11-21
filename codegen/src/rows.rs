//! Which rows of `ir::intrinsics::TABLE` the code generator emits, and which it leaves to tasks and handles.

/// The lowering that emits an intrinsic row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Emitter {
    List,
    Numeric,
    Text,
    Collect,
    Reflect,
    Print,
    System,
    Task,
    Channel,
    Lock,
    Atomic,
    Stream,
}

/// The lowering that emits the row, if any.
pub(crate) fn emitter(name: &str) -> Option<Emitter> {
    Some(match name {
        "list_new" | "list_len" | "list_push" => Emitter::List,
        "array_len" | "list_get" | "array_get" | "list_set" | "array_set" | "list_clear" | "list_pop" | "list_slice" | "array_slice" | "list_clone" | "array_clone" | "list_set_slice" | "list_contains" | "array_contains" | "equal_of" | "key_eq" | "hash_of" | "deep_clone" | "fault" | "typeof" => Emitter::Collect,
        "require" | "meta_ints" | "meta_texts" | "dyn_of" | "dyn_type" | "dyn_to" | "dyn_len" | "dyn_tag" | "dyn_get" | "dyn_set" | "dyn_show" | "dyn_build" | "dyn_make" => Emitter::Reflect,
        "print" | "println" | "eprint" | "eprintln" => Emitter::Print,
        _ if name.starts_with("sys_") => Emitter::System,
        "task_await" | "task_await_timeout" | "task_cancel" | "task_is_ready" | "task_view" | "task_value" | "spawn_call" | "sleep" => Emitter::Task,
        "channel_new" | "receiver_recv_timeout" | "sender_send" | "sender_try_send" | "sender_close" | "sender_clone" | "receiver_recv" | "receiver_try_recv" | "receiver_is_closed" | "receiver_close" | "receiver_clone" | "port_open" => Emitter::Channel,
        "mutex_new" | "mutex_take" | "mutex_put" | "once_new" | "once_begin" | "once_finish" | "once_value" => Emitter::Lock,
        _ if name.starts_with("atomic_") => Emitter::Atomic,
        "stream_next" => Emitter::Stream,
        _ if name.starts_with("num_") || name.starts_with("math_") || name.starts_with("float_") => Emitter::Numeric,
        _ if name.starts_with("string_") || name.starts_with("parse_") => Emitter::Text,
        _ => return None,
    })
}

/// The rows that are a constant in the lowered program and are never called.
pub const FOLDED_ROWS: &[&str] = &["type_id"];

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
