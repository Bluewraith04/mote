//! The register VM: interpreter, call frames, regions, scheduler and the offload pool.
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use isa::encoding::Instruction;
use isa::opcode::Opcode;
use isa::value::*;

/// Default per-task reduction budget: safepoint hits (back-edges and calls) before a task yields to the next ready one.
pub(crate) const DEFAULT_REDUCTION_BUDGET: u32 = 2000;

/// The most raw instructions between two [`Runtime::safepoint_poll`] calls, whatever the code does.
pub const SAFEPOINT_INSTRUCTION_INTERVAL: u32 = 1024;

/// Default bytes of register file one task may use.
pub(crate) const DEFAULT_MAX_STACK_BYTES: usize = 64 << 20;

pub(crate) const RECYCLED_REGISTER_SLOTS: usize = 4096;

pub(crate) const RECYCLED_FRAMES: usize = 1024;

pub(crate) const FREE_POOL_LIMIT: usize = 256;

/// Small shared helpers.
pub mod util;
pub mod machine;
pub mod roots;
pub mod handlers;
/// The bump allocator a runtime starts with.
pub mod alloc;
pub mod mutator;
pub mod arena;
pub mod intrinsic;
pub mod sched;
mod fault;
pub mod offload;
pub mod safepoint;
pub mod types;
pub(crate) mod sources;
pub(crate) mod locks;

pub use roots::RuntimeRoots;
pub use arena::ArenaStack;
pub use contracts::NativeCtx;
pub use intrinsic::{IntrinsicTypeTable, VmIntrinsicCtx};
pub use contracts::{NativeOutcome, PlatformContinuation, PlatformResult, SourceDecode};

#[derive(Debug, Clone, PartialEq, Eq)]
/// What a step of the interpreter ended with.
pub enum VmStatus {
    Running,
    /// The task spent its reduction budget; another ready task should run. A single-task run treats this as `Running`.
    Yielded,
    /// The task hit a `SCOPEEXIT` with children still running; `pc` stays on the instruction and the scheduler re-queues it when the last child finishes.
    Parked,
    Halted,
    /// A native asked to end the program with this exit code; the whole run stops.
    Exited(i32),
}

/// One structured-concurrency scope on a task's scope stack: its children still running, and the first fault one of them reported.
#[derive(Debug, Default)]
pub struct ScopeFrame {
    /// `task.call_stack.len()` when the scope opened; `SPAWN` attaches a child only at exactly this depth.
    pub owner_call_depth: usize,
    pub pending: usize,
    pub first_error: Option<String>,
    /// The faulted child's `Task<T>` handle, if it has one; `None` for a `spawn { }` statement, whose fault is always unobserved.
    pub first_error_handle: Option<NonNull<ObjectHeader>>,
}

pub use isa::code::CodeObject;

#[derive(Clone, Debug)]
/// One call frame.
pub struct Frame {
    pub return_pc: usize,
    pub base: usize,
    pub caller_code: usize,
    pub dest_reg: usize,
    /// The function object this frame was entered through for a `CALLV`, else `None`; `GETCAPTURE` reads its capture slots. A GC root.
    pub closure: Option<NonNull<ObjectHeader>>,
    /// Set when this frame is a resumed generator body: where `YIELD` and the final `RET` write back.
    pub generator: Option<GenLink>,
}

/// The `Generator` object a frame is running, and the absolute register that receives the `RESUME` status.
#[derive(Clone, Copy, Debug)]
pub struct GenLink {
    pub object: NonNull<ObjectHeader>,
    pub status_dest: usize,
}

/// The installed native-call dispatcher: called with the function index, a [`NativeCtx`] and the argument register window.
pub type NativeDispatchHook = std::sync::Arc<
    dyn Fn(u16, &mut dyn NativeCtx, &[Value]) -> NativeOutcome + Send + Sync,
>;

