//! The clock, real or virtual, and the hierarchical timer wheel that fires tokens at their deadlines.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Nanoseconds in one tick of the wheel; a deadline is rounded up to a tick.
pub const TICK_NS: u64 = 1_000_000;

const SLOT_BITS: u32 = 6;
const SLOTS: usize = 1 << SLOT_BITS;
const LEVELS: usize = 4;

/// Nanoseconds since the scheduler started, from the system or advanced by hand.
pub enum Clock {
    Real(Instant),
    Virtual(AtomicU64),
}

impl Clock {
    pub fn real() -> Self {
        Clock::Real(Instant::now())
    }

    pub fn virtual_time() -> Self {
        Clock::Virtual(AtomicU64::new(0))
    }

    pub fn now(&self) -> u64 {
        match self {
            Clock::Real(start) => start.elapsed().as_nanos() as u64,
            Clock::Virtual(now) => now.load(Ordering::SeqCst),
        }
    }

    pub fn is_virtual(&self) -> bool {
        matches!(self, Clock::Virtual(_))
    }

    /// Moves a virtual clock forward to `ns`; it never goes back, and a real clock ignores this.
    pub fn advance_to(&self, ns: u64) {
        if let Clock::Virtual(now) = self {
            now.fetch_max(ns, Ordering::SeqCst);
        }
    }
}

/// A timer in a wheel, for cancelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TimerId(u64);

struct Entry<T> {
    id: u64,
    tick: u64,
    token: T,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Due,
    Slot(usize, usize),
    Overflow,
}

/// Four levels of 64 slots, the first of one tick each and every next 64 times wider; later deadlines wait in an overflow list.
pub struct Wheel<T> {
    now_tick: u64,
    due: Vec<Entry<T>>,
    slots: Vec<Vec<Entry<T>>>,
    overflow: Vec<Entry<T>>,
    places: HashMap<u64, Place>,
    next_id: u64,
}

