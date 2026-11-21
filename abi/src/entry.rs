use crate::callconv::CONTEXT;
use crate::scalar::Scalar;

/// A function the runtime exports to compiled code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Entry {
    /// Stops the program at a fault site; never returns.
    Fault,
    /// Allocates a zeroed object of a fixed-size type.
    Alloc,
    /// Allocates a zeroed object with `count` elements.
    AllocArray,
    /// Stops the program with a message from `std` and no fault site; never returns.
    FaultMessage,
    /// Joins a stop of the world or a request of the scheduler; returns when it is over.
    Poll,
    /// Starts a task over a closure and returns its handle.
    TaskSpawn,
    /// Waits until a task has ended and returns its state.
    TaskAwait,
    /// Asks a task to stop at its next poll.
    TaskCancel,
    /// Returns 1 when a task has ended and 0 when not.
    TaskReady,
    /// Returns the text of the `Err` that `await` gives for a task that did not return.
    TaskMessage,
    /// Opens a `scope` and returns its token.
    ScopeBegin,
    /// Waits for every task of a scope.
    ScopeEnd,
    /// Allocates a channel of a channel type with room for a number of messages and returns it.
    ChannelNew,
    /// Allocates a `Sender` or a `Receiver` end of a channel and returns it.
    ChannelEnd,
    /// Copies a message from an address into a channel; returns 1 when it is sent and 0 when it is not.
    ChannelSend,
    /// Copies the next message of a channel to an address; returns 1 when there is one, 0 when the channel is closed and empty, and 2 when a wait ended with none.
    ChannelRecv,
    /// Closes a `Sender`.
    SenderClose,
    /// Returns another `Sender` of the same channel.
    SenderClone,
    /// Closes the receiving side of a channel.
    ReceiverClose,
    /// Returns another `Receiver` of the same channel.
    ReceiverClone,
    /// Returns 1 when no message can come and none is left.
    ReceiverClosed,
    /// Allocates a `Mutex` of a mutex type that is not locked and returns it.
    MutexNew,
    /// Locks a `Mutex` for the calling task, waiting while another holds it, and copies its value to an address.
    MutexTake,
    /// Copies a value from an address into a `Mutex` and unlocks it.
    MutexPut,
    /// Allocates a `Once` of a once type that has no value and returns it.
    OnceNew,
    /// Returns 1 when the calling task is to run the initializer of a `Once` and 0 when it has a value.
    OnceBegin,
    /// Copies the value from an address into a `Once`, which then has it.
    OnceFinish,
    /// Copies the value of a `Once` to an address.
    OnceValue,
    /// Sleeps a number of milliseconds of the scheduler's clock.
    Sleep,
    /// Waits for a task for at most a number of milliseconds; returns its state, which is pending when the time ran out.
    TaskAwaitTimeout,
    /// Like `ChannelRecv` with a wait, for at most a number of milliseconds; returns 1 when there is a message and 0 when there is none.
    ChannelRecvTimeout,
    /// Allocates the `PortEnd` of a channel of byte arrays that a port fills; the last parameter is the number of messages it keeps, and 0 keeps the newest only.
    PortNew,
    /// Returns a `MotePort *` for a library, which owns one reference to release with `port_close`.
    PortRef,
    /// Records a `with` the task is inside: a box holding the resource and the function that closes it from the box.
    Defer,
    /// Forgets the innermost `with` the task is inside, which has ended.
    Undefer,
    /// Like `ScopeEnd`; returns the text of the fault of a task nobody awaited, or null when there is none.
    ScopeEndError,
    /// Runs a native thunk on a helper thread with its argument block and parks the calling task until it returns.
    NativeBlocking,
}

