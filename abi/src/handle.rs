use crate::kind::Slot;
use crate::layout::{HEADER_SIZE, WORD};

/// The runtime objects a handle type stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HandleKind {
    /// A spawned task; the payload is its result.
    Task,
    /// The sending end of a channel.
    Sender,
    /// The receiving end of a channel.
    Receiver,
    /// The queue both ends share; the payload is the array that holds the messages.
    Channel,
    /// A lock around a value; the payload is that value.
    Mutex,
    /// A value set once; the payload is that value.
    Once,
    AtomicInt,
    AtomicBool,
    /// The external library side of a port.
    Port,
}

impl HandleKind {
    pub const ALL: [HandleKind; 9] = [
        HandleKind::Task,
        HandleKind::Sender,
        HandleKind::Receiver,
        HandleKind::Channel,
        HandleKind::Mutex,
        HandleKind::Once,
        HandleKind::AtomicInt,
        HandleKind::AtomicBool,
        HandleKind::Port,
    ];

    /// Bytes before the payload, header included; the payload starts here.
    pub fn fixed_size(self) -> u32 {
        match self {
            HandleKind::AtomicInt | HandleKind::AtomicBool | HandleKind::Port => HEADER_SIZE + WORD,
            HandleKind::Sender | HandleKind::Receiver | HandleKind::Mutex => HEADER_SIZE + 2 * WORD,
            HandleKind::Task | HandleKind::Once => HEADER_SIZE + 3 * WORD,
            HandleKind::Channel => HEADER_SIZE + 6 * WORD,
        }
    }

    /// The words before the payload, as `(name, offset)` in order.
    pub fn words(self) -> &'static [(&'static str, u32)] {
        match self {
            HandleKind::Task => &[("state", 8), ("runtime", 16), ("convert", TASK_CONVERT_OFFSET)],
            HandleKind::Sender | HandleKind::Receiver => &[("channel", 8), ("open", 16)],
            HandleKind::Channel => &[("senders", 8), ("receivers", 16), ("capacity", 24), ("head", 32), ("length", 40), ("waiters", 48)],
            HandleKind::Mutex => &[("owner", 8), ("waiters", 16)],
            HandleKind::Once => &[("state", 8), ("runner", 16), ("waiters", 24)],
            HandleKind::AtomicInt | HandleKind::AtomicBool => &[("value", 8)],
            HandleKind::Port => &[("port", 8)],
        }
    }

    /// The references among the words before the payload.
    pub fn slots(self) -> &'static [Slot] {
        match self {
            HandleKind::Sender | HandleKind::Receiver => &[Slot::Ref(8)],
            HandleKind::Task => &[Slot::Ref(TASK_CONVERT_OFFSET)],
            _ => &[],
        }
    }

    /// How many payload types a handle of this kind lists.
    pub fn payloads(self) -> usize {
        match self {
            HandleKind::Task | HandleKind::Channel | HandleKind::Mutex | HandleKind::Once => 1,
            _ => 0,
        }
    }
}

/// The offset of the word of a `Task` that holds the closure converting the result of a task it views, which is null for a task that was spawned.
pub const TASK_CONVERT_OFFSET: u32 = 24;
/// The offset of the word of a `Task` that holds the number of its record in the runtime.
pub const TASK_NUMBER_OFFSET: u32 = 16;

/// A task is running or has not started.
pub const TASK_PENDING: u64 = 0;
/// A task finished and its result is in the payload.
pub const TASK_DONE: u64 = 1;
/// A task was cancelled before it finished.
pub const TASK_CANCELLED: u64 = 2;
/// A task stopped at a fault.
pub const TASK_FAULTED: u64 = 3;

