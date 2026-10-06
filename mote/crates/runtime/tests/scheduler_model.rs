//! A model of the scheduler's park/wake protocol, explored over every interleaving of its critical sections.
//! A step is one lock-held section or atomic of `sched.rs`, `handlers/channel.rs`, `handlers/control.rs` and `offload.rs`; the model has no timing, so a state with nothing left to run is a hang.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

const MAX_STATES: usize = 4_000_000;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
enum Role {
    Pool,
    Home,
}

/// What a task does, one operation per `CALLINTRINSIC` / `SCOPEEXIT` / `SPAWN`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum Op {
    Work,
    Spawn(usize),
    ScopeExit,
    Recv(usize),
    Send(usize),
    Join(usize),
    Platform,
    Cancel(usize),
    Pin,
}

/// One atomic step of a worker or an outside thread.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum Micro {
    IncPending(usize),
    Schedule(usize),
    NotifyTask(bool),
    NotifyOne(Role),
    NotifyAllOf(Role),
    WakeEarly(usize, Option<u32>),
    CancelWake(usize),
    UnblockScope(usize),
    CancelChildren(usize),
    SetCancel(usize),
    ParkHandler(usize),
    YieldHandler(usize),
    CompleteHandle(usize),
    ParentUpdate(usize, bool),
    Recycle(usize),
    FinishMain,
    SignalDone,
    OffloadComplete(usize),
    SourcePush(usize),
    SourceWake(usize),
    Exit,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum TState {
    NotStarted,
    Ready,
    Running,
    Parking,
    Blocked,
    Done,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Task {
    pc: usize,
    state: TState,
    seq: u32,
    cancelled: bool,
    pinned: bool,
    pending: usize,
    scope_parked: bool,
    finished: bool,
    waiter: Option<(usize, u32)>,
    awaiting_platform: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum WState {
    Loop,
    Waiting,
    Woken,
    Run(usize),
    Exited,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Worker {
    role: Role,
    state: WState,
    micros: Vec<Micro>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Who {
    Worker(usize),
    Ext(usize),
}

struct Scenario {
    name: &'static str,
    progs: Vec<Vec<Op>>,
    parents: Vec<Option<usize>>,
    channels: Vec<usize>,
    workers: Vec<Role>,
    exts: Vec<Vec<Micro>>,
    preempt: bool,
    spurious: bool,
    /// Channels fed by an event source: a receiver parked on one keeps the run from counting as a deadlock.
    sources: Vec<usize>,
    /// The scope wake as it was before the fix: `unblock_or_note_early` from `complete_child`, with no re-check as the parent parks.
    legacy_scope_wake: bool,
    /// A cancelled task leaving `recv` or `send` does not hand its wake on, as before the fix.
    legacy_no_forward: bool,
    /// The program itself may leave a receiver without an item, so a deadlock with nothing buffered is allowed; a buffered item with a receiver parked is still a lost wake.
    may_starve: bool,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct World {
    tasks: Vec<Task>,
    workers: Vec<Worker>,
    exts: Vec<Vec<Micro>>,
    run_queue: Vec<usize>,
    home_queue: Vec<usize>,
    blocked: BTreeSet<usize>,
    woken_early: BTreeMap<usize, Option<u32>>,
    recv_waiters: Vec<Vec<(usize, u32)>>,
    send_waiters: Vec<Vec<(usize, u32)>>,
    buffered: Vec<usize>,
    pool_cv: BTreeSet<usize>,
    home_cv: BTreeSet<usize>,
    idle_count: usize,
    offload_inflight: usize,
    offload_pending: BTreeSet<usize>,
    offload_done: BTreeSet<usize>,
    done: bool,
    deadlock_declared: bool,
    /// A hard violation; ends the run.
    violation: Option<String>,
    /// A wake that reached a task not waiting where the wake was sent from; not a hang by itself.
    stale: Option<String>,
}

type Step = (World, String);

impl World {
    fn new(sc: &Scenario) -> World {
        let task = |state| Task { pc: 0, state, seq: 0, cancelled: false, pinned: false, pending: 0, scope_parked: false, finished: false, waiter: None, awaiting_platform: false };
        let mut tasks = vec![task(TState::NotStarted); sc.progs.len()];
        tasks[0].state = TState::Ready;
        World {
            tasks,
            workers: sc.workers.iter().map(|&role| Worker { role, state: WState::Loop, micros: Vec::new() }).collect(),
            exts: sc.exts.clone(),
            run_queue: vec![0],
            home_queue: Vec::new(),
            blocked: BTreeSet::new(),
            woken_early: BTreeMap::new(),
            recv_waiters: vec![Vec::new(); sc.channels.len()],
            send_waiters: vec![Vec::new(); sc.channels.len()],
            buffered: vec![0; sc.channels.len()],
            pool_cv: BTreeSet::new(),
            home_cv: BTreeSet::new(),
            idle_count: 0,
            offload_inflight: 0,
            offload_pending: BTreeSet::new(),
            offload_done: BTreeSet::new(),
            done: false,
            deadlock_declared: false,
            violation: None,
            stale: None,
        }
    }

    fn push_front(&mut self, who: Who, micros: Vec<Micro>) {
        let queue = match who {
            Who::Worker(i) => &mut self.workers[i].micros,
            Who::Ext(i) => &mut self.exts[i],
        };
        queue.splice(0..0, micros);
    }

    fn enqueue(&mut self, t: usize) {
        self.tasks[t].state = TState::Ready;
        if self.tasks[t].pinned {
            self.home_queue.push(t);
        } else {
            self.run_queue.push(t);
        }
    }

    fn has_work_for(&self, role: Role) -> bool {
        match role {
            Role::Pool => !self.run_queue.is_empty(),
            Role::Home => !self.home_queue.is_empty(),
        }
    }

    fn pop_for(&mut self, role: Role) -> Option<usize> {
        let queue = if role == Role::Home { &mut self.home_queue } else { &mut self.run_queue };
        if queue.is_empty() {
            None
        } else {
            Some(queue.remove(0))
        }
    }

    /// `SchedState::forget_waiter`: drops task `id`'s entries from every waiter list.
    fn forget_waiter(&mut self, id: usize) {
        for list in self.recv_waiters.iter_mut().chain(self.send_waiters.iter_mut()) {
            list.retain(|&(w, _)| w != id);
        }
    }

    fn note_stale(&mut self, text: String) {
        if self.stale.is_none() {
            self.stale = Some(text);
        }
    }

    /// Whether a wake carrying `token` addresses what `id` is waiting on now.
    fn wake_is_current(&self, id: usize, token: Option<u32>) -> bool {
        let t = &self.tasks[id];
        let parked = matches!(t.state, TState::Parking | TState::Blocked);
        match token {
            Some(s) => parked && t.seq == s,
            None => parked && t.scope_parked,
        }
    }

    /// `Runtime::sources_can_wake`: a registered source with a receiver parked on its channel.
    fn sources_can_wake(&self, sc: &Scenario) -> bool {
        sc.sources.iter().any(|&c| !self.recv_waiters[c].is_empty())
    }

    fn all_done(&self) -> bool {
        self.tasks.iter().all(|t| t.state == TState::Done)
    }

    fn describe(&self) -> String {
        let tasks: Vec<String> = self.tasks.iter().enumerate().map(|(i, t)| format!("t{i}:{:?}@{}", t.state, t.pc)).collect();
        let workers: Vec<String> = self.workers.iter().map(|w| format!("{:?}{:?}", w.role, w.state)).collect();
        format!("tasks [{}], workers [{}], run {:?}, home {:?}, blocked {:?}, early {:?}, recv {:?}", tasks.join(" "), workers.join(" "), self.run_queue, self.home_queue, self.blocked, self.woken_early, self.recv_waiters)
    }

    /// `SchedState::wake_or_note_early`: moves a parked task to its ready queue and answers whether it was pinned, or notes the wake for a park in flight.
    fn wake_quiet(&mut self, id: usize, token: Option<u32>) -> Option<bool> {
        // A wake popped before a cancel landed reaches a task that will fault; it forwards the wake and recycle drops the note.
        if !self.wake_is_current(id, token) && !self.tasks[id].cancelled {
            self.note_stale(format!("a wake for task {id} (token {token:?}) reached it while {:?} (seq {})", self.tasks[id].state, self.tasks[id].seq));
        }
        if self.blocked.remove(&id) {
            let pinned = self.tasks[id].pinned;
            self.enqueue(id);
            Some(pinned)
        } else {
            self.woken_early.entry(id).or_insert(token);
            None
        }
    }

    /// `Runtime::unblock_or_note_early`: the wake, then a notification outside the lock if it moved a task.
    fn wake_or_note_early(&mut self, id: usize, token: Option<u32>, who: Who) {
        if let Some(pinned) = self.wake_quiet(id, token) {
            self.push_front(who, vec![Micro::NotifyTask(pinned)]);
        }
    }

    fn successors(&self, sc: &Scenario) -> Vec<Step> {
        let mut out = Vec::new();
        for wi in 0..self.workers.len() {
            match self.workers[wi].state {
                WState::Exited => {}
                WState::Waiting => {
                    if sc.spurious {
                        let mut w = self.clone();
                        w.pool_cv.remove(&wi);
                        w.home_cv.remove(&wi);
                        w.workers[wi].state = WState::Woken;
                        out.push((w, format!("w{wi}: spurious wakeup")));
                    }
                }
                _ => self.worker_step(sc, wi, &mut out),
            }
        }
        for ei in 0..self.exts.len() {
            let Some(front) = self.exts[ei].first().cloned() else { continue };
            if let Micro::OffloadComplete(t) = front
                && !self.offload_pending.contains(&t)
            {
                continue;
            }
            let mut w = self.clone();
            w.exts[ei].remove(0);
            w.run_micro(sc, Who::Ext(ei), front.clone(), &mut out, format!("e{ei}: {front:?}"));
        }
        out
    }

    fn worker_step(&self, sc: &Scenario, wi: usize, out: &mut Vec<Step>) {
        let who = Who::Worker(wi);
        if !self.workers[wi].micros.is_empty() {
            let mut w = self.clone();
            let m = w.workers[wi].micros.remove(0);
            w.run_micro(sc, who, m.clone(), out, format!("w{wi}: {m:?}"));
            return;
        }
        let role = self.workers[wi].role;
        match self.workers[wi].state {
            WState::Woken => {
                let mut w = self.clone();
                w.idle_count -= 1;
                w.workers[wi].state = WState::Loop;
                out.push((w, format!("w{wi}: wakes and leaves the idle count")));
            }
            WState::Loop => self.pop(sc, wi, role, out),
            WState::Run(t) => {
                self.exec_op(sc, wi, t, out);
                if sc.preempt && self.tasks[t].pc < sc.progs[t].len() {
                    let mut w = self.clone();
                    w.workers[wi].micros = vec![Micro::YieldHandler(t)];
                    out.push((w, format!("w{wi}: t{t} spends its budget")));
                }
            }
            WState::Waiting | WState::Exited => {}
        }
    }

    /// The top of `drive_loop`: take a task, or count as idle and wait.
    fn pop(&self, sc: &Scenario, wi: usize, role: Role, out: &mut Vec<Step>) {
        let mut w = self.clone();
        if w.done {
            w.workers[wi].state = WState::Exited;
            out.push((w, format!("w{wi}: sees done and returns")));
            return;
        }
        if let Some(t) = w.pop_for(role) {
            w.tasks[t].state = TState::Running;
            w.workers[wi].state = WState::Run(t);
            out.push((w, format!("w{wi}: takes t{t}")));
            return;
        }
        w.idle_count += 1;
        let nothing_ready = w.run_queue.is_empty() && w.home_queue.is_empty();
        if w.idle_count == sc.workers.len() && nothing_ready && w.offload_inflight == 0 && !w.sources_can_wake(sc) {
            w.done = true;
            w.deadlock_declared = true;
            w.idle_count -= 1;
            w.workers[wi].micros = vec![Micro::NotifyAllOf(Role::Home), Micro::NotifyAllOf(Role::Pool), Micro::Exit];
            out.push((w, format!("w{wi}: declares a deadlock")));
            return;
        }
        if role == Role::Home {
            w.home_cv.insert(wi);
        } else {
            w.pool_cv.insert(wi);
        }
        w.workers[wi].state = WState::Waiting;
        out.push((w, format!("w{wi}: goes idle and waits")));
    }

    /// Runs task `t`'s next operation on worker `wi`.
    fn exec_op(&self, sc: &Scenario, wi: usize, t: usize, out: &mut Vec<Step>) {
        let who = Who::Worker(wi);
        let mut w = self.clone();
        let task = &self.tasks[t];
        let label = |what: &str| format!("w{wi}: t{t} {what}");
        let Some(op) = sc.progs[t].get(task.pc).cloned() else {
            let ms = if t == 0 {
                vec![Micro::FinishMain]
            } else {
                vec![Micro::CompleteHandle(t), Micro::ParentUpdate(t, false), Micro::Recycle(t)]
            };
            w.workers[wi].micros = ms;
            out.push((w, label("halts")));
            return;
        };
        let faulting = task.cancelled && matches!(op, Op::Recv(_) | Op::Send(_) | Op::Join(_));
        if faulting {
            let mut micros = Vec::new();
            if !sc.legacy_no_forward {
                let next = match op {
                    Op::Recv(c) if w.buffered[c] > 0 && !w.recv_waiters[c].is_empty() => Some(w.recv_waiters[c].remove(0)),
                    Op::Send(c) if w.buffered[c] < sc.channels[c] && !w.send_waiters[c].is_empty() => Some(w.send_waiters[c].remove(0)),
                    _ => None,
                };
                micros.extend(next.map(|(id, s)| Micro::WakeEarly(id, Some(s))));
            }
            if t == 0 {
                micros.push(Micro::FinishMain);
            } else {
                micros.extend([Micro::CompleteHandle(t), Micro::ParentUpdate(t, true), Micro::Recycle(t)]);
            }
            w.workers[wi].micros = micros;
            out.push((w, label("sees it was cancelled and faults")));
            return;
        }
        let park = |w: &mut World| {
            w.tasks[t].state = TState::Parking;
            w.workers[wi].micros = vec![Micro::ParkHandler(t)];
        };
        match op {
            Op::Work => {
                w.tasks[t].pc += 1;
                out.push((w, label("works")));
            }
            Op::Spawn(c) => {
                w.tasks[t].pc += 1;
                w.workers[wi].micros = vec![Micro::IncPending(t), Micro::Schedule(c), Micro::NotifyTask(false)];
                out.push((w, label(&format!("spawns t{c}"))));
            }
            Op::ScopeExit => {
                if w.tasks[t].pending > 0 {
                    w.tasks[t].scope_parked = true;
                    park(&mut w);
                    out.push((w, label("parks at the scope exit")));
                } else {
                    w.tasks[t].scope_parked = false;
                    w.tasks[t].pc += 1;
                    out.push((w, label("leaves its scope")));
                }
            }
            Op::Recv(c) => {
                if w.buffered[c] > 0 {
                    w.buffered[c] -= 1;
                    w.tasks[t].pc += 1;
                    if !w.send_waiters[c].is_empty() {
                        let (id, s) = w.send_waiters[c].remove(0);
                        w.workers[wi].micros = vec![Micro::WakeEarly(id, Some(s))];
                    }
                    out.push((w, label(&format!("receives from c{c}"))));
                } else {
                    w.tasks[t].seq += 1;
                    let s = w.tasks[t].seq;
                    w.recv_waiters[c].push((t, s));
                    park(&mut w);
                    out.push((w, label(&format!("parks on c{c}"))));
                }
            }
            Op::Send(c) => {
                if w.buffered[c] == sc.channels[c] {
                    w.tasks[t].seq += 1;
                    let s = w.tasks[t].seq;
                    w.send_waiters[c].push((t, s));
                    park(&mut w);
                    out.push((w, label(&format!("parks sending on c{c}"))));
                } else {
                    w.buffered[c] += 1;
                    w.tasks[t].pc += 1;
                    if !w.recv_waiters[c].is_empty() {
                        let (id, s) = w.recv_waiters[c].remove(0);
                        w.workers[wi].micros = vec![Micro::WakeEarly(id, Some(s))];
                    }
                    out.push((w, label(&format!("sends on c{c}"))));
                }
            }
            Op::Join(h) => {
                if w.tasks[h].finished {
                    w.tasks[t].pc += 1;
                    out.push((w, label(&format!("joins t{h}"))));
                } else {
                    w.tasks[t].seq += 1;
                    let s = w.tasks[t].seq;
                    w.tasks[h].waiter = Some((t, s));
                    park(&mut w);
                    out.push((w, label(&format!("parks joining t{h}"))));
                }
            }
            Op::Platform => {
                if w.tasks[t].awaiting_platform {
                    if w.offload_done.remove(&t) {
                        w.tasks[t].awaiting_platform = false;
                        w.tasks[t].pc += 1;
                        out.push((w, label("collects its platform response")));
                    } else {
                        park(&mut w);
                        out.push((w, label("parks again awaiting the platform")));
                    }
                } else {
                    w.tasks[t].seq += 1;
                    w.tasks[t].awaiting_platform = true;
                    w.offload_pending.insert(t);
                    w.offload_inflight += 1;
                    park(&mut w);
                    out.push((w, label("submits a blocking request and parks")));
                }
            }
            Op::Cancel(x) => {
                w.tasks[t].pc += 1;
                w.workers[wi].micros = vec![Micro::SetCancel(x), Micro::CancelWake(x)];
                out.push((w, label(&format!("cancels t{x}"))));
            }
            Op::Pin => {
                w.tasks[t].pc += 1;
                w.tasks[t].pinned = true;
                w.workers[wi].micros = vec![Micro::YieldHandler(t)];
                out.push((w, label("pins itself")));
            }
        }
        let _ = who;
    }

    /// Runs one atomic step of a worker or an outside thread; the step's result is pushed on `out`.
    fn run_micro(mut self, sc: &Scenario, who: Who, m: Micro, out: &mut Vec<Step>, label: String) {
        match m {
            Micro::IncPending(t) => self.tasks[t].pending += 1,
            Micro::Schedule(c) => self.enqueue(c),
            Micro::NotifyTask(pinned) => {
                let mut next = Vec::new();
                if pinned {
                    next.push(Micro::NotifyOne(Role::Home));
                }
                next.push(Micro::NotifyOne(Role::Pool));
                self.push_front(who, next);
            }
            Micro::NotifyOne(role) => {
                let waiting: Vec<usize> = if role == Role::Home { self.home_cv.iter().copied().collect() } else { self.pool_cv.iter().copied().collect() };
                if waiting.is_empty() {
                    out.push((self, label));
                    return;
                }
                for wi in waiting {
                    let mut w = self.clone();
                    w.pool_cv.remove(&wi);
                    w.home_cv.remove(&wi);
                    w.workers[wi].state = WState::Woken;
                    out.push((w, format!("{label} (wakes w{wi})")));
                }
                return;
            }
            Micro::NotifyAllOf(role) => {
                let waiting: Vec<usize> = if role == Role::Home { std::mem::take(&mut self.home_cv).into_iter().collect() } else { std::mem::take(&mut self.pool_cv).into_iter().collect() };
                for wi in waiting {
                    self.workers[wi].state = WState::Woken;
                }
            }
            Micro::WakeEarly(id, token) => self.wake_or_note_early(id, token, who),
            Micro::CancelWake(id) => {
                if self.blocked.remove(&id) {
                    let pinned = self.tasks[id].pinned;
                    self.enqueue(id);
                    self.push_front(who, vec![Micro::NotifyTask(pinned)]);
                }
            }
            Micro::UnblockScope(id) => {
                if self.blocked.contains(&id) && self.tasks[id].scope_parked {
                    self.blocked.remove(&id);
                    let pinned = self.tasks[id].pinned;
                    self.enqueue(id);
                    self.push_front(who, vec![Micro::NotifyTask(pinned)]);
                }
            }
            Micro::SetCancel(x) => {
                self.tasks[x].cancelled = true;
                self.forget_waiter(x);
            }
            Micro::CancelChildren(p) => {
                let kids: Vec<usize> = (0..self.tasks.len()).filter(|&c| sc.parents[c] == Some(p) && self.tasks[c].state != TState::Done && self.tasks[c].state != TState::NotStarted && !self.tasks[c].cancelled).collect();
                let mut next = Vec::new();
                for c in kids {
                    self.tasks[c].cancelled = true;
                    self.forget_waiter(c);
                    next.push(Micro::CancelWake(c));
                }
                self.push_front(who, next);
            }
            Micro::ParkHandler(t) => {
                let Who::Worker(wi) = who else { unreachable!() };
                let cancelled = !self.tasks[t].awaiting_platform && self.tasks[t].cancelled;
                let scope_done = !sc.legacy_scope_wake && self.tasks[t].scope_parked && self.tasks[t].pending == 0;
                if cancelled {
                    self.forget_waiter(t);
                }
                let early = self.woken_early.remove(&t);
                if let Some(token) = early
                    && !self.wake_is_current(t, token)
                {
                    self.note_stale(format!("an early wake {token:?} ended an unrelated park of task {t} (seq {})", self.tasks[t].seq));
                }
                if early.is_some() || cancelled || scope_done {
                    let pinned = self.tasks[t].pinned;
                    self.enqueue(t);
                    self.push_front(who, vec![Micro::NotifyTask(pinned)]);
                } else {
                    self.tasks[t].state = TState::Blocked;
                    self.blocked.insert(t);
                }
                self.workers[wi].state = WState::Loop;
            }
            Micro::YieldHandler(t) => {
                let Who::Worker(wi) = who else { unreachable!() };
                let role = self.workers[wi].role;
                let runs = (role == Role::Home) == self.tasks[t].pinned;
                if runs && !self.has_work_for(role) {
                    // keeps running
                } else {
                    let pinned = self.tasks[t].pinned;
                    self.enqueue(t);
                    self.push_front(who, vec![Micro::NotifyTask(pinned)]);
                    self.workers[wi].state = WState::Loop;
                }
            }
            Micro::CompleteHandle(c) => {
                self.tasks[c].finished = true;
                if let Some((id, s)) = self.tasks[c].waiter.take() {
                    self.push_front(who, vec![Micro::WakeEarly(id, Some(s))]);
                }
            }
            Micro::ParentUpdate(c, faulted) => {
                if let Some(p) = sc.parents[c] {
                    self.tasks[p].pending = self.tasks[p].pending.saturating_sub(1);
                    let wake = self.tasks[p].pending == 0;
                    let mut next = Vec::new();
                    if faulted {
                        next.push(Micro::CancelChildren(p));
                    }
                    if wake {
                        next.push(if sc.legacy_scope_wake { Micro::WakeEarly(p, None) } else { Micro::UnblockScope(p) });
                    }
                    self.push_front(who, next);
                }
            }
            Micro::Recycle(t) => {
                let Who::Worker(wi) = who else { unreachable!() };
                if self.tasks[t].cancelled {
                    for list in self.recv_waiters.iter_mut().chain(self.send_waiters.iter_mut()) {
                        list.retain(|&(id, _)| id != t);
                    }
                }
                self.woken_early.remove(&t);
                self.tasks[t].state = TState::Done;
                self.workers[wi].state = WState::Loop;
            }
            Micro::FinishMain => {
                let Who::Worker(wi) = who else { unreachable!() };
                self.tasks[0].state = TState::Done;
                self.workers[wi].micros = vec![Micro::SignalDone, Micro::NotifyAllOf(Role::Home), Micro::NotifyAllOf(Role::Pool), Micro::Exit];
            }
            Micro::SignalDone => self.done = true,
            Micro::OffloadComplete(t) => {
                self.offload_pending.remove(&t);
                self.offload_done.insert(t);
                self.offload_inflight -= 1;
                let token = Some(self.tasks[t].seq);
                self.wake_quiet(t, token);
                self.push_front(who, vec![Micro::NotifyAllOf(Role::Home), Micro::NotifyAllOf(Role::Pool)]);
            }
            Micro::SourcePush(c) => {
                self.buffered[c] += 1;
                self.push_front(who, vec![Micro::SourceWake(c)]);
            }
            Micro::SourceWake(c) => {
                for (id, s) in std::mem::take(&mut self.recv_waiters[c]) {
                    self.wake_quiet(id, Some(s));
                }
                self.push_front(who, vec![Micro::NotifyAllOf(Role::Home), Micro::NotifyAllOf(Role::Pool)]);
            }
            Micro::Exit => {
                let Who::Worker(wi) = who else { unreachable!() };
                self.workers[wi].state = WState::Exited;
            }
        }
        out.push((self, label));
    }
}

struct Report {
    states: usize,
    stale: Option<Vec<String>>,
}

enum Failure {
    Violation { why: String, trace: Vec<String> },
    TooBig(usize),
}

impl Failure {
    fn render(&self) -> String {
        match self {
            Failure::Violation { why, trace } => format!("{why}\n  {}", trace.join("\n  ")),
            Failure::TooBig(n) => format!("more than {n} states"),
        }
    }
}

fn trace_to(index: usize, parent: &[(usize, String)]) -> Vec<String> {
    let mut steps = Vec::new();
    let mut at = index;
    while at != 0 {
        steps.push(parent[at].1.clone());
        at = parent[at].0;
    }
    steps.reverse();
    steps
}

/// Breadth-first over every state; a violation or a stuck state is a failure with the shortest trace to it.
fn explore(sc: &Scenario) -> Result<Report, Failure> {
    let start = World::new(sc);
    let mut seen: HashMap<World, usize> = HashMap::new();
    let mut parent: Vec<(usize, String)> = vec![(0, String::new())];
    let mut queue: VecDeque<(World, usize)> = VecDeque::new();
    seen.insert(start.clone(), 0);
    queue.push_back((start, 0));
    let mut stale: Option<Vec<String>> = None;
    while let Some((world, index)) = queue.pop_front() {
        if stale.is_none() && world.stale.is_some() {
            let mut trace = trace_to(index, &parent);
            trace.push(format!("=> {}", world.stale.clone().unwrap()));
            stale = Some(trace);
        }
        if let Some(why) = &world.violation {
            return Err(Failure::Violation { why: why.clone(), trace: trace_to(index, &parent) });
        }
        let next = world.successors(sc);
        if next.is_empty() {
            let finished = world.done && !world.deadlock_declared && world.all_done() && world.workers.iter().all(|w| w.state == WState::Exited);
            if !finished {
                let lost = world.buffered.iter().zip(&world.recv_waiters).any(|(b, w)| *b > 0 && !w.is_empty());
                if sc.may_starve && world.deadlock_declared && !lost {
                    continue;
                }
                let why = if lost { "LOST WAKE: an item is buffered and a receiver is parked on it" } else if world.deadlock_declared { "a deadlock was declared with work left" } else { "nothing can run and the program has not finished" };
                return Err(Failure::Violation { why: format!("{why}: {}", world.describe()), trace: trace_to(index, &parent) });
            }
            continue;
        }
        for (successor, label) in next {
            if seen.contains_key(&successor) {
                continue;
            }
            if seen.len() >= MAX_STATES {
                return Err(Failure::TooBig(MAX_STATES));
            }
            let id = parent.len();
            parent.push((index, label));
            seen.insert(successor.clone(), id);
            queue.push_back((successor, id));
        }
    }
    Ok(Report { states: seen.len(), stale })
}

fn scenario(name: &'static str, progs: Vec<Vec<Op>>, parents: Vec<Option<usize>>) -> Scenario {
    Scenario { name, progs, parents, channels: Vec::new(), workers: vec![Role::Pool, Role::Pool], exts: Vec::new(), sources: Vec::new(), preempt: false, spurious: false, legacy_scope_wake: false,legacy_no_forward: false, may_starve: false }
}

fn must_hold(sc: &Scenario) -> Report {
    match explore(sc) {
        Ok(report) => {
            eprintln!("{}: {} states", sc.name, report.states);
            report
        }
        Err(f) => panic!("{}: {}", sc.name, f.render()),
    }
}

fn must_be_clean(sc: &Scenario) {
    let report = must_hold(sc);
    if let Some(trace) = report.stale {
        panic!("{}: a stale wake\n  {}", sc.name, trace.join("\n  "));
    }
}

#[test]
fn a_parent_waits_for_one_child() {
    let sc = scenario("one child", vec![vec![Op::Spawn(1), Op::ScopeExit], vec![]], vec![None, Some(0)]);
    must_be_clean(&sc);
}

#[test]
fn a_parent_waits_for_two_children_under_preemption() {
    let mut sc = scenario("two children", vec![vec![Op::Spawn(1), Op::Spawn(2), Op::ScopeExit], vec![Op::Work], vec![]], vec![None, Some(0), Some(0)]);
    sc.preempt = true;
    must_be_clean(&sc);
}

#[test]
fn the_scope_wake_before_the_fix_leaves_a_stale_early_wake() {
    let mut sc = scenario("legacy scope wake", vec![vec![Op::Spawn(1), Op::Work, Op::ScopeExit], vec![]], vec![None, Some(0)]);
    sc.legacy_scope_wake = true;
    sc.preempt = true;
    let report = must_hold(&sc);
    assert!(report.stale.is_some(), "the model should find the stale early wake the old code left");
}

#[test]
fn a_join_wakes_the_joiner_whichever_side_arrives_first() {
    let mut sc = scenario("join", vec![vec![Op::Spawn(1), Op::Join(1), Op::ScopeExit], vec![Op::Work]], vec![None, Some(0)]);
    sc.preempt = true;
    must_be_clean(&sc);
}

#[test]
fn a_receiver_parks_until_a_sender_in_another_task_sends() {
    let mut sc = scenario("recv and send", vec![vec![Op::Spawn(1), Op::Recv(0), Op::ScopeExit], vec![Op::Send(0)]], vec![None, Some(0)]);
    sc.channels = vec![1];
    sc.preempt = true;
    must_be_clean(&sc);
}

#[test]
fn a_full_channel_parks_the_sender_until_a_receive() {
    let mut sc = scenario("full channel", vec![vec![Op::Spawn(1), Op::Recv(0), Op::Recv(0), Op::ScopeExit], vec![Op::Send(0), Op::Send(0)]], vec![None, Some(0)]);
    sc.channels = vec![1];
    sc.preempt = true;
    must_be_clean(&sc);
}

#[test]
fn a_blocking_native_completing_on_another_thread_wakes_its_task() {
    let mut sc = scenario("offload", vec![vec![Op::Platform]], vec![None]);
    sc.exts = vec![vec![Micro::OffloadComplete(0)]];
    must_be_clean(&sc);
}

#[test]
fn a_child_in_a_blocking_native_does_not_look_like_a_deadlock() {
    let mut sc = scenario("offload in a child", vec![vec![Op::Spawn(1), Op::ScopeExit], vec![Op::Platform]], vec![None, Some(0)]);
    sc.exts = vec![vec![Micro::OffloadComplete(1)]];
    sc.preempt = true;
    must_be_clean(&sc);
}

#[test]
fn a_parent_parked_on_a_channel_is_not_woken_by_a_child_finishing() {
    let mut sc = scenario("recv inside a scope", vec![vec![Op::Spawn(1), Op::Recv(0), Op::ScopeExit], vec![]], vec![None, Some(0)]);
    sc.channels = vec![1];
    sc.exts = vec![vec![Micro::SourcePush(0)]];
    sc.sources = vec![0];
    sc.preempt = true;
    must_be_clean(&sc);
}

#[test]
fn two_receivers_share_one_channel() {
    let mut sc = scenario("two receivers", vec![vec![Op::Spawn(1), Op::Spawn(2), Op::ScopeExit], vec![Op::Recv(0)], vec![Op::Recv(0)]], vec![None, Some(0), Some(0)]);
    sc.channels = vec![2];
    sc.exts = vec![vec![Micro::SourcePush(0), Micro::SourcePush(0)]];
    sc.sources = vec![0];
    sc.preempt = true;
    must_be_clean(&sc);
}

#[test]
fn cancelling_a_parked_receiver_leaves_no_waiter_that_eats_a_wake() {
    let mut sc = scenario("cancel a receiver", vec![vec![Op::Spawn(1), Op::Spawn(2), Op::Spawn(3), Op::Cancel(1), Op::ScopeExit], vec![Op::Recv(0)], vec![Op::Recv(0)], vec![Op::Send(0), Op::Send(0)]], vec![None, Some(0), Some(0), Some(0)]);
    sc.channels = vec![2];
    sc.preempt = true;
    must_be_clean(&sc);
}

#[test]
fn a_pinned_task_runs_on_the_home_worker_and_wakes_there() {
    let mut sc = scenario("pinned", vec![vec![Op::Spawn(1), Op::ScopeExit], vec![Op::Pin, Op::Recv(0)]], vec![None, Some(0)]);
    sc.workers = vec![Role::Pool, Role::Home];
    sc.channels = vec![1];
    sc.exts = vec![vec![Micro::SourcePush(0)]];
    sc.sources = vec![0];
    sc.preempt = true;
    must_be_clean(&sc);
}

#[test]
fn spurious_condvar_wakeups_change_nothing() {
    let mut sc = scenario("spurious", vec![vec![Op::Spawn(1), Op::Recv(0), Op::ScopeExit], vec![Op::Send(0)]], vec![None, Some(0)]);
    sc.channels = vec![1];
    sc.spurious = true;
    must_be_clean(&sc);
}

#[test]
fn the_cancel_before_the_fix_loses_a_wake_to_a_cancelled_receiver() {
    let mut sc = scenario("legacy cancel wake", vec![vec![Op::Spawn(1), Op::Spawn(2), Op::Spawn(3), Op::Recv(0), Op::ScopeExit], vec![Op::Recv(0)], vec![Op::Send(0)], vec![Op::Cancel(1)]], vec![None, Some(0), Some(0), Some(0)]);
    sc.channels = vec![2];
    sc.preempt = true;
    sc.may_starve = true;
    sc.legacy_no_forward = true;
    assert!(explore(&sc).is_err(), "the model should find the wake the old cancel lost");
}

#[test]
fn a_send_woken_receiver_that_is_cancelled_passes_the_item_on() {
    let mut sc = scenario("cancelled receiver", vec![vec![Op::Spawn(1), Op::Spawn(2), Op::Spawn(3), Op::Cancel(1), Op::ScopeExit], vec![Op::Recv(0)], vec![Op::Recv(0)], vec![Op::Send(0)]], vec![None, Some(0), Some(0), Some(0)]);
    sc.channels = vec![2];
    sc.preempt = true;
    sc.may_starve = true;
    must_hold(&sc);
}

#[test]
fn a_parent_waiting_on_a_channel_is_not_starved_by_a_cancelled_child_that_was_sent_the_wake() {
    let mut sc = scenario("parent behind a cancelled receiver", vec![vec![Op::Spawn(1), Op::Spawn(2), Op::Spawn(3), Op::Recv(0), Op::ScopeExit], vec![Op::Recv(0)], vec![Op::Send(0)], vec![Op::Cancel(1)]], vec![None, Some(0), Some(0), Some(0)]);
    sc.channels = vec![2];
    sc.preempt = true;
    sc.may_starve = true;
    must_hold(&sc);
}
