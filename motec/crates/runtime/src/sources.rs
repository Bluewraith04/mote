//! Event sources: a `Channel` fed by a platform source through a bounded queue.
//!
//! The source thread only touches the queue. Events move into the channel when a task calls `recv`, on that worker.

use std::collections::HashMap;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use contracts::{event_queue, EventQueue, Overflow, SourceDecode, SourceHandle, SourceRequest};
use isa::value::{ObjectHeader, Value, BACKING_DATA_BASE};

use crate::handlers::sched_intrinsics::{
    alloc_channel, CHAN_SLOT_BACKING, CHAN_SLOT_CAPACITY, CHAN_SLOT_HEAD, CHAN_SLOT_ID, CHAN_SLOT_LEN,
};
use crate::Runtime;

pub(crate) struct SourceState {
    pub(crate) queue: EventQueue,
    handle: Mutex<Option<SourceHandle>>,
    decode: SourceDecode,
}

pub(crate) type Sources = Mutex<HashMap<u64, Arc<SourceState>>>;

impl Runtime {
    pub(crate) fn open_source(
        &self,
        request: SourceRequest,
        capacity: usize,
        overflow: Overflow,
        decode: SourceDecode,
    ) -> Result<Value, String> {
        let platform = match request {
            SourceRequest::Custom(_) => None,
            _ => Some(self.platform.clone().ok_or("no platform installed")?),
        };
        let (sink, queue) = event_queue(capacity.max(1), overflow);
        let (channel, channel_id, state) = self.register_source(queue, decode);
        let started = match (request, platform) {
            (SourceRequest::Custom(start), _) => (start.0)(sink),
            (request, Some(platform)) => platform.open_source(request, sink).map_err(|e| e.message),
            (_, None) => Err("no platform installed".to_string()),
        };
        match started {
            Ok(handle) => *state.handle.lock().unwrap() = Some(handle),
            Err(message) => {
                self.sources.lock().unwrap().remove(&channel_id);
                return Err(message);
            }
        }
        Ok(channel)
    }

    fn register_source(&self, queue: EventQueue, decode: SourceDecode) -> (Value, u64, Arc<SourceState>) {
        let mut channel = alloc_channel(self, queue.capacity());
        // SAFETY: `alloc_channel` initialised every slot.
        let channel_id = unsafe { channel.as_mut().get_field(CHAN_SLOT_ID) }.as_uint().unwrap_or(0);
        let (sched, wake) = (self.sched.clone(), self.wake.clone());
        queue.set_waker(Arc::new(move || {
            let mut st = sched.lock().unwrap();
            let waiting: Vec<u64> = st.recv_waiters.remove(&channel_id).into_iter().flatten().collect();
            for id in waiting {
                st.wake_or_note_early(id);
            }
            drop(st);
            wake.all();
        }));
        let state = Arc::new(SourceState { queue, handle: Mutex::new(None), decode });
        self.sources.lock().unwrap().insert(channel_id, state.clone());
        (Value::boxed(channel), channel_id, state)
    }

    pub(crate) fn source(&self, channel_id: u64) -> Option<Arc<SourceState>> {
        self.sources.lock().unwrap().get(&channel_id).cloned()
    }

    pub(crate) fn close_source(&self, channel_id: u64) {
        let state = self.sources.lock().unwrap().remove(&channel_id);
        if let Some(state) = state {
            state.stop();
        }
    }

    pub(crate) fn sources_can_wake(&self, st: &crate::sched::SchedState) -> bool {
        let sources = self.sources.lock().unwrap();
        sources.keys().any(|id| st.recv_waiters.get(id).is_some_and(|w| !w.is_empty()))
    }

    pub(crate) fn close_all_sources(&self) {
        let all: Vec<_> = self.sources.lock().unwrap().drain().map(|(_, s)| s).collect();
        for state in all {
            state.stop();
        }
    }
}

impl SourceState {
    fn stop(&self) {
        self.queue.close();
        if let Some(handle) = self.handle.lock().unwrap().take() {
            handle.close();
        }
    }

    pub(crate) fn has_news(&self) -> bool {
        !self.queue.is_empty() || !self.queue.is_live()
    }
}

pub(crate) fn pump(rt: &Runtime, state: &SourceState, channel: &mut NonNull<ObjectHeader>) -> Result<(), String> {
    // SAFETY: a live channel's slots are initialised by `alloc_channel`; the caller holds its lock.
    let (capacity, head, mut len, backing) = unsafe {
        let h = channel.as_ref();
        (
            h.get_field(CHAN_SLOT_CAPACITY).as_uint().unwrap_or(0) as usize,
            h.get_field(CHAN_SLOT_HEAD).as_uint().unwrap_or(0) as usize,
            h.get_field(CHAN_SLOT_LEN).as_uint().unwrap_or(0) as usize,
            h.get_field(CHAN_SLOT_BACKING),
        )
    };
    let mut backing = backing.as_object_ptr().ok_or("source channel backing is corrupt")?;
    while len < capacity {
        let Some(payload) = state.queue.try_pop() else { break };
        let value = rt.with_intrinsic_ctx(|ctx| (state.decode)(ctx, payload))?;
        let slot = BACKING_DATA_BASE + (head + len) % capacity;
        // SAFETY: `slot` is within the backing's capacity since `len < capacity`.
        unsafe { backing.as_mut().set_field(slot, value) };
        len += 1;
    }
    // SAFETY: as above.
    unsafe {
        channel.as_mut().set_field(CHAN_SLOT_LEN, Value::uint(len as u64));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(_: &mut dyn contracts::NativeCtx, _: contracts::EventPayload) -> Result<Value, String> {
        Ok(Value::null())
    }

    #[test]
    fn a_source_ending_with_a_parked_receiver_still_counts_as_waking() {
        let rt = Runtime::new(Vec::new());
        let (sink, queue) = event_queue(2, Overflow::Pause);
        rt.register_source(queue, decode);
        let id = *rt.sources.lock().unwrap().keys().next().unwrap();
        let mut st = rt.sched.lock().unwrap();
        st.recv_waiters.entry(id).or_default().push_back(0);
        let ender = std::thread::spawn(move || sink.end());
        while rt.source(id).unwrap().queue.is_live() {
            std::thread::yield_now();
        }
        assert!(rt.sources_can_wake(&st));
        drop(st);
        ender.join().unwrap();
        assert!(!rt.sources_can_wake(&rt.sched.lock().unwrap()));
    }
}