/// Resolves a native name to a registry id at load.
pub type NativeResolver = std::sync::Arc<dyn Fn(&str) -> Option<u16> + Send + Sync>;

/// The name the VM has always used for [`contracts::HeapStats`].
pub type GcStats = contracts::HeapStats;

/// The installed collector. `runtime` cannot depend on `gc`; the composition root injects one, and until then [`alloc::LeakingHeap`] is used.
pub type GcHook = Box<dyn contracts::Heap>;

/// State shared by every task: the code, the heap, the collector, module globals and native dispatch. One instance per program.
pub struct Runtime {
    pub code_objects: Vec<CodeObject>,
    pub type_descriptors: Vec<TypeDescriptor>,
    pub native_dispatcher: Option<NativeDispatchHook>,
    /// Registry ids for the program's native table; empty means operands are registry ids.
    pub native_ids: Vec<u16>,
    pub native_resolver: Option<NativeResolver>,
    /// The source texts the code objects' span tables index; empty in a release build.
    pub source_files: Vec<isa::code::SourceFile>,
    /// The heap every allocation goes through (per-thread mutators, see [`mutator`]).
    pub heap: GcHook,
    pub(crate) heap_id: u64,
    pub(crate) heap_installed: bool,
    pub(crate) schedule_seed: Option<u64>,
    pub(crate) budget_stream: std::sync::atomic::AtomicU64,
    /// Module-level globals addressed by `GETGLOBAL` / `SETGLOBAL`; sized from `CompiledProgram::global_count`, grown by `SETGLOBAL` if needed.
    pub globals: Mutex<Vec<Value>>,
    /// The shared descriptor every heap string points its `type_ptr` at.
    pub string_type: Box<TypeDescriptor>,
    /// Shared descriptors for the reserved intrinsic-collection ids.
    pub intrinsic_types: IntrinsicTypeTable,
    /// Type-term patterns, interned type ids and run-time instance descriptors.
    pub types: types::TypeInterner,

    /// The run queue, parked tasks, free pool and id counters behind one lock.
    pub sched: Arc<Mutex<sched::SchedState>>,
    pub(crate) wake: Arc<sched::Wake>,
    pub(crate) safepoint: Box<dyn contracts::SafepointCoordinator>,
    pub(crate) ready_len: Arc<std::sync::atomic::AtomicUsize>,
    exit_code: Mutex<Option<i32>>,
    exit_flag: std::sync::atomic::AtomicBool,
    /// The outside world natives reach through `NativeCtx::platform`.
    pub platform: Option<Arc<dyn contracts::Platform>>,
    pub(crate) gen_regions: arena::GeneratorRegions,
    pub(crate) offload: offload::OffloadPool,
    pub(crate) channel_locks: locks::LockStripes,
    pub(crate) sources: sources::Sources,
    pub(crate) cell_locks: locks::LockStripes,
    pub(crate) max_stack_slots: usize,
}

// SAFETY: every mutable field is behind a lock or atomic; the raw object pointers it holds are touched only by the worker running that task, or with every worker stopped at a safepoint.
unsafe impl Send for Runtime {}
unsafe impl Sync for Runtime {}