impl Entry {
    /// New entries go at the end.
    pub const ALL: [Entry; 37] = [
        Entry::Fault,
        Entry::Alloc,
        Entry::AllocArray,
        Entry::FaultMessage,
        Entry::Poll,
        Entry::TaskSpawn,
        Entry::TaskAwait,
        Entry::TaskCancel,
        Entry::TaskReady,
        Entry::TaskMessage,
        Entry::ScopeBegin,
        Entry::ScopeEnd,
        Entry::ChannelNew,
        Entry::ChannelEnd,
        Entry::ChannelSend,
        Entry::ChannelRecv,
        Entry::SenderClose,
        Entry::SenderClone,
        Entry::ReceiverClose,
        Entry::ReceiverClone,
        Entry::ReceiverClosed,
        Entry::MutexNew,
        Entry::MutexTake,
        Entry::MutexPut,
        Entry::OnceNew,
        Entry::OnceBegin,
        Entry::OnceFinish,
        Entry::OnceValue,
        Entry::Sleep,
        Entry::TaskAwaitTimeout,
        Entry::ChannelRecvTimeout,
        Entry::PortNew,
        Entry::PortRef,
        Entry::Defer,
        Entry::Undefer,
        Entry::ScopeEndError,
        Entry::NativeBlocking,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Entry::Fault => "mote_fault",
            Entry::Alloc => "mote_alloc",
            Entry::AllocArray => "mote_alloc_array",
            Entry::FaultMessage => "mote_fault_message",
            Entry::Poll => "mote_poll",
            Entry::TaskSpawn => "mote_task_spawn",
            Entry::TaskAwait => "mote_task_await",
            Entry::TaskCancel => "mote_task_cancel",
            Entry::TaskReady => "mote_task_ready",
            Entry::TaskMessage => "mote_task_message",
            Entry::ScopeBegin => "mote_scope_begin",
            Entry::ScopeEnd => "mote_scope_end",
            Entry::ChannelNew => "mote_channel_new",
            Entry::ChannelEnd => "mote_channel_end",
            Entry::ChannelSend => "mote_channel_send",
            Entry::ChannelRecv => "mote_channel_recv",
            Entry::SenderClose => "mote_sender_close",
            Entry::SenderClone => "mote_sender_clone",
            Entry::ReceiverClose => "mote_receiver_close",
            Entry::ReceiverClone => "mote_receiver_clone",
            Entry::ReceiverClosed => "mote_receiver_closed",
            Entry::MutexNew => "mote_mutex_new",
            Entry::MutexTake => "mote_mutex_take",
            Entry::MutexPut => "mote_mutex_put",
            Entry::OnceNew => "mote_once_new",
            Entry::OnceBegin => "mote_once_begin",
            Entry::OnceFinish => "mote_once_finish",
            Entry::OnceValue => "mote_once_value",
            Entry::Sleep => "mote_sleep",
            Entry::TaskAwaitTimeout => "mote_task_await_timeout",
            Entry::ChannelRecvTimeout => "mote_channel_recv_timeout",
            Entry::PortNew => "mote_port_new",
            Entry::PortRef => "mote_port_ref",
            Entry::Defer => "mote_defer",
            Entry::Undefer => "mote_undefer",
            Entry::ScopeEndError => "mote_scope_end_error",
            Entry::NativeBlocking => "mote_native_blocking",
        }
    }

    /// The machine parameters, the task context first.
    pub fn params(self) -> &'static [Scalar] {
        match self {
            Entry::Poll | Entry::ScopeBegin | Entry::Undefer => &[CONTEXT],
            Entry::Defer => &[CONTEXT, Scalar::Ref, Scalar::I64],
            Entry::Fault | Entry::Alloc | Entry::MutexNew | Entry::OnceNew => &[CONTEXT, Scalar::I32],
            Entry::AllocArray => &[CONTEXT, Scalar::I32, Scalar::I64],
            Entry::FaultMessage | Entry::TaskAwait | Entry::TaskCancel | Entry::TaskReady | Entry::TaskMessage | Entry::OnceBegin | Entry::PortRef => &[CONTEXT, Scalar::Ref],
            Entry::PortNew => &[CONTEXT, Scalar::Ref, Scalar::I32, Scalar::I64],
            Entry::MutexTake | Entry::MutexPut | Entry::OnceFinish | Entry::OnceValue => &[CONTEXT, Scalar::Ref, Scalar::I64],
            Entry::NativeBlocking => &[CONTEXT, Scalar::I64, Scalar::I64, Scalar::I64],
            Entry::TaskSpawn => &[CONTEXT, Scalar::I32, Scalar::I64, Scalar::Ref, Scalar::I64],
            Entry::ScopeEnd | Entry::Sleep | Entry::ScopeEndError => &[CONTEXT, Scalar::I64],
            Entry::TaskAwaitTimeout => &[CONTEXT, Scalar::Ref, Scalar::I64],
            Entry::ChannelNew => &[CONTEXT, Scalar::I32, Scalar::I64],
            Entry::ChannelEnd => &[CONTEXT, Scalar::Ref, Scalar::I32],
            Entry::ChannelSend | Entry::ChannelRecv | Entry::ChannelRecvTimeout => &[CONTEXT, Scalar::Ref, Scalar::I64, Scalar::I64],
            Entry::SenderClose | Entry::SenderClone | Entry::ReceiverClose | Entry::ReceiverClone | Entry::ReceiverClosed => &[CONTEXT, Scalar::Ref],
        }
    }

    pub fn results(self) -> &'static [Scalar] {
        match self {
            Entry::Fault | Entry::FaultMessage | Entry::Poll | Entry::TaskCancel | Entry::ScopeEnd | Entry::SenderClose | Entry::ReceiverClose | Entry::Defer | Entry::Undefer => &[],
            Entry::MutexTake | Entry::MutexPut | Entry::OnceFinish | Entry::OnceValue | Entry::Sleep | Entry::NativeBlocking => &[],
            Entry::Alloc | Entry::AllocArray | Entry::TaskSpawn | Entry::TaskMessage | Entry::ChannelNew | Entry::ChannelEnd | Entry::SenderClone | Entry::ReceiverClone | Entry::MutexNew | Entry::OnceNew | Entry::PortNew | Entry::ScopeEndError => &[Scalar::Ref],
            Entry::TaskAwait | Entry::TaskReady | Entry::ScopeBegin | Entry::ChannelSend | Entry::ChannelRecv | Entry::ReceiverClosed | Entry::OnceBegin | Entry::TaskAwaitTimeout | Entry::ChannelRecvTimeout | Entry::PortRef => &[Scalar::I64],
        }
    }

    pub fn never_returns(self) -> bool {
        matches!(self, Entry::Fault | Entry::FaultMessage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn names_are_unique_and_prefixed() {
        let names: HashSet<_> = Entry::ALL.iter().map(|entry| entry.name()).collect();
        assert_eq!(names.len(), Entry::ALL.len());
        assert!(names.iter().all(|name| name.starts_with("mote_")));
    }

    #[test]
    fn allocation_takes_the_context_and_a_type_id_and_returns_a_reference() {
        assert_eq!(Entry::Alloc.params(), [CONTEXT, Scalar::I32]);
        assert_eq!(Entry::AllocArray.params(), [CONTEXT, Scalar::I32, Scalar::I64]);
        assert_eq!(Entry::Alloc.results(), [Scalar::Ref]);
    }

    #[test]
    fn every_entry_takes_the_context_first() {
        assert!(Entry::ALL.iter().all(|entry| entry.params()[0] == CONTEXT));
    }

    #[test]
    fn only_the_fault_entries_never_return() {
        for entry in [Entry::Fault, Entry::FaultMessage] {
            assert!(entry.never_returns());
            assert!(entry.results().is_empty());
        }
        assert!(!Entry::Alloc.never_returns());
        assert!(!Entry::AllocArray.never_returns());
        assert!(Entry::ALL.iter().filter(|entry| entry.never_returns()).count() == 2);
    }

    #[test]
    fn a_fault_with_a_message_takes_the_message_string_and_no_site() {
        assert_eq!(Entry::FaultMessage.params(), [CONTEXT, Scalar::Ref]);
        assert_eq!(Entry::FaultMessage.name(), "mote_fault_message");
    }

    #[test]
    fn the_task_entries_take_the_handle_and_the_closure_by_reference() {
        assert_eq!(Entry::TaskSpawn.params(), [CONTEXT, Scalar::I32, Scalar::I64, Scalar::Ref, Scalar::I64]);
        assert_eq!(Entry::TaskSpawn.results(), [Scalar::Ref]);
        for entry in [Entry::TaskAwait, Entry::TaskCancel, Entry::TaskReady, Entry::TaskMessage] {
            assert_eq!(entry.params(), [CONTEXT, Scalar::Ref]);
        }
        assert_eq!(Entry::TaskAwait.results(), [Scalar::I64]);
        assert_eq!(Entry::TaskMessage.results(), [Scalar::Ref]);
    }

    #[test]
    fn the_channel_entries_pass_the_end_first_and_an_address_for_the_message() {
        assert_eq!(Entry::ChannelNew.params(), [CONTEXT, Scalar::I32, Scalar::I64]);
        assert_eq!(Entry::ChannelEnd.params(), [CONTEXT, Scalar::Ref, Scalar::I32]);
        for entry in [Entry::ChannelSend, Entry::ChannelRecv] {
            assert_eq!(entry.params(), [CONTEXT, Scalar::Ref, Scalar::I64, Scalar::I64]);
            assert_eq!(entry.results(), [Scalar::I64]);
        }
        for entry in [Entry::SenderClose, Entry::SenderClone, Entry::ReceiverClose, Entry::ReceiverClone, Entry::ReceiverClosed] {
            assert_eq!(entry.params(), [CONTEXT, Scalar::Ref]);
        }
        assert_eq!(Entry::ChannelNew.results(), [Scalar::Ref]);
        assert_eq!(Entry::SenderClone.results(), [Scalar::Ref]);
        assert!(Entry::SenderClose.results().is_empty());
    }

    #[test]
    fn the_lock_entries_pass_the_handle_first_and_an_address_for_the_value() {
        for entry in [Entry::MutexNew, Entry::OnceNew] {
            assert_eq!(entry.params(), [CONTEXT, Scalar::I32]);
            assert_eq!(entry.results(), [Scalar::Ref]);
        }
        for entry in [Entry::MutexTake, Entry::MutexPut, Entry::OnceFinish, Entry::OnceValue] {
            assert_eq!(entry.params(), [CONTEXT, Scalar::Ref, Scalar::I64]);
            assert!(entry.results().is_empty());
        }
        assert_eq!(Entry::OnceBegin.params(), [CONTEXT, Scalar::Ref]);
        assert_eq!(Entry::OnceBegin.results(), [Scalar::I64]);
    }

    #[test]
    fn the_timed_entries_take_the_time_in_milliseconds_last() {
        assert_eq!(Entry::Sleep.params(), [CONTEXT, Scalar::I64]);
        assert!(Entry::Sleep.results().is_empty());
        assert_eq!(Entry::TaskAwaitTimeout.params(), [CONTEXT, Scalar::Ref, Scalar::I64]);
        assert_eq!(Entry::ChannelRecvTimeout.params(), [CONTEXT, Scalar::Ref, Scalar::I64, Scalar::I64]);
        for entry in [Entry::TaskAwaitTimeout, Entry::ChannelRecvTimeout] {
            assert_eq!(entry.results(), [Scalar::I64]);
        }
    }

    #[test]
    fn a_port_is_opened_on_a_channel_and_referenced_by_address() {
        assert_eq!(Entry::PortNew.params(), [CONTEXT, Scalar::Ref, Scalar::I32, Scalar::I64]);
        assert_eq!(Entry::PortNew.results(), [Scalar::Ref]);
        assert_eq!(Entry::PortRef.params(), [CONTEXT, Scalar::Ref]);
        assert_eq!(Entry::PortRef.results(), [Scalar::I64]);
    }

    #[test]
    fn a_with_is_recorded_with_its_box_and_closer_and_forgotten_at_its_end() {
        assert_eq!(Entry::Defer.params(), [CONTEXT, Scalar::Ref, Scalar::I64]);
        assert_eq!(Entry::Undefer.params(), [CONTEXT]);
        assert!(Entry::Defer.results().is_empty() && Entry::Undefer.results().is_empty());
        assert_eq!(Entry::ScopeEndError.params(), [CONTEXT, Scalar::I64]);
        assert_eq!(Entry::ScopeEndError.results(), [Scalar::Ref]);
    }

    #[test]
    fn a_scope_is_opened_for_a_token_and_closed_with_it() {
        assert_eq!(Entry::ScopeBegin.params(), [CONTEXT]);
        assert_eq!(Entry::ScopeBegin.results(), [Scalar::I64]);
        assert_eq!(Entry::ScopeEnd.params(), [CONTEXT, Scalar::I64]);
        assert!(Entry::ScopeEnd.results().is_empty());
    }
}
