//! Runs a compiled program as the main task of a scheduler and brings a fault back as a value.

use std::io::Write;
use std::sync::Arc;

use mote_abi::{DebugTable, FaultKind, FaultTable, Meta, TaskContext, TypeTable, ELEMENTS_OFFSET, LENGTH_OFFSET};
use mote_scheduler::Scheduler;

use crate::loader::LinkError;
use crate::{CodeMap, FaultReport, HeapConfig, HeapStats, Located, Runtime};

/// The fault that stopped a program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fault {
    pub kind: FaultKind,
    pub message: String,
    /// The two ends of the span of the check, when it has one.
    pub span: Option<(u32, u32)>,
    /// The site of the check in the fault table, when it has one.
    pub site: Option<u32>,
    /// The functions of the stack that the code map names, innermost first, with the position of the call in each.
    pub frames: Vec<Located>,
}

impl Fault {
    fn of(report: &FaultReport, runtime: &Runtime) -> Self {
        let frames = runtime.code_map().map_or_else(Vec::new, |code| report.backtrace.iter().map_while(|&pc| code.locate(pc)).collect());
        Fault { kind: report.kind, message: report.message.clone(), span: report.span, site: report.site, frames }
    }

    /// The fault as an executable prints it: the message, where the check is, then each function of the stack with the line it was in.
    pub fn text(&self, debug: &DebugTable) -> String {
        let mut text = format!("fault: {}\n", self.message);
        if let Some((file, line, column)) = self.site.and_then(|site| debug.fault_place(site)) {
            text.push_str(&format!("  at {file}:{line}:{column}\n"));
        }
        for frame in &self.frames {
            match debug.place(frame.position) {
                Some((file, line, _)) => text.push_str(&format!("  in {} ({file}:{line})\n", frame.function)),
                None => text.push_str(&format!("  in {}\n", frame.function)),
            }
        }
        text
    }
}

/// What a run printed and how it ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ran {
    pub output: String,
    pub fault: Option<Fault>,
    /// Why the natives were not loaded, when nothing ran.
    pub link: Option<LinkError>,
    /// The text of the error, when the entry returned an `Err`.
    pub err: Option<String>,
    /// The exit code: what `exit` asked for, else the integer `main` returned, else 0.
    pub exit: i32,
    /// What the heap did during the run.
    pub stats: HeapStats,
}

/// A message a test host pushes to the port the program opened `port`th, `at_ms` after the start; none closes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Feed {
    pub port: usize,
    pub at_ms: u64,
    pub bytes: Option<Vec<u8>>,
}

/// Nanoseconds in a millisecond of the scheduler's clock.
const NANOS: u64 = 1_000_000;

/// A task of the scheduler that pushes each feed at its time on the scheduler's clock; a port not opened by then misses the feed.
fn scripted_feeder(runtime: Arc<Runtime>, mut feeds: Vec<Feed>) {
    feeds.sort_by_key(|feed| feed.at_ms);
    for feed in feeds {
        let at = feed.at_ms.saturating_mul(NANOS);
        let now = mote_scheduler::now();
        if at > now {
            mote_scheduler::sleep(at - now);
        }
        match &feed.bytes {
            Some(bytes) => drop(runtime.feed_port(feed.port, bytes)),
            None => drop(runtime.close_port(feed.port)),
        }
    }
}

/// What the program's task and the host share.
struct Link {
    run: extern "C" fn(*mut TaskContext),
    runtime: Arc<Runtime>,
    fault: Option<Fault>,
    /// The feeds the host gives, delivered from a task of the scheduler.
    scripted: Vec<Feed>,
}

/// The program's task: runs start-up and `main`, and ends the run with a fault when a detached task faulted and nobody awaited it.
fn program(address: usize) {
    // SAFETY: `address` is the link, which the host keeps alive while the scheduler runs.
    let link = unsafe { &mut *(address as *mut Link) };
    let runtime = &link.runtime;
    let context = mote_scheduler::current().context();
    // SAFETY: the context is this task's own block.
    let context = unsafe { &mut *context };
    runtime.attach(context);
    runtime.join_task(context, mote_scheduler::current());
    if !link.scripted.is_empty() {
        let (runtime, feeds) = (runtime.clone(), std::mem::take(&mut link.scripted));
        mote_scheduler::spawn(move || scripted_feeder(runtime, feeds));
    }
    (link.run)(context);
    if let Some(report) = runtime.unawaited_fault() {
        link.fault = Some(Fault::of(&report, runtime));
    }
    runtime.leave(context);
}