/// One task's execution state: call stack, register file, regions and program counter, plus scheduler bookkeeping.
pub struct TaskContext {
    pub call_stack: Vec<Frame>,
    pub registers: Vec<Value>,
    pub pc: usize,
    pub current_code: usize,
    /// Task-local regions; allocates nothing until the first region allocation.
    pub arenas: ArenaStack,
    /// While a generator body runs on `arenas`, the stacks it displaced, each with the call depth of that generator frame.
    pub arena_saves: Vec<(usize, Box<ArenaStack>)>,
    /// Segment bytes held by `arena_saves`; they still count toward the stack limit.
    pub saved_arena_bytes: usize,
    pub(crate) shared: Arc<sched::TaskShared>,
    /// Scheduler identity: `0` is the main task, [`Runtime::acquire_task`] assigns the rest.
    pub task_id: u64,
    /// Reduction budget: [`Runtime::safepoint_poll`] spends one unit per safepoint while other tasks wait, and at zero the next `step` yields.
    pub budget: u32,
    /// Instructions dispatched since the last [`Runtime::safepoint_poll`]; reset by every poll.
    pub instrs_since_safepoint: u32,
    /// Parked on a platform request; the re-executed `CALLNATIVE` collects its response.
    pub awaiting_platform: bool,
    /// Parked at a `SCOPEEXIT` for children: the scheduler re-checks the scope's `pending` as it parks the task, and a finishing child wakes only a task with this set.
    pub scope_parked: bool,
    /// This task's own `Task<T>` handle, if it was spawned; the scheduler writes completion into it and wakes whoever joined it.
    pub handle: Option<NonNull<ObjectHeader>>,
    /// `Shared` cells whose writer turn this task holds; released without a commit when it ends. A mark-only GC root.
    pub held_turns: Vec<NonNull<ObjectHeader>>,
    /// `Sender` handles this task owns; each is closed when the task ends. A mark-only GC root.
    pub owned_senders: Vec<NonNull<ObjectHeader>>,
    /// A `send` on a rendezvous channel waiting to be taken: `(channel id, the take count that completes it)`.
    pub handoff: Option<(u64, u64)>,
    /// Runs only on the home thread (`std.task.pin`).
    pub pinned: bool,
}

// SAFETY: a task is owned by one worker at a time; its object pointers are heap addresses, not thread-affine.
unsafe impl Send for TaskContext {}

impl Default for TaskContext {
    fn default() -> Self {
        TaskContext {
            call_stack: Vec::new(),
            registers: Vec::new(),
            pc: 0,
            current_code: 0,
            arenas: ArenaStack::new(),
            arena_saves: Vec::new(),
            saved_arena_bytes: 0,
            shared: Arc::new(sched::TaskShared::new(None, 0)),
            task_id: 0,
            budget: 0,
            instrs_since_safepoint: 0,
            awaiting_platform: false,
            scope_parked: false,
            handle: None,
            held_turns: Vec::new(),
            owned_senders: Vec::new(),
            handoff: None,
            pinned: false,
        }
    }
}

impl TaskContext {
    /// The entry task: one frame and a register file sized to the first code object, with a floor of 256 slots.
    pub fn entry(rt: &Runtime) -> Self {
        let initial_reg_count = rt
            .code_objects
            .first()
            .map(|c| c.register_count as usize)
            .unwrap_or(256);
        TaskContext {
            call_stack: vec![Frame {
                return_pc: 0,
                base: 0,
                caller_code: 0,
                dest_reg: 0,
                closure: None,
                generator: None,
            }],
            registers: vec![Value::null(); initial_reg_count.max(256)],
            pc: 0,
            current_code: 0,
            arenas: ArenaStack::new(),
            arena_saves: Vec::new(),
            saved_arena_bytes: 0,
            shared: Arc::new(sched::TaskShared::new(None, 0)),
            task_id: 0,
            budget: DEFAULT_REDUCTION_BUDGET,
            instrs_since_safepoint: 0,
            awaiting_platform: false,
            scope_parked: false,
            handle: None,
            held_turns: Vec::new(),
            owned_senders: Vec::new(),
            handoff: None,
            pinned: false,
        }
    }
}

impl Runtime {
    pub fn new(code_objects: Vec<CodeObject>) -> Self {
        Self::with_types(code_objects, vec![])
    }

