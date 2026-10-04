//! Event sources: a bounded queue a source thread fills and the runtime drains.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

/// What a source produces; the runtime opens it through [`crate::Platform::open_source`], except a custom one it starts itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceRequest {
    /// `Int(n)` for the nth tick, every `period_nanos`.
    Timer { period_nanos: u64 },
    /// A source the native supplies, started with the queue's sink.
    Custom(CustomSource),
}

/// Starts a source: given the sink, answers the handle that stops it.
#[derive(Clone)]
pub struct CustomSource(pub Arc<dyn Fn(EventSink) -> Result<SourceHandle, String> + Send + Sync>);

impl std::fmt::Debug for CustomSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CustomSource")
    }
}

impl PartialEq for CustomSource {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for CustomSource {}

/// Owned event data; a source thread never allocates a GC object.
#[derive(Clone, Debug, PartialEq)]
pub enum EventPayload {
    Unit,
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
    Bytes(Vec<u8>),
    List(Vec<EventPayload>),
}

/// What a source does when its queue is full.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overflow {
    /// `push` blocks the source thread until there is room.
    Pause,
    /// The new event replaces the newest queued one.
    Coalesce,
    /// The source ends; queued events still drain.
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The outcome of pushing an event.
pub enum PushResult {
    Queued,
    Coalesced,
    /// `try_push` under `Pause` with no room; the event was not queued.
    Full,
    /// The source is over (ended, overflow-closed or closed by the receiver); stop producing.
    Closed,
}

struct Queue {
    events: VecDeque<EventPayload>,
    ended: bool,
    closed: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    space: Condvar,
    capacity: usize,
    overflow: Overflow,
    waker: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl Shared {
    fn wake(&self) {
        let waker = self.waker.lock().unwrap().clone();
        if let Some(waker) = waker {
            waker();
        }
    }
}

/// The source's end of a queue. Cloneable and `Send + Sync`.
#[derive(Clone)]
pub struct EventSink {
    shared: Arc<Shared>,
}

/// The runtime's end of a queue.
pub struct EventQueue {
    shared: Arc<Shared>,
}

/// A queue holding at most `capacity` events (at least 1) and its two ends.
pub fn event_queue(capacity: usize, overflow: Overflow) -> (EventSink, EventQueue) {
    let shared = Arc::new(Shared {
        queue: Mutex::new(Queue { events: VecDeque::new(), ended: false, closed: false }),
        space: Condvar::new(),
        capacity: capacity.max(1),
        overflow,
        waker: Mutex::new(None),
    });
    (EventSink { shared: shared.clone() }, EventQueue { shared })
}

impl EventSink {
    /// Queues `payload`, blocking under [`Overflow::Pause`] while the queue is full.
    pub fn push(&self, payload: EventPayload) -> PushResult {
        self.push_with(payload, true)
    }

    /// Like [`Self::push`], but returns [`PushResult::Full`] instead of blocking.
    pub fn try_push(&self, payload: EventPayload) -> PushResult {
        self.push_with(payload, false)
    }

    fn push_with(&self, payload: EventPayload, block: bool) -> PushResult {
        let s = &self.shared;
        let mut q = s.queue.lock().unwrap();
        let result = loop {
            if q.ended || q.closed {
                return PushResult::Closed;
            }
            if q.events.len() < s.capacity {
                q.events.push_back(payload);
                break PushResult::Queued;
            }
            match s.overflow {
                Overflow::Coalesce => {
                    *q.events.back_mut().expect("a full queue has a newest event") = payload;
                    break PushResult::Coalesced;
                }
                Overflow::Close => {
                    q.ended = true;
                    drop(q);
                    s.wake();
                    return PushResult::Closed;
                }
                Overflow::Pause if !block => return PushResult::Full,
                Overflow::Pause => q = s.space.wait(q).unwrap(),
            }
        };
        drop(q);
        s.wake();
        result
    }

    /// The source is done; queued events still drain.
    pub fn end(&self) {
        self.shared.queue.lock().unwrap().ended = true;
        self.shared.space.notify_all();
        self.shared.wake();
    }

    /// Whether the source should stop producing.
    #[cfg(test)]
    pub(crate) fn is_closed(&self) -> bool {
        let q = self.shared.queue.lock().unwrap();
        q.ended || q.closed
    }
}

impl EventQueue {
    pub fn try_pop(&self) -> Option<EventPayload> {
        let event = self.shared.queue.lock().unwrap().events.pop_front();
        if event.is_some() {
            self.shared.space.notify_one();
        }
        event
    }

    pub fn capacity(&self) -> usize {
        self.shared.capacity
    }

