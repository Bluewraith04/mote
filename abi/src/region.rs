use std::sync::atomic::{AtomicU64, Ordering};

use crate::callconv::TaskContext;
use crate::handle::HandleKind;
use crate::kind::{Kind, TypeId};
use crate::layout::WORD;
use crate::table::TypeTable;

/// Largest fixed-size object the inline allocation sequence takes; a larger one calls `Entry::Alloc`.
pub const MAX_INLINE_ALLOC: u32 = 256;

/// The zeroed memory a task allocates from: addresses from `cursor` up to `limit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub cursor: u64,
    pub limit: u64,
}

impl Region {
    /// The inline sequence: the object's address and the cursor moved past it, or `None` for the slow path.
    pub fn bump(&mut self, size: u32) -> Option<u64> {
        debug_assert!(size.is_multiple_of(WORD) && self.cursor.is_multiple_of(u64::from(WORD)), "a size and a cursor are whole words");
        let end = self.cursor.checked_add(u64::from(size)).filter(|end| *end <= self.limit)?;
        let address = self.cursor;
        self.cursor = end;
        Some(address)
    }
}

impl TaskContext {
    pub fn region(&self) -> Region {
        Region { cursor: self.region_cursor, limit: self.region_limit }
    }

    pub fn set_region(&mut self, region: Region) {
        self.region_cursor = region.cursor;
        self.region_limit = region.limit;
    }

    /// Asks the task to enter the runtime at its next poll; callable from another thread.
    ///
    /// # Safety
    /// `self` points to a live context.
    pub unsafe fn request_poll(context: *mut TaskContext) {
        // SAFETY: the caller's promise; the field is a naturally aligned word.
        unsafe { AtomicU64::from_ptr(&raw mut (*context).poll) }.store(1, Ordering::SeqCst);
    }

    /// Clears the poll word, as the task does when it enters the runtime to answer a poll.
    pub fn take_poll(&mut self) -> bool {
        // SAFETY: the field is a naturally aligned word of a live context.
        unsafe { AtomicU64::from_ptr(&raw mut self.poll) }.swap(0, Ordering::SeqCst) != 0
    }
}

/// Whether the runtime keeps a record for an object of the kind and frees it when the object dies.
pub fn is_finalized(kind: &Kind) -> bool {
    matches!(kind, Kind::Handle(HandleKind::Port | HandleKind::Task | HandleKind::Channel | HandleKind::Mutex | HandleKind::Once, _))
}

/// The bytes the inline sequence takes for a type, or `None` when the type calls `Entry::Alloc`.
pub fn inline_size(table: &TypeTable, id: TypeId) -> Option<u32> {
    let info = table.get(id);
    let object = info.object.as_ref()?;
    if object.element.is_some() || is_finalized(&info.kind) {
        return None;
    }
    let size = object.size;
    (size <= MAX_INLINE_ALLOC).then_some(size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::Kind;
    use crate::scalar::Scalar;

    #[test]
    fn region_a_bump_returns_the_cursor_and_moves_it_past_the_object() {
        let mut region = Region { cursor: 0x1000, limit: 0x1040 };
        assert_eq!(region.bump(24), Some(0x1000));
        assert_eq!(region.cursor, 0x1018);
        assert_eq!(region.bump(16), Some(0x1018));
        assert_eq!(region.cursor, 0x1028);
    }

    #[test]
    fn region_an_object_ending_exactly_at_the_limit_fits() {
        let mut region = Region { cursor: 0x1000, limit: 0x1010 };
        assert_eq!(region.bump(16), Some(0x1000));
        assert_eq!(region.cursor, region.limit);
        assert_eq!(region.bump(8), None);
    }

    #[test]
    fn region_an_object_past_the_limit_leaves_the_cursor_for_the_slow_path() {
        let mut region = Region { cursor: 0x1000, limit: 0x1010 };
        assert_eq!(region.bump(24), None);
        assert_eq!(region.cursor, 0x1000);
    }

    #[test]
    fn region_a_cursor_near_the_top_of_the_address_space_does_not_wrap() {
        let mut region = Region { cursor: u64::MAX - 7, limit: u64::MAX };
        assert_eq!(region.bump(16), None);
    }

    #[test]
    fn region_a_requested_poll_is_taken_once() {
        let mut context = TaskContext::default();
        assert!(!context.take_poll());
        // SAFETY: the context is alive.
        unsafe { TaskContext::request_poll(&mut context) };
        assert_eq!(context.poll, 1);
        assert!(context.take_poll());
        assert!(!context.take_poll());
    }

    #[test]
    fn region_lives_in_the_first_two_words_of_the_context() {
        let mut context = TaskContext::default();
        context.set_region(Region { cursor: 8, limit: 64 });
        assert_eq!((context.region_cursor, context.region_limit), (8, 64));
        assert_eq!(context.region(), Region { cursor: 8, limit: 64 });
    }

    #[test]
    fn region_only_a_small_fixed_size_object_is_allocated_inline() {
        let mut table = TypeTable::new();
        let int = table.add("Int", Kind::Scalar(Scalar::I64));
        let small = table.add("Small", Kind::Class(vec![int, int]));
        let wide = table.add("Wide", Kind::Class(vec![int; 40]));
        let text = table.add("String", Kind::String);
        let list = table.add("List<Int>", Kind::List(int));
        assert_eq!(inline_size(&table, small), Some(24));
        assert_eq!(inline_size(&table, wide), None);
        assert_eq!(inline_size(&table, text), None);
        assert_eq!(inline_size(&table, list), Some(32));
        assert_eq!(inline_size(&table, int), None);
    }

    #[test]
    fn region_a_port_is_finalized_so_it_calls_the_runtime() {
        let mut table = TypeTable::new();
        let port = table.add("Port", Kind::Handle(HandleKind::Port, vec![]));
        let sender = table.add("Sender", Kind::Handle(HandleKind::Sender, vec![]));
        assert_eq!(inline_size(&table, port), None);
        assert!(inline_size(&table, sender).is_some());
    }

    #[test]
    fn region_a_task_channel_mutex_and_once_are_finalized_like_a_port() {
        let mut table = TypeTable::new();
        let int = table.add("Int", Kind::Scalar(Scalar::I64));
        for kind in [HandleKind::Task, HandleKind::Channel, HandleKind::Mutex, HandleKind::Once] {
            let id = table.add("Handle", Kind::Handle(kind, vec![int]));
            assert_eq!(inline_size(&table, id), None, "{kind:?}");
            assert!(is_finalized(&table.get(id).kind), "{kind:?}");
        }
        assert!(!is_finalized(&Kind::Handle(HandleKind::Sender, vec![])));
    }
}
