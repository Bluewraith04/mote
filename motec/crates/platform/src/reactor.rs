//! The reactor: one thread holding every wait for time or socket readiness, so a sleeping
//! or idle task holds no thread. Timers are a heap; readiness comes from the OS poller.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use contracts::{EventPayload, EventSink, PlatformError, PlatformErrorKind, PlatformRequest, PlatformResponse, PushResult, Wake, Woken};
use polling::{Event, Events, Poller};

const TICK_RETRY: Duration = Duration::from_millis(1);

#[derive(Clone)]
pub(crate) enum Pollable {
    Stream(Arc<TcpStream>),
    Listener(Arc<TcpListener>),
    Udp(Arc<UdpSocket>),
}

impl Pollable {
    fn add(&self, poller: &Poller, key: usize) -> std::io::Result<()> {
        let interest = Event::readable(key);
        // SAFETY: the entry owns a clone of the socket and every path that drops the entry deletes it first.
        unsafe {
            match self {
                Pollable::Stream(s) => poller.add(&**s, interest),
                Pollable::Listener(s) => poller.add(&**s, interest),
                Pollable::Udp(s) => poller.add(&**s, interest),
            }
        }
    }

    fn delete(&self, poller: &Poller) {
        let _ = match self {
            Pollable::Stream(s) => poller.delete(&**s),
            Pollable::Listener(s) => poller.delete(&**s),
            Pollable::Udp(s) => poller.delete(&**s),
        };
    }
}

enum Entry {
    Sleep { wake: Wake },
    Readable { source: Pollable, request: PlatformRequest, wake: Wake },
    Ticker { sink: EventSink, period: Duration, count: i64 },
}

struct Slot {
    entry: Entry,
    deadline: Option<Instant>,
}

#[derive(Default)]
struct State {
    next_key: usize,
    slots: HashMap<usize, Slot>,
    timers: BinaryHeap<Reverse<(Instant, usize)>>,
}

pub(crate) struct Reactor {
    poller: Poller,
    state: Mutex<State>,
    shutdown: AtomicBool,
}

fn timed_out() -> PlatformError {
    PlatformError { kind: PlatformErrorKind::TimedOut, message: "read timed out".to_string() }
}

fn after(now: Instant, delay: Duration) -> Instant {
    now.checked_add(delay).unwrap_or_else(|| now + Duration::from_secs(60 * 60 * 24 * 365 * 30))
}

impl Reactor {
    pub(crate) fn start() -> std::io::Result<Arc<Reactor>> {
        let reactor = Arc::new(Reactor { poller: Poller::new()?, state: Mutex::new(State::default()), shutdown: AtomicBool::new(false) });
        let running = reactor.clone();
        std::thread::Builder::new().name("mote-reactor".to_string()).spawn(move || running.run())?;
        Ok(reactor)
    }