    pub fn with_types(code_objects: Vec<CodeObject>, type_descriptors: Vec<TypeDescriptor>) -> Self {
        let state = sched::SchedState::new();
        let ready_len = state.run_queue.len_handle();
        let sched = Arc::new(Mutex::new(state));
        let wake = Arc::new(sched::Wake::default());
        Runtime {
            code_objects,
            type_descriptors,
            native_dispatcher: None,
            native_ids: Vec::new(),
            native_resolver: None,
            source_files: Vec::new(),
            platform: None,
            gen_regions: arena::GeneratorRegions::default(),
            offload: offload::OffloadPool::new(sched.clone(), wake.clone()),
            heap: Box::new(alloc::LeakingHeap::default()),
            heap_id: mutator::next_heap_id(),
            heap_installed: false,
            globals: Mutex::new(Vec::new()),
            string_type: Box::new(TypeDescriptor::string_type()),
            intrinsic_types: IntrinsicTypeTable::new(),
            types: types::TypeInterner::default(),
            sched,
            wake,
            safepoint: Box::new(safepoint::StopTheWorld::new()),
            ready_len,
            exit_code: Mutex::new(None),
            exit_flag: std::sync::atomic::AtomicBool::new(false),
            channel_locks: locks::LockStripes::new(),
            sources: Mutex::new(std::collections::HashMap::new()),
            cell_locks: locks::LockStripes::new(),
            schedule_seed: None,
            budget_stream: std::sync::atomic::AtomicU64::new(0),
            max_stack_slots: DEFAULT_MAX_STACK_BYTES / std::mem::size_of::<Value>(),
        }
    }

    /// Caps the register file of every task at `bytes`.
    pub fn set_max_stack(&mut self, bytes: usize) {
        self.max_stack_slots = bytes / std::mem::size_of::<Value>();
    }

    pub(crate) fn check_stack(&self, needed: usize, region_bytes: usize, depth: usize) -> Result<(), String> {
        if needed + region_bytes / std::mem::size_of::<Value>() > self.max_stack_slots {
            return Err(format!("stack overflow: {depth} calls deep"));
        }
        Ok(())
    }

    /// Varies where tasks switch, reproducibly: each time slice gets a seeded random budget.
    pub fn set_schedule_seed(&mut self, seed: u64) {
        self.schedule_seed = Some(seed);
    }

