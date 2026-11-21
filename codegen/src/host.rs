//! Runs a program compiled in memory.

use std::sync::Arc;

use mote_runtime::exec::Image;
pub use mote_runtime::exec::{Fault, Feed, Ran};
use mote_runtime::loader::load;
use mote_runtime::{HeapStats, Runtime};

use crate::jit::Compiled;

impl Compiled {
    /// Runs start-up and `main`; a fault ends the run and is returned.
    pub fn run(self) -> Ran {
        self.run_with(|_| {})
    }

    /// Like `run`, with the runtime handed to `started` before the program's task starts.
    pub fn run_with(self, started: impl FnOnce(&Arc<Runtime>)) -> Ran {
        #[cfg(target_os = "linux")]
        if std::env::var("MOTE_PERF_MAP").is_ok_and(|value| value == "1") {
            let _ = std::fs::write(format!("/tmp/perf-{}.map", std::process::id()), self.perf_map());
        }
        let Compiled { jit, faults, types, meta, run, link: table, native_slots, files, failure, code, heap, scheduler, feeds, args, sink, single_threaded, .. } = self;
        if let Some(slots) = native_slots {
            let (address, _) = jit.get_finalized_data(slots);
            // SAFETY: the table has a word for each function of the link table, and nothing runs yet.
            if let Err(error) = unsafe { load(&table, &files, address as *mut usize) } {
                // SAFETY: nothing ran.
                unsafe { jit.free_memory() };
                return Ran { output: String::new(), fault: None, link: Some(error), err: None, exit: 0, stats: HeapStats::default() };
            }
        }
        // SAFETY: `run` is the address of the function `compile` made, which takes the context and returns nothing.
        let run: extern "C" fn(*mut mote_abi::TaskContext) = unsafe { std::mem::transmute(run) };
        let failure = failure.map(|cell| jit.get_finalized_data(cell).0 as usize);
        let ran = Image { run, types: types.table, faults, meta, code, failure, heap, scheduler, feeds, args, sink, single_threaded }.run_with(started);
        // SAFETY: the program has stopped for good, so nothing runs the code and no stack frame returns into it.
        unsafe { jit.free_memory() };
        ran
    }
}