    pub fn len(&self) -> usize {
        self.shared.queue.lock().unwrap().events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the source can still push (it has not ended or been closed).
    pub fn is_live(&self) -> bool {
        let q = self.shared.queue.lock().unwrap();
        !q.ended && !q.closed
    }

    /// Nothing queued and nothing more will come.
    pub fn is_finished(&self) -> bool {
        let q = self.shared.queue.lock().unwrap();
        q.events.is_empty() && (q.ended || q.closed)
    }

    /// The receiver is done: pending and future pushes return [`PushResult::Closed`].
    pub fn close(&self) {
        self.shared.queue.lock().unwrap().closed = true;
        self.shared.space.notify_all();
        self.shared.wake();
    }

    /// Called after every push, `end` and `close`, on the calling thread with no lock held.
    pub fn set_waker(&self, waker: Arc<dyn Fn() + Send + Sync>) {
        *self.shared.waker.lock().unwrap() = Some(waker);
    }
}

/// Stops a running source when closed.
pub struct SourceHandle {
    stop: Option<Box<dyn FnOnce() + Send + Sync>>,
}

impl SourceHandle {
    pub fn new(stop: impl FnOnce() + Send + Sync + 'static) -> Self {
        SourceHandle { stop: Some(Box::new(stop)) }
    }

    pub fn close(mut self) {
        if let Some(stop) = self.stop.take() {
            stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn ints(q: &EventQueue) -> Vec<i64> {
        std::iter::from_fn(|| q.try_pop()).map(|e| if let EventPayload::Int(n) = e { n } else { panic!() }).collect()
    }

    #[test]
    fn events_come_out_in_order() {
        let (sink, q) = event_queue(4, Overflow::Pause);
        for n in 1..=3 {
            assert_eq!(sink.push(EventPayload::Int(n)), PushResult::Queued);
        }
        assert_eq!(ints(&q), vec![1, 2, 3]);
    }

    #[test]
    fn coalesce_replaces_the_newest_when_full() {
        let (sink, q) = event_queue(2, Overflow::Coalesce);
        sink.push(EventPayload::Int(1));
        sink.push(EventPayload::Int(2));
        assert_eq!(sink.push(EventPayload::Int(3)), PushResult::Coalesced);
        assert_eq!(ints(&q), vec![1, 3]);
    }

    #[test]
    fn close_policy_ends_the_source_but_queued_events_drain() {
        let (sink, q) = event_queue(1, Overflow::Close);
        sink.push(EventPayload::Int(1));
        assert_eq!(sink.push(EventPayload::Int(2)), PushResult::Closed);
        assert!(!q.is_live() && !q.is_finished());
        assert_eq!(ints(&q), vec![1]);
        assert!(q.is_finished());
    }

    #[test]
    fn try_push_under_pause_reports_full_without_blocking() {
        let (sink, q) = event_queue(1, Overflow::Pause);
        sink.try_push(EventPayload::Int(1));
        assert_eq!(sink.try_push(EventPayload::Int(2)), PushResult::Full);
        q.try_pop();
        assert_eq!(sink.try_push(EventPayload::Int(2)), PushResult::Queued);
    }

    #[test]
    fn pause_blocks_the_pusher_until_the_receiver_pops() {
        let (sink, q) = event_queue(1, Overflow::Pause);
        sink.push(EventPayload::Int(1));
        let pusher = std::thread::spawn(move || sink.push(EventPayload::Int(2)));
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(q.len(), 1);
        q.try_pop();
        assert_eq!(pusher.join().unwrap(), PushResult::Queued);
        assert_eq!(ints(&q), vec![2]);
    }

    #[test]
    fn closing_releases_a_paused_pusher() {
        let (sink, q) = event_queue(1, Overflow::Pause);
        sink.push(EventPayload::Int(1));
        let pusher = std::thread::spawn(move || sink.push(EventPayload::Int(2)));
        std::thread::sleep(std::time::Duration::from_millis(20));
        q.close();
        assert_eq!(pusher.join().unwrap(), PushResult::Closed);
    }

    #[test]
    fn end_stops_pushes_and_finishes_after_the_drain() {
        let (sink, q) = event_queue(2, Overflow::Pause);
        sink.push(EventPayload::Int(1));
        sink.end();
        assert!(sink.is_closed());
        assert_eq!(sink.push(EventPayload::Int(2)), PushResult::Closed);
        assert!(!q.is_finished());
        assert_eq!(ints(&q), vec![1]);
        assert!(q.is_finished());
    }

    #[test]
    fn the_waker_runs_on_push_end_and_close() {
        let (sink, q) = event_queue(2, Overflow::Pause);
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        q.set_waker(Arc::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
        }));
        sink.push(EventPayload::Unit);
        sink.end();
        q.close();
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn a_handle_runs_its_stop_once() {
        let n = Arc::new(AtomicUsize::new(0));
        let c = n.clone();
        SourceHandle::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
        })
        .close();
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }
}