    pub(crate) fn next_budget(&self) -> u32 {
        let Some(seed) = self.schedule_seed else { return DEFAULT_REDUCTION_BUDGET };
        let n = self.budget_stream.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut z = seed.wrapping_add(n.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        1 + (z % u64::from(DEFAULT_REDUCTION_BUDGET)) as u32
    }

    pub fn with_type_registry(
        code_objects: Vec<CodeObject>,
        registry: &isa::value::TypeRegistry,
    ) -> Self {
        Self::with_types(code_objects, registry.types().to_vec())
    }

    /// Pre-sizes the module-globals array; `SETGLOBAL` grows it anyway.
    pub fn set_global_count(&mut self, count: usize) {
        *self.globals.lock().unwrap() = vec![Value::null(); count];
    }

    /// Installs a heap; must happen before any allocation, and the previous heap's objects are not migrated.
    pub fn set_heap(&mut self, heap: GcHook) {
        self.heap = heap;
        self.heap_id = mutator::next_heap_id();
        self.heap_installed = true;
    }

    pub(crate) fn alloc_heap_string(&self, s: &str) -> Value {
        self.with_mutator(|m| intrinsic::alloc_string_object(m, self.string_type.as_ref(), s.as_bytes()))
    }

    pub fn decode(i: Instruction) -> (u8, u32) {
        let opcode_bits: u8 = (i & 0xFF) as u8;
        let operand_bits = i >> 8;
        (opcode_bits, operand_bits)
    }

    pub fn fetch(&self, task: &TaskContext) -> Instruction {
        self.code_objects[task.current_code].instructions[task.pc]
    }

    pub(crate) fn request_exit(&self, code: i32) {
        self.exit_code.lock().unwrap().get_or_insert(code);
        self.exit_flag.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub(crate) fn exit_requested(&self) -> bool {
        self.exit_flag.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The code a native asked the program to exit with, if any.
    pub(crate) fn exit_code(&self) -> Option<i32> {
        *self.exit_code.lock().unwrap()
    }

    pub fn safepoint_poll(&self, task: &mut TaskContext) -> Result<(), String> {
        task.instrs_since_safepoint = 0;
        self.gc_safepoint(task)?;
        if self.ready_len.load(std::sync::atomic::Ordering::Relaxed) != 0 {
            task.budget = task.budget.saturating_sub(1);
        }
        if self.exit_requested() {
            return Err("program exited".to_string());
        }
        if !task.awaiting_platform && task.shared.cancelled.load(std::sync::atomic::Ordering::SeqCst) {
            self.cancel_children(task.task_id, None);
            return Err("task was cancelled".to_string());
        }
        Ok(())
    }

    /// The installed collector's counters, or `None` if no collector is installed.
    pub fn gc_stats(&self) -> Option<GcStats> {
        self.heap_installed.then(|| self.heap.stats())
    }

    /// Objects allocated by block size (see [`contracts::Heap::size_histogram`]); empty without a collector.
    pub fn size_histogram(&self) -> Vec<u64> {
        self.heap.size_histogram()
    }

    pub fn step(&self, task: &mut TaskContext) -> Result<VmStatus, String> {
        if task.call_stack.is_empty() {
            return Ok(VmStatus::Halted);
        }
        if task.budget == 0 {
            task.budget = self.next_budget();
            return Ok(VmStatus::Yielded);
        }
        if task.current_code >= self.code_objects.len() {
            return Err("Invalid current_code index".to_string());
        }
        if task.pc >= self.code_objects[task.current_code].instructions.len() {
            if task.call_stack.len() <= 1 {
                task.call_stack.pop();
                return Ok(VmStatus::Halted);
            }
            let finished_frame = task.call_stack.pop().unwrap();
            task.registers[finished_frame.dest_reg] = Value::null();
            task.current_code = finished_frame.caller_code;
            task.pc = finished_frame.return_pc;
            return Ok(VmStatus::Running);
        }
        task.instrs_since_safepoint += 1;
        if task.instrs_since_safepoint >= SAFEPOINT_INSTRUCTION_INTERVAL {
            self.safepoint_poll(task)?;
        }
        let inst = self.fetch(task);
        let (opcode_byte, operands) = Self::decode(inst);
        let opcode = Opcode::try_from(opcode_byte).map_err(|b| format!("Unknown opcode: {}", b))?;
        handlers::execute(self, task, opcode, operands)
    }
}

/// Environment variable [`Runtime::run_entry`] reads to run the test suite on N workers.
pub(crate) const TEST_WORKERS_ENV: &str = "MOTE_TEST_WORKERS";

impl Runtime {
    /// Runs a fresh entry task and every task it spawns on [`TEST_WORKERS_ENV`] workers (default 1); returns the finished main task.
    pub fn run_entry(&mut self) -> Result<TaskContext, String> {
        let main = TaskContext::entry(self);
        self.run_task(main)
    }

    /// [`run_entry`](Self::run_entry) on exactly `workers` workers.
    pub fn run_entry_on(&mut self, workers: usize) -> Result<TaskContext, String> {
        let main = TaskContext::entry(self);
        self.run_main_parallel(main, workers.max(1))
    }

    /// [`run_entry`](Self::run_entry) for a prepared main task.
    pub(crate) fn run_task(&mut self, main: TaskContext) -> Result<TaskContext, String> {
        let workers = std::env::var(TEST_WORKERS_ENV)
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|&n| n >= 1)
            .unwrap_or(1);
        self.run_main_parallel(main, workers)
    }

    /// `Exited(code)` once a native asked the program to exit, else `Halted`.
    pub fn status(&self) -> VmStatus {
        match self.exit_code() {
            Some(code) => VmStatus::Exited(code),
            None => VmStatus::Halted,
        }
    }

    pub fn set_native_dispatcher(&mut self, dispatcher: NativeDispatchHook) {
        self.native_dispatcher = Some(dispatcher);
    }

    /// Installs the source texts fault locations print from.
    pub fn set_sources(&mut self, sources: Vec<isa::code::SourceFile>) {
        self.source_files = sources;
    }

    pub fn set_native_resolver(&mut self, resolver: NativeResolver) {
        self.native_resolver = Some(resolver);
    }

    /// Resolve the program's native table by name; fails once, naming every missing native.
    pub fn set_native_table(&mut self, names: &[String]) -> Result<(), String> {
        if names.is_empty() {
            return Ok(());
        }
        let resolver = self.native_resolver.as_ref();
        let mut ids = Vec::with_capacity(names.len());
        let mut missing = Vec::new();
        for name in names {
            match resolver.and_then(|r| r(name)) {
                Some(id) => ids.push(id),
                None => missing.push(name.as_str()),
            }
        }
        if !missing.is_empty() {
            return Err(format!("unresolved natives: {}", missing.join(", ")));
        }
        self.native_ids = ids;
        Ok(())
    }

    pub fn set_platform(&mut self, platform: Arc<dyn contracts::Platform>) {
        self.platform = Some(platform);
    }
}

impl std::fmt::Debug for TaskContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskContext").field("task_id", &self.task_id).field("pc", &self.pc).finish_non_exhaustive()
    }
}