/// A `Once` nobody has begun.
pub const ONCE_EMPTY: u64 = 0;
/// A `Once` whose initializer is running.
pub const ONCE_RUNNING: u64 = 1;
/// A `Once` that holds its value.
pub const ONCE_DONE: u64 = 2;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::{Kind, TypeId};
    use crate::layout::Layout;
    use crate::scalar::Scalar;
    use crate::table::TypeTable;

    fn int(table: &mut TypeTable) -> TypeId {
        table.add("Int", Kind::Scalar(Scalar::I64))
    }

    #[test]
    fn handles_each_word_sits_after_the_header_in_order() {
        for kind in HandleKind::ALL {
            let mut at = HEADER_SIZE;
            for (_, offset) in kind.words() {
                assert_eq!(*offset, at, "{kind:?}");
                at += WORD;
            }
            assert_eq!(at, kind.fixed_size(), "{kind:?}");
        }
    }

    #[test]
    fn handles_the_payload_starts_after_the_words() {
        let mut table = TypeTable::new();
        let int = int(&mut table);
        let cases = [
            (HandleKind::Task, 32),
            (HandleKind::Mutex, 24),
            (HandleKind::Once, 32),
        ];
        for (kind, start) in cases {
            let id = table.add(format!("{kind:?}<Int>"), Kind::Handle(kind, vec![int]));
            let object = table.get(id).object.as_ref().unwrap();
            assert_eq!(object.fields.len(), 1, "{kind:?}");
            assert_eq!(object.fields[0].offset, start, "{kind:?}");
            assert_eq!(object.size, start + 8, "{kind:?}");
            assert_eq!(table.layout(id), Layout::REFERENCE);
        }
    }

    #[test]
    fn handles_a_value_without_a_payload_is_its_words() {
        let mut table = TypeTable::new();
        let sizes = [
            (HandleKind::AtomicInt, 16),
            (HandleKind::AtomicBool, 16),
            (HandleKind::Port, 16),
            (HandleKind::Sender, 24),
            (HandleKind::Receiver, 24),
        ];
        for (kind, size) in sizes {
            let id = table.add(format!("{kind:?}"), Kind::Handle(kind, vec![]));
            assert_eq!(table.get(id).object.as_ref().unwrap().size, size, "{kind:?}");
        }
    }

    #[test]
    fn handles_a_channel_end_refers_to_its_channel_and_the_channel_to_its_buffer() {
        let mut table = TypeTable::new();
        let string = table.add("String", Kind::String);
        let buffer = table.add("[String]", Kind::Array(string));
        let channel = table.add("Channel<String>", Kind::Handle(HandleKind::Channel, vec![buffer]));
        let sender = table.add("Sender<String>", Kind::Handle(HandleKind::Sender, vec![]));
        let channel_object = table.get(channel).object.as_ref().unwrap();
        assert_eq!(channel_object.size, 56 + 8);
        assert_eq!(channel_object.fields[0].offset, 56);
        assert_eq!(channel_object.slots, [Slot::Ref(56)]);
        assert_eq!(table.get(sender).object.as_ref().unwrap().slots, [Slot::Ref(8)]);
    }

    #[test]
    fn handles_the_payload_keeps_its_reference_slots() {
        let mut table = TypeTable::new();
        let string = table.add("String", Kind::String);
        let mutex = table.add("Mutex<String>", Kind::Handle(HandleKind::Mutex, vec![string]));
        let task = table.add("Task<String>", Kind::Handle(HandleKind::Task, vec![string]));
        assert_eq!(table.get(mutex).object.as_ref().unwrap().slots, [Slot::Ref(24)]);
        assert_eq!(table.get(task).object.as_ref().unwrap().slots, [Slot::Ref(24), Slot::Ref(32)]);
    }

    #[test]
    fn handles_a_payload_with_a_dynamic_word_is_scanned_by_its_type() {
        let mut table = TypeTable::new();
        let union = table.add("Int | String", Kind::Pair);
        let once = table.add("Once<Int | String>", Kind::Handle(HandleKind::Once, vec![union]));
        assert_eq!(table.get(once).object.as_ref().unwrap().slots, [Slot::Pair(32)]);
    }

    #[test]
    fn handles_every_kind_lists_the_payloads_it_takes() {
        let counts: Vec<usize> = HandleKind::ALL.iter().map(|kind| kind.payloads()).collect();
        assert_eq!(counts, [1, 0, 0, 1, 1, 1, 0, 0, 0]);
    }

    #[test]
    fn handles_the_states_are_stable_numbers() {
        assert_eq!([TASK_PENDING, TASK_DONE, TASK_CANCELLED, TASK_FAULTED], [0, 1, 2, 3]);
        assert_eq!([ONCE_EMPTY, ONCE_RUNNING, ONCE_DONE], [0, 1, 2]);
    }

    #[test]
    #[should_panic(expected = "handle payload")]
    fn handles_a_payload_of_the_wrong_count_is_rejected() {
        let mut table = TypeTable::new();
        table.add("Mutex", Kind::Handle(HandleKind::Mutex, vec![]));
    }
}