    pub(crate) fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = self.poller.notify();
    }

    pub(crate) fn sleep(&self, delay: Duration, wake: Wake) {
        self.insert(Entry::Sleep { wake }, Some(delay));
    }

    pub(crate) fn readable(
        &self,
        source: Pollable,
        request: PlatformRequest,
        limit: Option<Duration>,
        wake: Wake,
    ) -> Result<(), Box<(PlatformRequest, Wake)>> {
        let mut st = self.state.lock().unwrap();
        let key = st.next_key;
        st.next_key += 1;
        if source.add(&self.poller, key).is_err() {
            return Err(Box::new((request, wake)));
        }
        let entry = Entry::Readable { source, request, wake };
        self.place(&mut st, key, entry, limit);
        drop(st);
        let _ = self.poller.notify();
        Ok(())
    }

    pub(crate) fn ticker(&self, period: Duration, sink: EventSink) -> usize {
        self.insert(Entry::Ticker { sink, period, count: 0 }, Some(period))
    }

    pub(crate) fn cancel(&self, key: usize) {
        self.state.lock().unwrap().slots.remove(&key);
    }

    fn insert(&self, entry: Entry, limit: Option<Duration>) -> usize {
        let mut st = self.state.lock().unwrap();
        let key = st.next_key;
        st.next_key += 1;
        self.place(&mut st, key, entry, limit);
        drop(st);
        let _ = self.poller.notify();
        key
    }

    fn place(&self, st: &mut State, key: usize, entry: Entry, limit: Option<Duration>) {
        let deadline = limit.map(|d| after(Instant::now(), d));
        if let Some(at) = deadline {
            st.timers.push(Reverse((at, key)));
        }
        st.slots.insert(key, Slot { entry, deadline });
    }

    fn run(&self) {
        let mut events = Events::new();
        while !self.shutdown.load(Ordering::SeqCst) {
            let timeout = self.next_timeout();
            events.clear();
            let _ = self.poller.wait(&mut events, timeout);
            let mut wakes: Vec<(Wake, Woken)> = Vec::new();
            {
                let mut st = self.state.lock().unwrap();
                for event in events.iter() {
                    if let Some(Slot { entry: Entry::Readable { source, request, wake, .. }, .. }) = st.slots.remove(&event.key) {
                        source.delete(&self.poller);
                        wakes.push((wake, Woken::Run(request)));
                    }
                }
                self.expire(&mut st, &mut wakes);
            }
            for (wake, woken) in wakes {
                wake(woken);
            }
        }
    }

    fn next_timeout(&self) -> Option<Duration> {
        let mut st = self.state.lock().unwrap();
        while let Some(&Reverse((at, key))) = st.timers.peek() {
            if st.slots.get(&key).is_some_and(|s| s.deadline == Some(at)) {
                return Some(at.saturating_duration_since(Instant::now()));
            }
            st.timers.pop();
        }
        None
    }

    fn expire(&self, st: &mut State, wakes: &mut Vec<(Wake, Woken)>) {
        let now = Instant::now();
        while let Some(&Reverse((at, key))) = st.timers.peek() {
            if at > now {
                break;
            }
            st.timers.pop();
            let current = st.slots.get(&key).is_some_and(|s| s.deadline == Some(at));
            if !current {
                continue;
            }
            let Slot { entry, .. } = st.slots.remove(&key).expect("a current timer has a slot");
            match entry {
                Entry::Sleep { wake } => wakes.push((wake, Woken::Done(Ok(PlatformResponse::Unit)))),
                Entry::Readable { source, wake, .. } => {
                    source.delete(&self.poller);
                    wakes.push((wake, Woken::Done(Err(timed_out()))));
                }
                Entry::Ticker { sink, period, count } => {
                    let (count, next) = match sink.try_push(EventPayload::Int(count + 1)) {
                        PushResult::Queued | PushResult::Coalesced => (count + 1, period),
                        PushResult::Full => (count, TICK_RETRY),
                        PushResult::Closed => continue,
                    };
                    let deadline = after(now, next);
                    st.timers.push(Reverse((deadline, key)));
                    st.slots.insert(key, Slot { entry: Entry::Ticker { sink, period, count }, deadline: Some(deadline) });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::mpsc::{channel, Receiver};

    use contracts::{event_queue, Overflow};

    use super::*;

    fn wake() -> (Wake, Receiver<Woken>) {
        let (tx, rx) = channel();
        (Box::new(move |woken| tx.send(woken).unwrap()), rx)
    }

    fn wait(rx: &Receiver<Woken>) -> Woken {
        rx.recv_timeout(Duration::from_secs(5)).expect("the wait ended")
    }

    #[test]
    fn sleeps_end_in_order_of_their_deadlines() {
        let reactor = Reactor::start().unwrap();
        let (slow, slow_rx) = wake();
        let (fast, fast_rx) = wake();
        reactor.sleep(Duration::from_millis(200), slow);
        reactor.sleep(Duration::from_millis(20), fast);
        assert!(matches!(wait(&fast_rx), Woken::Done(Ok(PlatformResponse::Unit))));
        assert!(slow_rx.try_recv().is_err());
        assert!(matches!(wait(&slow_rx), Woken::Done(Ok(PlatformResponse::Unit))));
        reactor.shutdown();
    }

    #[test]
    fn a_readable_socket_runs_its_request_and_a_quiet_one_times_out() {
        let reactor = Reactor::start().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        let server = Arc::new(server);
        let request = || PlatformRequest::SocketRead { id: 1, max: 8 };

        let (quiet, quiet_rx) = wake();
        assert!(reactor.readable(Pollable::Stream(server.clone()), request(), Some(Duration::from_millis(30)), quiet).is_ok());
        let Woken::Done(Err(e)) = wait(&quiet_rx) else { panic!("not a timeout") };
        assert_eq!(e.kind, PlatformErrorKind::TimedOut);

        let (ready, ready_rx) = wake();
        assert!(reactor.readable(Pollable::Stream(server.clone()), request(), None, ready).is_ok());
        client.write_all(b"x").unwrap();
        assert!(matches!(wait(&ready_rx), Woken::Run(PlatformRequest::SocketRead { .. })));
        reactor.shutdown();
    }

    #[test]
    fn a_socket_already_readable_runs_at_once() {
        let reactor = Reactor::start().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        client.write_all(b"x").unwrap();
        let (ready, ready_rx) = wake();
        assert!(reactor.readable(Pollable::Stream(Arc::new(server)), PlatformRequest::SocketRead { id: 1, max: 8 }, None, ready).is_ok());
        assert!(matches!(wait(&ready_rx), Woken::Run(_)));
        reactor.shutdown();
    }

    #[test]
    fn a_ticker_pushes_until_cancelled() {
        let reactor = Reactor::start().unwrap();
        let (sink, queue) = event_queue(64, Overflow::Pause);
        let key = reactor.ticker(Duration::from_millis(10), sink);
        std::thread::sleep(Duration::from_millis(120));
        reactor.cancel(key);
        std::thread::sleep(Duration::from_millis(30));
        let seen = queue.len();
        assert!((3..=12).contains(&seen), "{seen} ticks");
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(queue.len(), seen);
        reactor.shutdown();
    }
}