impl TaskContext {
    /// Bytes of region segments this task holds, including the stacks a running generator displaced.
    pub(crate) fn region_bytes(&self) -> usize {
        self.arenas.segment_bytes() + self.saved_arena_bytes
    }

    /// One past the last register any active call window can use.
    pub(crate) fn live_register_extent(&self, code_objects: &[CodeObject]) -> usize {
        let Some(top) = self.call_stack.last() else {
            return 0;
        };
        let count = code_objects
            .get(self.current_code)
            .map(|c| c.register_count as usize)
            .unwrap_or(256);
        (top.base + count).min(self.registers.len())
    }

    /// Nulls every register above [`live_register_extent`](Self::live_register_extent), which may hold dangling pointers from returned callees.
    pub(crate) fn clear_dead_registers(&mut self, code_objects: &[CodeObject]) {
        if self.call_stack.is_empty() {
            return;
        }
        let extent = self.live_register_extent(code_objects);
        for dead in &mut self.registers[extent..] {
            *dead = Value::null();
        }
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.close_all_sources();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use isa::encoding::*;

    #[test]
    fn test_arithmetic_and_pc_advancement() {
        let code = CodeObject::new(
            vec![
                encode_ri(Opcode::LOADI, 0, 10),
                encode_ri(Opcode::LOADI, 1, 25),
                encode_r3(Opcode::ADD, 2, 0, 1),
                encode_none(Opcode::HALT),
            ],
            vec![],
            8,
            0,
        );
        let mut rt = Runtime::new(vec![code]);
        let task = rt.run_entry().unwrap();
        let status = rt.status();
        assert_eq!(status, VmStatus::Halted);
        assert_eq!(task.registers[2].as_int(), Some(35));
        assert_eq!(task.pc, 3);
    }

    #[test]
    fn test_unconditional_jump() {
        let code = CodeObject::new(
            vec![
                encode_ri(Opcode::LOADI, 0, 5),
                encode_ju(Opcode::JMP, 2),
                encode_ri(Opcode::LOADI, 0, 99),
                encode_none(Opcode::HALT),
            ],
            vec![],
            8,
            0,
        );
        let mut rt = Runtime::new(vec![code]);
        let task = rt.run_entry().unwrap();
        let status = rt.status();
        assert_eq!(status, VmStatus::Halted);
        assert_eq!(task.registers[0].as_int(), Some(5));
        assert_eq!(task.pc, 3);
    }

    #[test]
    fn test_conditional_jump_loop() {
        let code = CodeObject::new(
            vec![
                encode_ri(Opcode::LOADI, 0, 5),
                encode_ri(Opcode::LOADI, 1, 0),
                encode_ri(Opcode::LOADI, 2, 1),
                encode_r3(Opcode::EQ, 3, 0, 4),
                encode_jc(Opcode::JMPIF, 3, 4),
                encode_r3(Opcode::ADD, 1, 1, 0),
                encode_r3(Opcode::SUB, 0, 0, 2),
                encode_ju(Opcode::JMP, -4),
                encode_none(Opcode::HALT),
            ],
            vec![],
            8,
            0,
        );
        let mut rt = Runtime::new(vec![code]);
        let mut task = TaskContext::entry(&rt);
        task.registers[4] = Value::small_int(0);
        let task = rt.run_task(task).unwrap();
        let status = rt.status();
        assert_eq!(status, VmStatus::Halted);
        assert_eq!(task.registers[1].as_int(), Some(15));
        assert_eq!(task.registers[0].as_int(), Some(0));
    }

    #[test]
    fn test_function_call_non_recursive() {
        let main_code = CodeObject::new(
            vec![
                encode_ri(Opcode::LOADI, 2, 7),
                encode_call(1, 1),
                encode_none(Opcode::HALT),
            ],
            vec![],
            8,
            0,
        );
        let square_code = CodeObject::new(
            vec![
                encode_r3(Opcode::MUL, 1, 0, 0),
                encode_ret(1),
            ],
            vec![],
            4,
            1,
        );

        let mut rt = Runtime::new(vec![main_code, square_code]);
        let task = rt.run_entry().unwrap();
        let status = rt.status();
        assert_eq!(status, VmStatus::Halted);
        assert_eq!(task.registers[1].as_int(), Some(49));
    }

    #[test]
    fn test_function_call_recursive_factorial() {
        let main_code = CodeObject::new(
            vec![
                encode_ri(Opcode::LOADI, 2, 5),
                encode_call(1, 1),
                encode_none(Opcode::HALT),
            ],
            vec![],
            8,
            0,
        );
        let fact_code = CodeObject::new(
            vec![
                encode_ri(Opcode::LOADI, 1, 1),
                encode_r3(Opcode::LE, 2, 0, 1),
                encode_jc(Opcode::JMPIFNOT, 2, 2),
                encode_ret(1),
                encode_r3(Opcode::SUB, 5, 0, 1),
                encode_call(4, 1),
                encode_r3(Opcode::MUL, 5, 0, 4),
                encode_ret(5),
            ],
            vec![],
            8,
            1,
        );

        let mut rt = Runtime::new(vec![main_code, fact_code]);
        let task = rt.run_entry().unwrap();
        let status = rt.status();
        assert_eq!(status, VmStatus::Halted);
        assert_eq!(task.registers[1].as_int(), Some(120));
    }

    #[test]
    fn test_callv_indirect_call_through_a_function_value() {
        let main_code = CodeObject::new(
            vec![
                encode_newobj(0, 0),
                encode_ri(Opcode::LOADI, 1, 1),
                encode_setfield(0, 0, 1),
                encode_ri(Opcode::LOADI, 2, 20),
                encode_callv(3, 0, 2),
                encode_none(Opcode::HALT),
            ],
            vec![],
            8,
            0,
        );
        let dbl_code = CodeObject::new(
            vec![encode_r3(Opcode::ADD, 1, 0, 0), encode_ret(1)],
            vec![],
            4,
            1,
        );
        let fn_type = TypeDescriptor::function_type(0);
        let mut rt = Runtime::with_types(vec![main_code, dbl_code], vec![fn_type]);
        let task = rt.run_entry().unwrap();
        assert_eq!(rt.status(), VmStatus::Halted);
        assert_eq!(task.registers[3].as_int(), Some(40));
    }

    #[test]
    fn test_getcapture_reads_the_closure_object() {
        let main_code = CodeObject::new(
            vec![
                encode_newobj(0, 0),
                encode_ri(Opcode::LOADI, 1, 1),
                encode_setfield(0, 0, 1),
                encode_ri(Opcode::LOADI, 2, 7),
                encode_setfield(0, 1, 2),
                encode_ri(Opcode::LOADI, 3, 5),
                encode_callv(4, 0, 3),
                encode_none(Opcode::HALT),
            ],
            vec![],
            8,
            0,
        );
        let closure_code = CodeObject::new(
            vec![
                encode_getcapture(1, 0),
                encode_r3(Opcode::ADD, 2, 0, 1),
                encode_ret(2),
            ],
            vec![],
            4,
            1,
        );
        let fn_type = TypeDescriptor::function_type(1);
        let mut rt = Runtime::with_types(vec![main_code, closure_code], vec![fn_type]);
        let task = rt.run_entry().unwrap();
        assert_eq!(rt.status(), VmStatus::Halted);
        assert_eq!(task.registers[4].as_int(), Some(12));
    }

    #[test]
    fn test_getglobal_setglobal_roundtrip_and_autogrow() {
        let code = CodeObject::new(
            vec![
                encode_ri(Opcode::LOADI, 0, 77),
                encode_setglobal(0, 3),
                encode_getglobal(1, 3),
                encode_getglobal(2, 0),
                encode_none(Opcode::HALT),
            ],
            vec![],
            8,
            0,
        );
        let mut rt = Runtime::new(vec![code]);
        assert_eq!(rt.globals.lock().unwrap().len(), 0);
        let task = rt.run_entry().unwrap();
        assert_eq!(rt.status(), VmStatus::Halted);
        assert_eq!(rt.globals.lock().unwrap().len(), 4);
        assert_eq!(task.registers[1].as_int(), Some(77));
        assert!(task.registers[2].is_null());
    }

    #[test]
    fn test_getcapture_in_a_non_closure_frame_errors() {
        let main_code = CodeObject::new(
            vec![encode_call(0, 1), encode_none(Opcode::HALT)],
            vec![],
            4,
            0,
        );
        let fn_code = CodeObject::new(
            vec![encode_getcapture(0, 0), encode_ret(0)],
            vec![],
            4,
            0,
        );
        let mut rt = Runtime::new(vec![main_code, fn_code]);
        let err = rt.run_entry().unwrap_err();
        assert!(err.contains("not entered through a closure"), "got: {err}");
    }

    #[test]
    fn test_callv_on_a_non_function_value_errors() {
        let code = CodeObject::new(
            vec![
                encode_ri(Opcode::LOADI, 0, 5),
                encode_callv(1, 0, 0),
                encode_none(Opcode::HALT),
            ],
            vec![],
            8,
            0,
        );
        let mut rt = Runtime::new(vec![code]);
        let err = rt.run_entry().unwrap_err();
        assert!(err.contains("not a callable value"), "got: {err}");
    }

    #[test]
    fn test_object_allocation_and_field_access() {
        let point_type = TypeDescriptor::with_field_count(100, 2);

        let main_code = CodeObject::new(
            vec![
                encode_newobj(0, 0),
                encode_ri(Opcode::LOADI, 1, 10),
                encode_ri(Opcode::LOADI, 2, 20),
                encode_setfield(0, 0, 1),
                encode_setfield(0, 1, 2),
                encode_getfield(3, 0, 0),
                encode_getfield(4, 0, 1),
                encode_r3(Opcode::ADD, 5, 3, 4),
                encode_typeof(6, 0),
                encode_none(Opcode::HALT),
            ],
            vec![],
            8,
            0,
        );

        let mut rt = Runtime::with_types(vec![main_code], vec![point_type]);
        let task = rt.run_entry().unwrap();
        let status = rt.status();
        assert_eq!(status, VmStatus::Halted);
        assert_eq!(task.registers[5].as_int(), Some(30));
        assert_eq!(task.registers[6].as_int(), Some(100));
    }
}
