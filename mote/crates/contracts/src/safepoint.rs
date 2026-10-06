//! The stop-the-world handshake: mutators check in at safepoints while one of them collects.

use isa::value::ObjectHeader;
use std::ptr::NonNull;

/// A root a mutator reports during a pause.
pub type PausedRoot = NonNull<ObjectHeader>;

/// Coordinates a pause across every entered mutator.
pub trait SafepointCoordinator: Send + Sync {
    /// Joins the set of running mutators; blocks while a pause is in progress.
    fn enter(&self);

    /// Leaves the set; the caller holds no roots the collector cannot reach.
    fn leave(&self);

    /// Leaves the set, handing over `roots` when a pause is in progress (roots only the leaver can reach).
    fn leave_reporting(&self, roots: &mut dyn FnMut() -> Vec<PausedRoot>);

    /// Lock-free: a pause is in progress.
    fn pause_pending(&self) -> bool;

    /// At a safepoint: if a pause is in progress, reports `roots` once per pause and blocks until it ends.
    fn participate(&self, roots: &mut dyn FnMut() -> Vec<PausedRoot>);

    /// Stops every other entered mutator and runs `collect`; returns false, running nothing, when another pause won.
    fn stop_the_world(&self, collect: &mut dyn FnMut()) -> bool;

    /// The roots reported for the pause in progress; call from `collect`.
    fn take_reported_roots(&self) -> Vec<PausedRoot>;
}
