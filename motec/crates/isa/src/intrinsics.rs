//! Intrinsic ids and the slot layouts of scheduler objects: the compiler↔runtime contract for `CALLINTRINSIC`.

/// Intrinsic ids: an index into [`INTRINSIC_NAMES`]. `CALLINTRINSIC` carries one; they share no space with natives.
pub const TASK_JOIN_INTRINSIC: u8 = 0;
pub const TASK_CANCEL_INTRINSIC: u8 = 1;
pub const CHANNEL_NEW_INTRINSIC: u8 = 2;
pub const CHANNEL_SEND_INTRINSIC: u8 = 3;
pub const CHANNEL_RECV_INTRINSIC: u8 = 4;
pub const CHANNEL_CLOSE_INTRINSIC: u8 = 5;
/// `std.task.any`: waits on every handle in a `List<Task<T>>` at once.
pub const TASK_ANY_INTRINSIC: u8 = 6;
/// `Shared(v)`: a cell holding `v` sealed; the second argument is a `SEAL_*` mode.
pub const SHARED_NEW_INTRINSIC: u8 = 7;
/// `s.get()`: the current version; never waits.
pub const SHARED_GET_INTRINSIC: u8 = 8;
/// Takes the writer turn; returns a mutable copy of the version when the second argument is `1`.
pub const SHARED_BEGIN_INTRINSIC: u8 = 9;
/// Commits the value (sealed by the `SEAL_*` mode in the third argument) and gives up the turn.
pub const SHARED_COMMIT_INTRINSIC: u8 = 10;
/// `wait_until`'s step: the current version, parking until a commit while it is the second argument (unless the third is `1`).
pub const SHARED_WAIT_INTRINSIC: u8 = 11;
/// `t.is_ready()`: whether the task has finished; never waits.
pub const TASK_IS_READY_INTRINSIC: u8 = 12;
/// `Channel(n)`'s second step: the first `Sender` handle of a channel, owned by the calling task.
pub const CHANNEL_SENDER_INTRINSIC: u8 = 13;
/// `tx.clone()`: another `Sender` of the same channel, owned by the calling task.
pub const SENDER_CLONE_INTRINSIC: u8 = 14;
/// `std.task.pin` / `unpin` / `is_pinned`: the argument is a `PIN_*` mode; the answer is whether the task is pinned afterwards.
pub const TASK_PIN_INTRINSIC: u8 = 15;

/// `TASK_PIN_INTRINSIC` modes.
pub const PIN_OFF: i64 = 0;
pub const PIN_ON: i64 = 1;
pub const PIN_QUERY: i64 = 2;

/// How a value becomes a sealed version.
pub const SEAL_COPY: i64 = 0;
/// Sealed in place: a fresh or moved value.
pub const SEAL_IN_PLACE: i64 = 1;
/// A working copy after `update`: its own objects are sealed in place, anything else is copied.
pub const SEAL_CLAIM: i64 = 2;

/// Intrinsic names, indexed by id.
pub const INTRINSIC_NAMES: [&str; 16] = [
    "task.join",
    "task.cancel",
    "channel.new",
    "channel.send",
    "channel.recv",
    "channel.close",
    "task.any",
    "shared.new",
    "shared.get",
    "shared.begin",
    "shared.commit",
    "shared.wait",
    "task.is_ready",
    "channel.sender",
    "sender.clone",
    "task.pin",
];

/// The id of the intrinsic named `name`.
pub fn intrinsic_id(name: &str) -> Option<u8> {
    INTRINSIC_NAMES.iter().position(|n| *n == name).map(|i| i as u8)
}

/// `Task<T>` header slots ([`isa::value::TASK_TYPE_ID`]'s doc comment).
pub const TASK_SLOT_ID: usize = 0;
pub const TASK_SLOT_STATUS: usize = 1;
pub const TASK_SLOT_RESULT: usize = 2;
/// `null` when no task is parked on `join`, else the waiting task's id.
/// Not `uint(0)`: the main task's id is `0`.
pub const TASK_SLOT_WAITER: usize = 3;
/// Set to `1` by `join` once it has handed a resolved status to a caller; `SCOPEEXIT` uses it to tell an observed child fault from an unobserved one.
pub const TASK_SLOT_OBSERVED: usize = 4;

/// `TASK_SLOT_STATUS` values.
pub const STATUS_PENDING: i64 = 0;
pub const STATUS_OK: i64 = 1;
pub const STATUS_ERR: i64 = 2;

/// Channel header slots ([`isa::value::CHANNEL_TYPE_ID`]'s doc comment).
pub const CHAN_SLOT_ID: usize = 0;
pub const CHAN_SLOT_CAPACITY: usize = 1;
pub const CHAN_SLOT_HEAD: usize = 2;
pub const CHAN_SLOT_LEN: usize = 3;
pub const CHAN_SLOT_CLOSED: usize = 4;
pub const CHAN_SLOT_SENDERS: usize = 5;
pub const CHAN_SLOT_TAKEN: usize = 6;
pub const CHAN_SLOT_BACKING: usize = 7;
pub const CHAN_SLOT_RENDEZVOUS: usize = 8;

/// `Sender<T>` handle slots ([`isa::value::SENDER_TYPE_ID`]'s doc comment).
pub const SENDER_SLOT_CHANNEL: usize = 0;
pub const SENDER_SLOT_CLOSED: usize = 1;

/// The status `recv` leaves in the register after its receiver argument.
pub const RECV_EMPTY: i64 = 0;
pub const RECV_GOT_VALUE: i64 = 1;

/// `Shared<T>` header slots ([`isa::value::SHARED_TYPE_ID`]'s doc comment).
pub const SHARED_SLOT_ID: usize = 0;
pub const SHARED_SLOT_VERSION: usize = 1;
pub const SHARED_SLOT_HOLDER: usize = 2;

