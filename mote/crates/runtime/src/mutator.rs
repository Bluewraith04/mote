//! Each worker thread allocates through its own `Mutator`, cached here per heap.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

use contracts::Mutator;

use crate::Runtime;

static NEXT_HEAP_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_heap_id() -> u64 {
    NEXT_HEAP_ID.fetch_add(1, Ordering::Relaxed)
}

thread_local! {
    static CURRENT: RefCell<Option<(u64, Box<dyn Mutator>)>> = const { RefCell::new(None) };
}

impl Runtime {
    /// Runs `f` with this thread's mutator for the installed heap, creating it on first use; `f` must not allocate through the runtime again.
    pub(crate) fn with_mutator<R>(&self, f: impl FnOnce(&mut dyn Mutator) -> R) -> R {
        CURRENT.with(|cell| {
            let mut slot = cell.borrow_mut();
            if slot.as_ref().is_none_or(|(id, _)| *id != self.heap_id) {
                *slot = Some((self.heap_id, self.heap.mutator()));
            }
            f(slot.as_mut().expect("just installed").1.as_mut())
        })
    }
}

impl Runtime {
    /// Runs `f` with an [`crate::VmIntrinsicCtx`] over this thread's mutator.
    pub(crate) fn with_intrinsic_ctx<R>(&self, f: impl FnOnce(&mut crate::VmIntrinsicCtx) -> R) -> R {
        self.with_mutator(|mutator| {
            let mut ctx = crate::VmIntrinsicCtx {
                mutator,
                heap: self.heap.as_ref(),
                types: &self.intrinsic_types,
                string_type: self.string_type.as_ref(),
                platform: self.platform.as_deref(),
            };
            f(&mut ctx)
        })
    }
}