/// A program ready to run, and how.
pub struct Image {
    /// The function that runs start-up and the entry, given the task's context.
    pub run: extern "C" fn(*mut TaskContext),
    pub types: TypeTable,
    pub faults: FaultTable,
    pub meta: Meta,
    pub code: CodeMap,
    /// The address of the three words the run fills: 1 and the text of the error when the entry returns an `Err`, and the integer an entry returns.
    pub failure: Option<usize>,
    pub heap: HeapConfig,
    pub scheduler: mote_scheduler::Config,
    /// What a test host pushes to the ports the program opens.
    pub feeds: Vec<Feed>,
    /// The arguments `std.io.args` gives.
    pub args: Vec<String>,
    /// Where the program's output goes as it is printed; none keeps it for `Ran::output`.
    pub sink: Option<Box<dyn Write + Send>>,
}

impl Image {
    /// Runs start-up and the entry; a fault ends the run and is returned.
    pub fn run(self) -> Ran {
        self.run_with(|_| {})
    }

    /// Like `run`, with the runtime handed to `started` before the program's task starts.
    pub fn run_with(self, started: impl FnOnce(&Arc<Runtime>)) -> Ran {
        let Image { run, types, faults, meta, code, failure, heap, mut scheduler, feeds, args, sink } = self;
        if !feeds.is_empty() && scheduler.seed.is_none() {
            scheduler = mote_scheduler::Config::seeded(0);
        }
        let runtime = Runtime::new(Arc::new(types), faults, heap);
        runtime.set_data(meta);
        runtime.set_code_map(code);
        runtime.set_program_args(args);
        if let Some(sink) = sink {
            runtime.set_sink(sink);
        }
        started(&runtime);
        runtime.set_scripted(!feeds.is_empty());
        let mut link = Box::new(Link { run, runtime: runtime.clone(), fault: None, scripted: feeds });
        let address = &mut *link as *mut Link as usize;
        runtime.set_fault_hook(Arc::new(move |report| {
            // SAFETY: the link outlives the run, and the hook runs in the program's task while the host waits.
            let link = unsafe { &mut *(address as *mut Link) };
            link.fault = Some(Fault::of(report, &link.runtime));
            // SAFETY: the context is this task's own block.
            link.runtime.leave(unsafe { &mut *mote_scheduler::current().context() });
            mote_scheduler::exit()
        }));
        let outcome = Scheduler::new(scheduler).expect("a scheduler").run_or_exit(move || program(address));
        runtime.leave_all();
        if let Err(deadlock) = outcome {
            link.fault = Some(Fault { kind: FaultKind::Deadlock, message: deadlock.to_string(), span: None, site: None, frames: Vec::new() });
        }
        runtime.flush_sink();
        let stats = runtime.heap().stats();
        let output = String::from_utf8_lossy(&runtime.take_output()).into_owned();
        let fault = link.fault.take();
        let err = failure.filter(|_| fault.is_none()).and_then(|cell| {
            // SAFETY: the cell has two words; the text is a string object in the runtime's heap, which is alive until it drops.
            unsafe {
                let words = cell as *const usize;
                if *words == 0 {
                    return None;
                }
                let text = *words.add(1) as *const u8;
                if text.is_null() {
                    return Some(String::new());
                }
                let length = *(text.add(LENGTH_OFFSET as usize) as *const usize);
                let bytes = std::slice::from_raw_parts(text.add(ELEMENTS_OFFSET as usize), length);
                Some(String::from_utf8_lossy(bytes).into_owned())
            }
        });
        let mut exit = runtime.exit_code();
        if exit == 0 && fault.is_none() {
            // SAFETY: the cell has three words, the third of which is the integer the entry returned.
            exit = failure.map_or(0, |cell| unsafe { *(cell as *const i64).add(2) } as i32);
        }
        Ran { output, fault, link: None, err, exit, stats }
    }
}
