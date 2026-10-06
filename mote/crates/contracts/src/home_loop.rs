//! The hook a window system gives the scheduler's home thread, so it can wait in the system's event loop.

use std::sync::{Arc, RwLock};
use std::time::Duration;

pub trait HomeLoop: Send + Sync {
    /// Runs the window system's events on the home thread for up to `timeout` (`None`: until woken), returning early after a [`wake`](Self::wake).
    fn wait(&self, timeout: Option<Duration>);

    /// Makes a `wait` in progress, or the next one, return; any thread may call it.
    fn wake(&self);

    /// Whether a window is open: a program with one is not deadlocked while the home thread waits.
    fn is_open(&self) -> bool;
}

static HOOK: RwLock<Option<Arc<dyn HomeLoop>>> = RwLock::new(None);

/// Installs the process's home loop; the last one installed wins.
pub fn install_home_loop(hook: Arc<dyn HomeLoop>) {
    *HOOK.write().unwrap_or_else(|e| e.into_inner()) = Some(hook);
}

/// The installed home loop, if any.
pub fn home_loop() -> Option<Arc<dyn HomeLoop>> {
    HOOK.read().unwrap_or_else(|e| e.into_inner()).clone()
}