impl<T> Wheel<T> {
    pub fn new(now_ns: u64) -> Self {
        Self {
            now_tick: now_ns / TICK_NS,
            due: Vec::new(),
            slots: (0..LEVELS * SLOTS).map(|_| Vec::new()).collect(),
            overflow: Vec::new(),
            places: HashMap::new(),
            next_id: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.places.len()
    }

    pub fn is_empty(&self) -> bool {
        self.places.is_empty()
    }

    /// Arms a timer that fires once `deadline_ns` has passed.
    pub fn insert(&mut self, deadline_ns: u64, token: T) -> TimerId {
        let id = self.next_id;
        self.next_id += 1;
        self.place(Entry { id, tick: deadline_ns.div_ceil(TICK_NS), token });
        TimerId(id)
    }

    /// Disarms a timer; its token, or `None` when it already fired.
    pub fn cancel(&mut self, id: TimerId) -> Option<T> {
        let place = self.places.remove(&id.0)?;
        let list = match place {
            Place::Due => &mut self.due,
            Place::Slot(level, slot) => &mut self.slots[level * SLOTS + slot],
            Place::Overflow => &mut self.overflow,
        };
        let at = list.iter().position(|entry| entry.id == id.0).expect("a timer is where the wheel says");
        Some(list.swap_remove(at).token)
    }

    fn place(&mut self, entry: Entry<T>) {
        let now = self.now_tick;
        if entry.tick <= now {
            self.places.insert(entry.id, Place::Due);
            self.due.push(entry);
            return;
        }
        for level in 0..LEVELS {
            let shift = SLOT_BITS * level as u32;
            if (entry.tick >> shift) - (now >> shift) < SLOTS as u64 {
                let slot = ((entry.tick >> shift) as usize) & (SLOTS - 1);
                self.places.insert(entry.id, Place::Slot(level, slot));
                self.slots[level * SLOTS + slot].push(entry);
                return;
            }
        }
        self.places.insert(entry.id, Place::Overflow);
        self.overflow.push(entry);
    }

    /// The earliest tick any timer is armed for.
    fn next_tick(&self) -> Option<u64> {
        let mut best = self.due.iter().map(|entry| entry.tick).min();
        for level in 0..LEVELS {
            let shift = SLOT_BITS * level as u32;
            let start = ((self.now_tick >> shift) as usize) & (SLOTS - 1);
            let first = (0..SLOTS).map(|step| &self.slots[level * SLOTS + (start + step) % SLOTS]).find(|list| !list.is_empty());
            if let Some(tick) = first.and_then(|list| list.iter().map(|entry| entry.tick).min()) {
                best = Some(best.map_or(tick, |best| best.min(tick)));
            }
        }
        if let Some(tick) = self.overflow.iter().map(|entry| entry.tick).min() {
            best = Some(best.map_or(tick, |best| best.min(tick)));
        }
        best
    }

    /// The nanosecond the earliest timer is armed for, rounded to its tick.
    pub fn next_deadline(&self) -> Option<u64> {
        self.next_tick().map(|tick| tick.max(self.now_tick) * TICK_NS)
    }

    /// Moves the wheel to `tick` and returns what fires there, cascading the slots of the wider levels.
    fn land(&mut self, tick: u64, fired: &mut Vec<Entry<T>>) {
        self.now_tick = tick;
        for level in (1..LEVELS).rev() {
            let slot = ((tick >> (SLOT_BITS * level as u32)) as usize) & (SLOTS - 1);
            for entry in std::mem::take(&mut self.slots[level * SLOTS + slot]) {
                self.places.remove(&entry.id);
                self.place(entry);
            }
        }
        for entry in std::mem::take(&mut self.overflow) {
            self.places.remove(&entry.id);
            self.place(entry);
        }
        fired.append(&mut self.due);
        for entry in std::mem::take(&mut self.slots[(tick as usize) & (SLOTS - 1)]) {
            fired.push(entry);
        }
    }

    /// Moves time to `now_ns`: the tokens of every timer whose deadline has passed, earliest first.
    pub fn advance(&mut self, now_ns: u64) -> Vec<T> {
        let target = (now_ns / TICK_NS).max(self.now_tick);
        let mut fired = Vec::new();
        while let Some(next) = self.next_tick() {
            if next > target {
                break;
            }
            self.land(next.max(self.now_tick), &mut fired);
        }
        if target > self.now_tick {
            self.land(target, &mut fired);
        }
        fired.sort_by_key(|entry| (entry.tick, entry.id));
        for entry in &fired {
            self.places.remove(&entry.id);
        }
        fired.into_iter().map(|entry| entry.token).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self, below: u64) -> u64 {
            self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) % below
        }
    }

    #[test]
    fn timers_a_virtual_clock_moves_only_when_told_and_never_back() {
        let clock = Clock::virtual_time();
        assert_eq!(clock.now(), 0);
        clock.advance_to(5_000);
        clock.advance_to(3_000);
        assert_eq!(clock.now(), 5_000);
        assert!(clock.is_virtual());
    }

    #[test]
    fn timers_a_real_clock_runs_and_ignores_being_advanced() {
        let clock = Clock::real();
        let first = clock.now();
        std::thread::sleep(std::time::Duration::from_millis(2));
        clock.advance_to(u64::MAX);
        let second = clock.now();
        assert!(second >= first + 2_000_000 && second < 10_000_000_000);
        assert!(!clock.is_virtual());
    }

    #[test]
    fn timers_a_timer_fires_once_its_deadline_has_passed_and_not_before() {
        let mut wheel = Wheel::new(0);
        wheel.insert(5 * TICK_NS, "five");
        assert!(wheel.advance(4 * TICK_NS).is_empty());
        assert_eq!(wheel.advance(5 * TICK_NS), ["five"]);
        assert!(wheel.is_empty());
        assert!(wheel.advance(100 * TICK_NS).is_empty());
    }

    #[test]
    fn timers_a_deadline_is_rounded_up_to_the_next_tick() {
        let mut wheel = Wheel::new(0);
        wheel.insert(TICK_NS + 1, "late");
        assert!(wheel.advance(TICK_NS).is_empty());
        assert_eq!(wheel.advance(2 * TICK_NS), ["late"]);
    }

    #[test]
    fn timers_a_deadline_already_past_fires_at_the_next_advance() {
        let mut wheel = Wheel::new(10 * TICK_NS);
        wheel.insert(3 * TICK_NS, "past");
        assert_eq!(wheel.next_deadline(), Some(10 * TICK_NS));
        assert_eq!(wheel.advance(10 * TICK_NS), ["past"]);
    }

    #[test]
    fn timers_many_fire_in_deadline_order_and_ties_in_arming_order() {
        let mut wheel = Wheel::new(0);
        for (deadline, name) in [(30, "c"), (10, "a"), (30, "d"), (20, "b"), (5_000, "f"), (400_000, "g"), (30_000_000, "h")] {
            wheel.insert(deadline * TICK_NS, name);
        }
        assert_eq!(wheel.advance(u64::MAX / 2), ["a", "b", "c", "d", "f", "g", "h"]);
    }

    #[test]
    fn timers_a_cancelled_timer_never_fires_and_a_fired_one_cannot_be_cancelled() {
        let mut wheel = Wheel::new(0);
        let near = wheel.insert(3 * TICK_NS, 1);
        let far = wheel.insert(9_000 * TICK_NS, 2);
        let overflow = wheel.insert(50_000_000 * TICK_NS, 3);
        let due = wheel.insert(0, 4);
        assert_eq!(wheel.len(), 4);
        assert_eq!(wheel.cancel(far), Some(2));
        assert_eq!(wheel.cancel(overflow), Some(3));
        assert_eq!(wheel.cancel(due), Some(4));
        assert_eq!(wheel.cancel(far), None);
        assert_eq!(wheel.advance(u64::MAX / 2), [1]);
        assert_eq!(wheel.cancel(near), None);
        assert!(wheel.is_empty());
    }

    #[test]
    fn timers_the_next_deadline_is_the_earliest_timer_across_the_levels() {
        let mut wheel = Wheel::new(0);
        assert_eq!(wheel.next_deadline(), None);
        wheel.insert(70_000 * TICK_NS, ());
        assert_eq!(wheel.next_deadline(), Some(70_000 * TICK_NS));
        wheel.insert(300 * TICK_NS, ());
        assert_eq!(wheel.next_deadline(), Some(300 * TICK_NS));
        wheel.insert(7 * TICK_NS, ());
        assert_eq!(wheel.next_deadline(), Some(7 * TICK_NS));
    }

    #[test]
    fn timers_a_jump_over_many_levels_cascades_what_is_not_yet_due() {
        let mut wheel = Wheel::new(0);
        wheel.insert(5_000 * TICK_NS, "a");
        wheel.insert(5_100 * TICK_NS, "b");
        assert!(wheel.advance(5_050 * TICK_NS).len() == 1);
        assert_eq!(wheel.next_deadline(), Some(5_100 * TICK_NS));
        assert_eq!(wheel.advance(5_100 * TICK_NS), ["b"]);
    }

    #[test]
    fn timers_the_wheel_agrees_with_a_list_over_random_arming_cancelling_and_jumping() {
        for seed in 1..=20u64 {
            let mut random = Lcg(seed);
            let mut wheel = Wheel::new(0);
            let mut model: Vec<(TimerId, u64, u64)> = Vec::new();
            let mut now = 0u64;
            for step in 0..600 {
                match random.next(10) {
                    0..=4 => {
                        let span = [60, 4_000, 260_000, 20_000_000][random.next(4) as usize];
                        let deadline = now + random.next(span * TICK_NS);
                        let id = wheel.insert(deadline, step);
                        model.push((id, deadline.div_ceil(TICK_NS), step));
                    }
                    5 | 6 if !model.is_empty() => {
                        let at = random.next(model.len() as u64) as usize;
                        let (id, _, token) = model.remove(at);
                        assert_eq!(wheel.cancel(id), Some(token));
                    }
                    _ => {
                        let jump = [1, 3, 70, 5_000, 300_000, 40_000_000][random.next(6) as usize];
                        now += random.next(jump * TICK_NS + 1);
                        let mut expected: Vec<(u64, u64)> = model.iter().filter(|(_, tick, _)| *tick <= now / TICK_NS).map(|(id, tick, token)| (*tick * 1_000_000 + id.0, *token)).collect();
                        expected.sort();
                        model.retain(|(_, tick, _)| *tick > now / TICK_NS);
                        let fired = wheel.advance(now);
                        assert_eq!(fired, expected.iter().map(|(_, token)| *token).collect::<Vec<_>>(), "seed {seed} step {step}");
                    }
                }
                let earliest = model.iter().map(|(_, tick, _)| *tick).min().map(|tick| tick.max(now / TICK_NS) * TICK_NS);
                assert_eq!(wheel.next_deadline(), earliest, "seed {seed} step {step}");
                assert_eq!(wheel.len(), model.len());
            }
        }
    }
}
