//! Compiles a module to machine code in memory.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::Write;

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, AbiParam, InstBuilder, MemFlags, Signature, StackSlotData, StackSlotKind, UserFuncName};
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{default_libcall_names, DataDescription, DataId, FuncId as ClifFunc, Linkage, Module as ClifModule};
use mote_abi::{closure_signature, passing, signature, Entry, FaultTable, FrameDescriptor, Header, LinkTable, Meta, Passing, Slot, CLOSURE_CODE_OFFSET, FLAG_STATIC};
use mote_runtime::{CodeMap, FunctionInfo, GlobalRoot, HeapConfig};
use mote_base::{Interner, Symbol};
use mote_ir::{string_constants, Callee, EntryKind, FuncId, Function, FunctionKind, Module, Op, Operand};
use mote_semantics::{DefId, DefKind, LangItem, Program, TyId, TyKind};

use mote_runtime::dynamic::{self, mote_dyn};
use crate::debug::{self, Debug};
use crate::frames;
use crate::function::{abi_params, const_ty, Env, Imports, Lowering};
use crate::host::Feed;
use mote_runtime::helpers::{self, mote_fmod, mote_print, mote_math, mote_show, mote_string_compare, mote_string_cut, mote_system, mote_text, mote_value};
use mote_runtime::loader::{self, mote_native};
use crate::meta;
use crate::native;
use crate::tasks;
use crate::typetable::{self, TypeMap};
use crate::values::{emit_store, leaves_of, Addr};

/// Why a module was not compiled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileError(String);

impl CompileError {
    pub(crate) fn internal(what: impl fmt::Display) -> Self {
        CompileError(format!("internal: {what}"))
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, out: &mut fmt::Formatter) -> fmt::Result {
        out.write_str(&self.0)
    }
}

impl std::error::Error for CompileError {}

/// A program in memory: its code, its tables and the address that runs start-up and `main`.
pub struct Compiled {
    pub(crate) jit: JITModule,
    pub(crate) faults: FaultTable,
    pub(crate) types: TypeMap,
    pub(crate) meta: Meta,
    pub(crate) run: usize,
    /// The libraries and functions the loader opens before `main`.
    pub(crate) link: LinkTable,
    /// The words the loader fills with the address of each linked function.
    pub(crate) native_slots: Option<DataId>,
    /// The file the loader opens for each library of `link`.
    pub(crate) files: Vec<String>,
    /// Two words the run fills when the entry returns an `Err`: 1, then the text of the error.
    pub(crate) failure: Option<DataId>,
    /// The frame descriptors of the call sites and the static data that holds references.
    pub(crate) code: CodeMap,
    /// The start, the size and the name of each function that has call sites.
    pub(crate) functions: Vec<(usize, usize, String)>,
    /// The settings of the heap the program runs in.
    pub(crate) heap: HeapConfig,
    /// The settings of the scheduler the program runs on.
    pub(crate) scheduler: mote_scheduler::Config,
    /// What a test host pushes to the ports the program opens.
    pub(crate) feeds: Vec<Feed>,
    /// The arguments `std.io.args` gives.
    pub(crate) args: Vec<String>,
    /// Where the program's output goes as it is printed; none keeps it for `Ran::output`.
    pub(crate) sink: Option<Box<dyn Write + Send>>,
}

impl Compiled {
    /// The functions of the program as `perf` reads them from `/tmp/perf-<pid>.map`: a line of the start, the size and the name.
    pub fn perf_map(&self) -> String {
        self.functions.iter().map(|(start, size, name)| format!("{start:x} {size:x} {name}\n")).collect()
    }

    /// Sends the output of the program to the writer as it is printed; `Ran::output` is then empty.
    pub fn with_sink(mut self, sink: Box<dyn Write + Send>) -> Self {
        self.sink = Some(sink);
        self
    }

    /// Sets what is pushed to the ports, each at its time on the scheduler's clock; a run with feeds and no seed is seeded with 0.
    pub fn with_feeds(mut self, feeds: Vec<Feed>) -> Self {
        self.feeds = feeds;
        self
    }

    /// Sets the arguments the program reads with `std.io.args`.
    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    /// Sets the heap the program runs in.
    pub fn with_heap(mut self, heap: HeapConfig) -> Self {
        self.heap = heap;
        self
    }

    /// Sets the scheduler the program runs on, such as a seeded one for a repeatable order of tasks.
    pub fn with_scheduler(mut self, scheduler: mote_scheduler::Config) -> Self {
        self.scheduler = scheduler;
        self
    }

    /// Names the file to open for each library of the program, in the order of its link table.
    pub fn with_libraries(mut self, files: Vec<String>) -> Self {
        self.files = files;
        self
    }
}

/// The host's target; position-independent code for an object file, which a linker places.
pub(crate) fn isa(pic: bool) -> Result<cranelift_codegen::isa::OwnedTargetIsa, CompileError> {
    let mut flags = settings::builder();
    for (name, value) in [("opt_level", "speed"), ("preserve_frame_pointers", "true"), ("is_pic", if pic { "true" } else { "false" }), ("use_colocated_libcalls", "false"), ("enable_probestack", "false")] {
        flags.set(name, value).map_err(CompileError::internal)?;
    }
    let native = cranelift_native::builder().map_err(CompileError::internal)?;
    native.finish(settings::Flags::new(flags)).map_err(CompileError::internal)
}

/// What the roots reach: the functions in the order they are found, the globals they use, and the functions made into values.
struct Reach {
    order: Vec<FuncId>,
    globals: Vec<DefId>,
    valued: HashSet<FuncId>,
}

/// The functions reachable from the roots by direct calls and closures.
fn reachable(module: &Module, roots: &[FuncId]) -> Reach {
    let mut reach = Reach { order: Vec::new(), globals: Vec::new(), valued: HashSet::new() };
    let mut seen = HashSet::new();
    let mut work: Vec<FuncId> = roots.iter().rev().copied().collect();
    while let Some(id) = work.pop() {
        if !seen.insert(id) {
            continue;
        }
        reach.order.push(id);
        for (_, block) in module.functions[id].blocks.iter() {
            for inst in &block.insts {
                match &inst.op {
                    Op::Call { callee: Callee::Function(target), .. } => work.push(*target),
                    Op::Defer { func, .. } => work.push(*func),
                    Op::Closure { func, .. } | Op::Spawn { func, .. } | Op::Stream { resume: func, .. } => {
                        reach.valued.insert(*func);
                        work.push(*func);
                    }
                    Op::LoadGlobal(def) | Op::StoreGlobal { def, .. } if !reach.globals.contains(def) => reach.globals.push(*def),
                    _ => {}
                }
            }
        }
    }
    reach
}

/// The bytes of a string constant as an object outside the heap: header, length, text, zero padding to a whole word.
fn string_object(string: mote_abi::TypeId, text: &str) -> Vec<u8> {
    let mut contents = Header::new(string, FLAG_STATIC).bits().to_le_bytes().to_vec();
    contents.extend((text.len() as u64).to_le_bytes());
    contents.extend(text.as_bytes());
    contents.resize(contents.len().div_ceil(8) * 8, 0);
    contents
}

/// The Cranelift signature of a function under the `tail` convention, with the task context first and the closure after the result slot when it is called through one.
fn function_signature(function: &Function, closure_conv: bool, types: &TypeMap) -> Result<Signature, CompileError> {
    let args: Vec<_> = function.params.iter().map(|&param| types.id(function.regs[param].ty)).collect();
    let abi = if closure_conv { closure_signature(&types.table, &args, types.id(function.ret)) } else { signature(&types.table, &args, types.id(function.ret)) };
    let mut params = abi_params(&abi.params);
    if !function.out_params.is_empty() {
        params.push(AbiParam::new(types::I64));
    }
    Ok(Signature { params, returns: abi_params(&abi.results), call_conv: CallConv::Tail })
}

/// The signature of a runtime entry or helper under the host's convention.
pub(crate) fn c_signature(params: &[mote_abi::Scalar], results: &[mote_abi::Scalar], call_conv: CallConv) -> Signature {
    Signature { params: abi_params(params), returns: abi_params(results), call_conv }
}

/// Compiles the functions start-up and the entry reach.
pub fn compile(module: &Module, program: &mut Program, interner: &Interner, entry: EntryKind) -> Result<Compiled, CompileError> {
    compile_with(module, program, interner, entry, None)
}

/// The position of the variant of `Result` with the name.
fn result_variant(program: &Program, interner: &Interner, name: &str) -> Option<u32> {
    let def = *program.lang.get(&LangItem::Result)?;
    match &program.defs[def].kind {
        DefKind::Enum(item) => item.variants.iter().position(|variant| interner.resolve(variant.name) == name).map(|at| at as u32),
        _ => None,
    }
}

/// `compile`, where `error_text` is the function that gives the text of the error of an entry that returns a `Result` whose error is not an `Error`.
pub fn compile_with(module: &Module, program: &mut Program, interner: &Interner, entry: EntryKind, error_text: Option<FuncId>) -> Result<Compiled, CompileError> {
    build(module, program, interner, Some(entry), error_text)
}

/// Compiles start-up alone.
pub fn compile_start(module: &Module, program: &mut Program, interner: &Interner) -> Result<Compiled, CompileError> {
    build(module, program, interner, None, None)
}

/// A program lowered into a module: its code and data are defined and the module is not finished.
pub(crate) struct Lowered<M> {
    pub module: M,
    /// The exported function that runs start-up and the entry.
    pub run: ClifFunc,
    pub faults: FaultTable,
    pub types: TypeMap,
    /// For each function its call sites as the offset of the return address and the frame there.
    pub safepoints: Vec<(ClifFunc, Vec<(u32, FrameDescriptor)>)>,
    /// The name, size and positions of each function that has call sites.
    pub debug: Vec<(ClifFunc, Debug)>,
    /// The static data that holds references, with where the references are.
    pub roots: Vec<(DataId, Vec<Slot>)>,
    pub link: LinkTable,
    pub native_slots: Option<DataId>,
    pub failure: Option<DataId>,
}

fn build(module: &Module, program: &mut Program, interner: &Interner, entry: Option<EntryKind>, error_text: Option<FuncId>) -> Result<Compiled, CompileError> {
    let mut builder = JITBuilder::with_isa(isa(false)?, default_libcall_names());
    for entry in Entry::ALL {
        builder.symbol(entry.name(), mote_runtime::address(entry) as *const u8);
    }
    builder.symbol(helpers::PRINT, mote_print as *const u8);
    builder.symbol(helpers::FMOD, mote_fmod as *const u8);
    builder.symbol(helpers::SHOW, mote_show as *const u8);
    builder.symbol(helpers::COMPARE, mote_string_compare as *const u8);
    builder.symbol(helpers::CUT, mote_string_cut as *const u8);
    builder.symbol(helpers::MATH, mote_math as *const u8);
    builder.symbol(helpers::TEXT, mote_text as *const u8);
    builder.symbol(helpers::SYSTEM, mote_system as *const u8);
    builder.symbol(helpers::VALUE, mote_value as *const u8);
    builder.symbol(dynamic::DYN, mote_dyn as *const u8);
    builder.symbol(loader::NATIVE, mote_native as *const u8);
    let Lowered { module: mut jit, run, faults, types, safepoints, debug, roots, link, native_slots, failure } = lower(JITModule::new(builder), module, program, interner, entry, error_text, &[])?;
    jit.finalize_definitions().map_err(CompileError::internal)?;
    let address = jit.get_finalized_function(run) as usize;
    let meta = meta::build(program, interner, &types);
    let mut code = CodeMap::new();
    let mut functions = Vec::new();
    for ((clif, sites), (_, info)) in safepoints.into_iter().zip(debug) {
        let start = jit.get_finalized_function(clif) as usize;
        for (offset, frame) in sites {
            code.add_frame(start + offset as usize, frame);
        }
        functions.push((start, info.size as usize, info.name.clone()));
        code.add_function(start, FunctionInfo { name: info.name, size: info.size as usize, rows: info.rows });
    }
    for (data, slots) in roots {
        code.add_root(GlobalRoot { address: jit.get_finalized_data(data).0 as usize, slots });
    }
    Ok(Compiled { jit, faults, types, meta, run: address, link, native_slots, files: Vec::new(), failure, code, functions, heap: HeapConfig::default(), scheduler: mote_scheduler::Config::new(), feeds: Vec::new(), args: Vec::new(), sink: None })
}

/// Lowers the functions start-up and the entry reach into the module, with the data they use; the slots of a library that `statics` marks are relocations to its symbols.
pub(crate) fn lower<M: ClifModule>(mut jit: M, module: &Module, program: &mut Program, interner: &Interner, entry: Option<EntryKind>, error_text: Option<FuncId>, statics: &[bool]) -> Result<Lowered<M>, CompileError> {
    let types = typetable::build(module, program, interner);
    let program: &Program = program;
    let ok_tag = result_variant(program, interner, "Ok");
    let found = entry.and_then(|entry| module.entries.iter().find(|candidate| candidate.kind == entry).map(|candidate| candidate.func));
    if found.is_none() && entry.is_some_and(|entry| entry != EntryKind::Main) {
        return Err(CompileError::internal("the program has no such entry"));
    }
    let result_ty = found.map(|entry| module.functions[entry].ret);
    let result_error = result_ty.and_then(|ty| match program.types.kind(ty) {
        TyKind::Adt { def, args } if program.lang.get(&LangItem::Result) == Some(def) && args.len() == 2 => Some(args[1]),
        _ => None,
    });
    let exit_int = result_ty.is_some_and(|ty| matches!(program.types.kind(ty), TyKind::Int(_)));
    let entries: Vec<FuncId> = module.start.into_iter().chain(found).collect();
    let roots: Vec<FuncId> = entries.iter().copied().chain(error_text.filter(|_| result_error.is_some())).collect();
    let Reach { order, globals, valued } = reachable(module, &roots);
    let closure_conv: HashSet<FuncId> = order.iter().copied().filter(|&id| valued.contains(&id) || matches!(module.functions[id].kind, FunctionKind::Closure { .. } | FunctionKind::Native(_) | FunctionKind::Resume { .. })).collect();

    let host = jit.isa().default_call_conv();

    let import = |jit: &mut M, name: &str, signature: &Signature| jit.declare_function(name, Linkage::Import, signature).map_err(CompileError::internal);
    use mote_abi::Scalar::{F64, I32, I64};
    let imports = Imports {
        fault: import(&mut jit, Entry::Fault.name(), &c_signature(Entry::Fault.params(), Entry::Fault.results(), host))?,
        alloc: import(&mut jit, Entry::Alloc.name(), &c_signature(Entry::Alloc.params(), Entry::Alloc.results(), host))?,
        alloc_array: import(&mut jit, Entry::AllocArray.name(), &c_signature(Entry::AllocArray.params(), Entry::AllocArray.results(), host))?,
        print: import(&mut jit, helpers::PRINT, &c_signature(&[I64, I64, I32, I32], &[], host))?,
        show: import(&mut jit, helpers::SHOW, &c_signature(&[I64, I32, I64], &[I64], host))?,
        compare: import(&mut jit, helpers::COMPARE, &c_signature(&[I64, I64], &[I64], host))?,
        cut: import(&mut jit, helpers::CUT, &c_signature(&[I64, I64, I64], &[I32], host))?,
        math: import(&mut jit, helpers::MATH, &c_signature(&[I32, F64, F64], &[F64], host))?,
        text: import(&mut jit, helpers::TEXT, &c_signature(&[I64, I32, I64, I64, I64, I32, I32, I32], &[I64], host))?,
        system: import(&mut jit, helpers::SYSTEM, &c_signature(&[I64, I32, I64, I32, I32, I32], &[I64], host))?,
        value: import(&mut jit, helpers::VALUE, &c_signature(&[I64, I32, I32, I64, I64], &[I64], host))?,
        dynamic: import(&mut jit, dynamic::DYN, &c_signature(&[I64, I32, I64, I64, I64, I64], &[I64], host))?,
        native: import(&mut jit, loader::NATIVE, &c_signature(&[I64, I32, I64, I64], &[I64], host))?,
        fault_message: import(&mut jit, Entry::FaultMessage.name(), &c_signature(Entry::FaultMessage.params(), Entry::FaultMessage.results(), host))?,
        poll: import(&mut jit, Entry::Poll.name(), &c_signature(Entry::Poll.params(), Entry::Poll.results(), host))?,
        fmod: import(&mut jit, helpers::FMOD, &c_signature(&[F64, F64], &[F64], host))?,
        task_spawn: import(&mut jit, Entry::TaskSpawn.name(), &c_signature(Entry::TaskSpawn.params(), Entry::TaskSpawn.results(), host))?,
        task_await: import(&mut jit, Entry::TaskAwait.name(), &c_signature(Entry::TaskAwait.params(), Entry::TaskAwait.results(), host))?,
        task_cancel: import(&mut jit, Entry::TaskCancel.name(), &c_signature(Entry::TaskCancel.params(), Entry::TaskCancel.results(), host))?,
        task_ready: import(&mut jit, Entry::TaskReady.name(), &c_signature(Entry::TaskReady.params(), Entry::TaskReady.results(), host))?,
        task_message: import(&mut jit, Entry::TaskMessage.name(), &c_signature(Entry::TaskMessage.params(), Entry::TaskMessage.results(), host))?,
        scope_begin: import(&mut jit, Entry::ScopeBegin.name(), &c_signature(Entry::ScopeBegin.params(), Entry::ScopeBegin.results(), host))?,
        scope_end: import(&mut jit, Entry::ScopeEnd.name(), &c_signature(Entry::ScopeEnd.params(), Entry::ScopeEnd.results(), host))?,
        channel_new: import(&mut jit, Entry::ChannelNew.name(), &c_signature(Entry::ChannelNew.params(), Entry::ChannelNew.results(), host))?,
        channel_end: import(&mut jit, Entry::ChannelEnd.name(), &c_signature(Entry::ChannelEnd.params(), Entry::ChannelEnd.results(), host))?,
        channel_send: import(&mut jit, Entry::ChannelSend.name(), &c_signature(Entry::ChannelSend.params(), Entry::ChannelSend.results(), host))?,
        channel_recv: import(&mut jit, Entry::ChannelRecv.name(), &c_signature(Entry::ChannelRecv.params(), Entry::ChannelRecv.results(), host))?,
        sender_close: import(&mut jit, Entry::SenderClose.name(), &c_signature(Entry::SenderClose.params(), Entry::SenderClose.results(), host))?,
        sender_clone: import(&mut jit, Entry::SenderClone.name(), &c_signature(Entry::SenderClone.params(), Entry::SenderClone.results(), host))?,
        receiver_close: import(&mut jit, Entry::ReceiverClose.name(), &c_signature(Entry::ReceiverClose.params(), Entry::ReceiverClose.results(), host))?,
        receiver_clone: import(&mut jit, Entry::ReceiverClone.name(), &c_signature(Entry::ReceiverClone.params(), Entry::ReceiverClone.results(), host))?,
        receiver_closed: import(&mut jit, Entry::ReceiverClosed.name(), &c_signature(Entry::ReceiverClosed.params(), Entry::ReceiverClosed.results(), host))?,
        mutex_new: import(&mut jit, Entry::MutexNew.name(), &c_signature(Entry::MutexNew.params(), Entry::MutexNew.results(), host))?,
        mutex_take: import(&mut jit, Entry::MutexTake.name(), &c_signature(Entry::MutexTake.params(), Entry::MutexTake.results(), host))?,
        mutex_put: import(&mut jit, Entry::MutexPut.name(), &c_signature(Entry::MutexPut.params(), Entry::MutexPut.results(), host))?,
        once_new: import(&mut jit, Entry::OnceNew.name(), &c_signature(Entry::OnceNew.params(), Entry::OnceNew.results(), host))?,
        once_begin: import(&mut jit, Entry::OnceBegin.name(), &c_signature(Entry::OnceBegin.params(), Entry::OnceBegin.results(), host))?,
        once_finish: import(&mut jit, Entry::OnceFinish.name(), &c_signature(Entry::OnceFinish.params(), Entry::OnceFinish.results(), host))?,
        once_value: import(&mut jit, Entry::OnceValue.name(), &c_signature(Entry::OnceValue.params(), Entry::OnceValue.results(), host))?,
        sleep: import(&mut jit, Entry::Sleep.name(), &c_signature(Entry::Sleep.params(), Entry::Sleep.results(), host))?,
        task_await_timeout: import(&mut jit, Entry::TaskAwaitTimeout.name(), &c_signature(Entry::TaskAwaitTimeout.params(), Entry::TaskAwaitTimeout.results(), host))?,
        channel_recv_timeout: import(&mut jit, Entry::ChannelRecvTimeout.name(), &c_signature(Entry::ChannelRecvTimeout.params(), Entry::ChannelRecvTimeout.results(), host))?,
        port_new: import(&mut jit, Entry::PortNew.name(), &c_signature(Entry::PortNew.params(), Entry::PortNew.results(), host))?,
        port_ref: import(&mut jit, Entry::PortRef.name(), &c_signature(Entry::PortRef.params(), Entry::PortRef.results(), host))?,
        defer: import(&mut jit, Entry::Defer.name(), &c_signature(Entry::Defer.params(), Entry::Defer.results(), host))?,
        undefer: import(&mut jit, Entry::Undefer.name(), &c_signature(Entry::Undefer.params(), Entry::Undefer.results(), host))?,
        scope_end_error: import(&mut jit, Entry::ScopeEndError.name(), &c_signature(Entry::ScopeEndError.params(), Entry::ScopeEndError.results(), host))?,
        native_blocking: import(&mut jit, Entry::NativeBlocking.name(), &c_signature(Entry::NativeBlocking.params(), Entry::NativeBlocking.results(), host))?,
    };

    let mut funcs: HashMap<FuncId, ClifFunc> = HashMap::new();
    let mut signatures: HashMap<FuncId, Signature> = HashMap::new();
    for &id in &order {
        let sig = function_signature(&module.functions[id], closure_conv.contains(&id), &types)?;
        let clif = jit.declare_function(&format!("f{}", id.index()), Linkage::Local, &sig).map_err(CompileError::internal)?;
        funcs.insert(id, clif);
        signatures.insert(id, sig);
    }
    let mut cells: HashMap<DefId, DataId> = HashMap::new();
    for (position, &def) in globals.iter().enumerate() {
        let DefKind::Global(global) = &program.defs[def].kind else { return Err(CompileError::internal("a global that is not a global")) };
        let data = jit.declare_data(&format!("g{position}"), Linkage::Local, true, false).map_err(CompileError::internal)?;
        let mut description = DataDescription::new();
        description.define_zeroinit(types.table.layout(types.id(global.ty)).size.div_ceil(8).max(1) as usize * 8);
        jit.define_data(data, &description).map_err(CompileError::internal)?;
        cells.insert(def, data);
    }

    let mut strings: HashMap<Symbol, DataId> = HashMap::new();
    for &id in &order {
        for text in string_constants(&module.functions[id]) {
            if strings.contains_key(&text) {
                continue;
            }
            let contents = string_object(types.id(TyId::STRING), interner.resolve(text));
            let data = jit.declare_data(&format!("s{}", strings.len()), Linkage::Local, false, false).map_err(CompileError::internal)?;
            let mut description = DataDescription::new();
            description.define(contents.into_boxed_slice());
            description.set_align(8);
            jit.define_data(data, &description).map_err(CompileError::internal)?;
            strings.insert(text, data);
        }
    }

    let mut type_names: HashMap<TyId, DataId> = HashMap::new();
    for &id in &order {
        let function = &module.functions[id];
        for (_, block) in function.blocks.iter() {
            for inst in &block.insts {
                let Op::Call { callee: Callee::Intrinsic(name), args, .. } = &inst.op else { continue };
                if interner.resolve(*name) != "typeof" {
                    continue;
                }
                let Some(arg) = args.first() else { continue };
                let ty = match arg {
                    Operand::Reg(reg) => function.regs[*reg].ty,
                    Operand::Const(constant) => const_ty(constant),
                };
                let named: Vec<TyId> = match program.types.kind(ty) {
                    TyKind::Union(members) => members.clone(),
                    _ => vec![ty],
                };
                for ty in named {
                    if type_names.contains_key(&ty) {
                        continue;
                    }
                    let contents = string_object(types.id(TyId::STRING), &program.render_source(ty, interner));
                    let data = jit.declare_data(&format!("t{}", type_names.len()), Linkage::Local, false, false).map_err(CompileError::internal)?;
                    let mut description = DataDescription::new();
                    description.define(contents.into_boxed_slice());
                    description.set_align(8);
                    jit.define_data(data, &description).map_err(CompileError::internal)?;
                    type_names.insert(ty, data);
                }
            }
        }
    }

    let mut meta_tables: HashMap<&'static str, DataId> = HashMap::new();
    let mut wanted: Vec<(&'static str, mote_abi::TypeId)> = Vec::new();
    for &id in &order {
        let function = &module.functions[id];
        for (_, block) in function.blocks.iter() {
            for inst in &block.insts {
                let Op::Call { callee: Callee::Intrinsic(name), .. } = &inst.op else { continue };
                let key = match interner.resolve(*name) {
                    "meta_ints" => "meta_ints",
                    "meta_texts" => "meta_texts",
                    _ => continue,
                };
                let Some(dest) = inst.dest else { continue };
                if wanted.iter().all(|(found, _)| *found != key) {
                    wanted.push((key, types.id(function.regs[dest].ty)));
                }
            }
        }
    }
    if !wanted.is_empty() {
        let table = mote_ir::reflect::encode(program);
        for (key, array) in wanted {
            let data = jit.declare_data(key, Linkage::Local, false, false).map_err(CompileError::internal)?;
            let mut description = DataDescription::new();
            description.set_align(8);
            let mut contents = Header::new(array, FLAG_STATIC).bits().to_le_bytes().to_vec();
            if key == "meta_ints" {
                contents.extend((table.ints.len() as u64).to_le_bytes());
                for number in &table.ints {
                    contents.extend((*number as i64).to_le_bytes());
                }
                description.define(contents.into_boxed_slice());
            } else {
                contents.extend((table.texts.len() as u64).to_le_bytes());
                let first = contents.len();
                contents.resize(first + table.texts.len() * 8, 0);
                description.define(contents.into_boxed_slice());
                for (at, text) in table.texts.iter().enumerate() {
                    let string = jit.declare_data(&format!("x{at}"), Linkage::Local, false, false).map_err(CompileError::internal)?;
                    let mut inner = DataDescription::new();
                    inner.define(string_object(types.id(TyId::STRING), text).into_boxed_slice());
                    inner.set_align(8);
                    jit.define_data(string, &inner).map_err(CompileError::internal)?;
                    let target = jit.declare_data_in_data(string, &mut description);
                    description.write_data_addr((first + at * 8) as u32, target, 0);
                }
            }
            jit.define_data(data, &description).map_err(CompileError::internal)?;
            meta_tables.insert(key, data);
        }
    }

    let (link, failures) = program.link_table(interner);
    let natives: HashMap<DefId, usize> = program.linked_natives().into_iter().filter(|def| failures.iter().all(|failure| failure.def != *def)).enumerate().map(|(index, def)| (def, index)).collect();
    let native_slots = if link.functions.is_empty() {
        None
    } else {
        let data = jit.declare_data("mote_natives", Linkage::Export, true, false).map_err(CompileError::internal)?;
        let mut description = DataDescription::new();
        let linked = |index: usize| statics.get(link.functions[index].library as usize).copied().unwrap_or(false);
        if (0..link.functions.len()).any(linked) {
            description.define(vec![0; link.functions.len() * 8].into_boxed_slice());
        } else {
            description.define_zeroinit(link.functions.len() * 8);
        }
        description.set_align(8);
        for index in (0..link.functions.len()).filter(|&index| linked(index)) {
            let symbol = jit.declare_function(&link.functions[index].symbol, Linkage::Import, &c_signature(&[], &[], host)).map_err(CompileError::internal)?;
            let target = jit.declare_func_in_data(symbol, &mut description);
            description.write_function_addr((index * 8) as u32, target);
        }
        jit.define_data(data, &description).map_err(CompileError::internal)?;
        Some(data)
    };
    let mut thunks: HashMap<usize, ClifFunc> = HashMap::new();
    for index in (0..link.functions.len()).filter(|&index| link.functions[index].sig.blocking) {
        let thunk_signature = c_signature(&[mote_abi::Scalar::I64], &[], host);
        let id = jit.declare_function(&format!("thunk{index}"), Linkage::Local, &thunk_signature).map_err(CompileError::internal)?;
        thunks.insert(index, id);
    }

    let mut task_thunks: HashMap<mote_abi::TypeId, ClifFunc> = HashMap::new();
    for &id in &order {
        let function = &module.functions[id];
        for (_, block) in function.blocks.iter() {
            for inst in &block.insts {
                let starts = match &inst.op {
                    Op::Spawn { .. } => true,
                    Op::Call { callee: Callee::Intrinsic(name), .. } => interner.resolve(*name) == "spawn_call",
                    _ => false,
                };
                let Some(dest) = inst.dest.filter(|_| starts) else { continue };
                let task = types.id(function.regs[dest].ty);
                if task_thunks.contains_key(&task) {
                    continue;
                }
                let clif = jit.declare_function(&format!("task{}", task.raw()), Linkage::Local, &c_signature(&[I64, I64, I64], &[], host)).map_err(CompileError::internal)?;
                task_thunks.insert(task, clif);
            }
        }
    }

    let mut viewed_tasks: HashSet<mote_abi::TypeId> = HashSet::new();
    for &id in &order {
        let function = &module.functions[id];
        for (_, block) in function.blocks.iter() {
            for inst in &block.insts {
                if let (Op::Call { callee: Callee::Intrinsic(name), .. }, Some(dest)) = (&inst.op, inst.dest)
                    && interner.resolve(*name) == "task_view"
                {
                    viewed_tasks.insert(types.id(function.regs[dest].ty));
                }
            }
        }
    }

    let mut defer_thunks: HashMap<FuncId, ClifFunc> = HashMap::new();
    for &id in &order {
        for (_, block) in module.functions[id].blocks.iter() {
            for inst in &block.insts {
                let Op::Defer { func, .. } = &inst.op else { continue };
                if defer_thunks.contains_key(func) {
                    continue;
                }
                let clif = jit.declare_function(&format!("close{}", func.index()), Linkage::Local, &c_signature(&[I64, I64], &[], host)).map_err(CompileError::internal)?;
                defer_thunks.insert(*func, clif);
            }
        }
    }

    let mut statics: HashMap<FuncId, DataId> = HashMap::new();
    for &id in order.iter().filter(|id| closure_conv.contains(id) && module.functions[**id].captures.is_empty() && !matches!(module.functions[**id].kind, FunctionKind::Resume { .. })) {
        let data = jit.declare_data(&format!("c{}", id.index()), Linkage::Local, false, false).map_err(CompileError::internal)?;
        let mut description = DataDescription::new();
        let mut contents = Header::new(types.closure(id), FLAG_STATIC).bits().to_le_bytes().to_vec();
        contents.extend([0; 8]);
        description.define(contents.into_boxed_slice());
        description.set_align(8);
        let code = jit.declare_func_in_data(funcs[&id], &mut description);
        description.write_function_addr(CLOSURE_CODE_OFFSET, code);
        jit.define_data(data, &description).map_err(CompileError::internal)?;
        statics.insert(id, data);
    }

    let mut faults = FaultTable::new();
    let mut context = jit.make_context();
    let mut builder_context = FunctionBuilderContext::new();
    let mut safepoints: Vec<(ClifFunc, Vec<(u32, FrameDescriptor)>)> = Vec::new();
    let mut debug: Vec<(ClifFunc, Debug)> = Vec::new();
    for &id in &order {
        let clif = funcs[&id];
        context.func.signature = signatures[&id].clone();
        context.func.name = UserFuncName::user(0, id.index() as u32);
        let frame = {
            let env = Env { module, program, interner, types: &types, ok_tag, clif: &jit, funcs: &funcs, globals: &cells, strings: &strings, type_names: &type_names, meta_tables: &meta_tables, natives: &natives, link: &link, native_slots, thunks: &thunks, task_thunks: &task_thunks, viewed_tasks: &viewed_tasks, defer_thunks: &defer_thunks, closure_conv: &closure_conv, statics: &statics, imports };
            let builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
            Lowering::lower(builder, env, &mut faults, id)?
        };
        jit.define_function(clif, &mut context).map_err(|error| CompileError::internal(format!("{error:?}")))?;
        let compiled = context.compiled_code().ok_or_else(|| CompileError::internal("a function with no machine code"))?;
        safepoints.push((clif, frames::describe(compiled, &frame, &types.table)?));
        debug.push((clif, debug::describe(debug::name_of(module, program, interner, id), compiled)));
        jit.clear_context(&mut context);
    }

    for (&index, &id) in &thunks {
        let slots = native_slots.ok_or_else(|| CompileError::internal("thunks with no native table"))?;
        context.func.signature = c_signature(&[mote_abi::Scalar::I64], &[], host);
        context.func.name = UserFuncName::user(2, index as u32);
        {
            let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
            native::thunk(&mut builder, &jit, &link.functions[index].sig, slots, index, host);
            builder.finalize();
        }
        jit.define_function(id, &mut context).map_err(|error| CompileError::internal(format!("{error:?}")))?;
        jit.clear_context(&mut context);
    }

    let mut task_thunks: Vec<_> = task_thunks.into_iter().collect();
    task_thunks.sort_by_key(|(task, _)| task.raw());
    for (task, id) in task_thunks {
        context.func.signature = c_signature(&[I64, I64, I64], &[], host);
        context.func.name = UserFuncName::user(3, task.raw());
        {
            let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
            tasks::thunk(&mut builder, &types.table, task)?;
            builder.finalize();
        }
        jit.define_function(id, &mut context).map_err(|error| CompileError::internal(format!("{error:?}")))?;
        jit.clear_context(&mut context);
    }

    let mut defer_thunks: Vec<_> = defer_thunks.into_iter().collect();
    defer_thunks.sort_by_key(|(func, _)| func.index());
    for (func, id) in defer_thunks {
        let function = &module.functions[func];
        let [param] = function.params[..] else { return Err(CompileError::internal("a close with parameters besides the resource")) };
        if !function.out_params.is_empty() {
            return Err(CompileError::internal("a close with parameters besides the resource"));
        }
        let target = jit.declare_func_in_func(funcs[&func], &mut context.func);
        context.func.signature = c_signature(&[I64, I64], &[], host);
        context.func.name = UserFuncName::user(4, func.index() as u32);
        {
            let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
            tasks::close_thunk(&mut builder, &types.table, target, types.id(function.regs[param].ty), types.id(function.ret), closure_conv.contains(&func))?;
            builder.finalize();
        }
        jit.define_function(id, &mut context).map_err(|error| CompileError::internal(format!("{error:?}")))?;
        jit.clear_context(&mut context);
    }

    let failure = match (result_error, exit_int) {
        (Some(_), _) | (_, true) => {
            let data = jit.declare_data("mote_failure", Linkage::Export, true, false).map_err(CompileError::internal)?;
            let mut description = DataDescription::new();
            description.define_zeroinit(24);
            description.set_align(8);
            jit.define_data(data, &description).map_err(CompileError::internal)?;
            Some(data)
        }
        _ => None,
    };
    let held = match result_error {
        Some(_) => {
            let data = jit.declare_data("entry_result", Linkage::Local, true, false).map_err(CompileError::internal)?;
            let mut description = DataDescription::new();
            description.define_zeroinit(types.table.layout(types.id(result_ty.expect("a result"))).size.div_ceil(8).max(1) as usize * 8);
            description.set_align(8);
            jit.define_data(data, &description).map_err(CompileError::internal)?;
            Some(data)
        }
        None => None,
    };
    let run = {
        let sig = c_signature(&[mote_abi::Scalar::I64], &[], host);
        let run = jit.declare_function("mote_run", Linkage::Export, &sig).map_err(CompileError::internal)?;
        context.func.signature = sig;
        context.func.name = UserFuncName::user(1, 0);
        {
            let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
            let first = builder.create_block();
            builder.append_block_params_for_function_params(first);
            builder.switch_to_block(first);
            let task = builder.block_params(first)[0];
            for &id in &entries {
                let function = &module.functions[id];
                let is_entry = Some(id) == found;
                let mut arguments = vec![task];
                let kept = held.filter(|_| is_entry);
                if matches!(passing(&types.table, types.id(function.ret)), Passing::Memory) {
                    let slot = match kept {
                        Some(data) => {
                            let global = jit.declare_data_in_func(data, builder.func);
                            builder.ins().symbol_value(types::I64, global)
                        }
                        None => {
                            let size = types.table.layout(types.id(function.ret)).size.div_ceil(8) * 8;
                            let slot = builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, size, 3));
                            builder.ins().stack_addr(types::I64, slot, 0)
                        }
                    };
                    arguments.push(slot);
                }
                if closure_conv.contains(&id) {
                    arguments.push(builder.ins().iconst(types::I64, 0));
                }
                let callee = jit.declare_func_in_func(funcs[&id], builder.func);
                let call = builder.ins().call(callee, &arguments);
                if let (Some(data), Passing::Registers(_)) = (kept, passing(&types.table, types.id(function.ret))) {
                    let global = jit.declare_data_in_func(data, builder.func);
                    let base = builder.ins().symbol_value(types::I64, global);
                    let mut leaves = Vec::new();
                    leaves_of(&types.table, types.id(function.ret), 0, &mut leaves);
                    let results = builder.inst_results(call).to_vec();
                    for ((offset, _), value) in leaves.iter().zip(results) {
                        emit_store(&mut builder, value, Addr { base, offset: *offset as i32 });
                    }
                }
                if let (true, true, Some(failure)) = (is_entry, exit_int, failure) {
                    let code = builder.inst_results(call)[0];
                    let wide = if builder.func.dfg.value_type(code) == types::I64 { code } else { builder.ins().sextend(types::I64, code) };
                    let cell = jit.declare_data_in_func(failure, builder.func);
                    let cell = builder.ins().symbol_value(types::I64, cell);
                    emit_store(&mut builder, wide, Addr { base: cell, offset: 16 });
                }
            }
            if let (Some(held), Some(failure), Some(error)) = (held, failure, result_error) {
                let result_id = types.id(result_ty.expect("a result"));
                let info = types.table.get(result_id);
                let ok = ok_tag.ok_or_else(|| CompileError::internal("no Ok variant"))?;
                let err = info.variants.iter().enumerate().map(|(at, _)| at as u32).find(|&at| at != ok).ok_or_else(|| CompileError::internal("no Err variant"))?;
                let payload = info.variants[err as usize].fields.first().copied();
                let global = jit.declare_data_in_func(held, builder.func);
                let base = builder.ins().symbol_value(types::I64, global);
                let tag_ty = match info.tag_size {
                    1 => types::I8,
                    2 => types::I16,
                    _ => types::I32,
                };
                let tag = builder.ins().load(tag_ty, MemFlags::trusted(), base, 0);
                let failed = builder.ins().icmp_imm(IntCC::NotEqual, tag, i64::from(ok));
                let on_err = builder.create_block();
                let done = builder.create_block();
                builder.ins().brif(failed, on_err, &[], done, &[]);
                builder.switch_to_block(on_err);
                let cell = jit.declare_data_in_func(failure, builder.func);
                let cell = builder.ins().symbol_value(types::I64, cell);
                let one = builder.ins().iconst(types::I64, 1);
                emit_store(&mut builder, one, Addr { base: cell, offset: 0 });
                let is_error = matches!(program.types.kind(error), TyKind::Adt { def, .. } if program.defs[*def].lang == Some(LangItem::Error));
                if let Some(payload) = payload {
                    let text = if is_error {
                        let object = builder.ins().load(types::I64, MemFlags::trusted(), base, payload.offset as i32);
                        let message = types.table.get(payload.ty).object.as_ref().and_then(|shape| shape.fields.get(1)).copied().ok_or_else(|| CompileError::internal("an Error with no message"))?;
                        Some(builder.ins().load(types::I64, MemFlags::trusted(), object, message.offset as i32))
                    } else if let Some(function) = error_text {
                        let mut arguments = vec![task];
                        if matches!(passing(&types.table, types.id(module.functions[function].ret)), Passing::Memory) {
                            return Err(CompileError::internal("an error text that is not a reference"));
                        }
                        if closure_conv.contains(&function) {
                            arguments.push(builder.ins().iconst(types::I64, 0));
                        }
                        match passing(&types.table, payload.ty) {
                            Passing::Registers(_) => {
                                let mut leaves = Vec::new();
                                leaves_of(&types.table, payload.ty, 0, &mut leaves);
                                for (offset, scalar) in leaves {
                                    arguments.push(builder.ins().load(crate::function::clif_type(scalar), MemFlags::trusted(), base, (payload.offset + offset) as i32));
                                }
                            }
                            Passing::Memory => arguments.push(builder.ins().iadd_imm(base, i64::from(payload.offset))),
                        }
                        let callee = jit.declare_func_in_func(funcs[&function], builder.func);
                        let call = builder.ins().call(callee, &arguments);
                        builder.inst_results(call).first().copied()
                    } else {
                        None
                    };
                    if let Some(text) = text {
                        emit_store(&mut builder, text, Addr { base: cell, offset: 8 });
                    }
                }
                builder.ins().jump(done, &[]);
                builder.switch_to_block(done);
            }
            builder.ins().return_(&[]);
            builder.seal_all_blocks();
            builder.finalize();
        }
        jit.define_function(run, &mut context).map_err(|error| CompileError::internal(format!("{error:?}")))?;
        jit.clear_context(&mut context);
        run
    };
    let mut roots: Vec<(DataId, Vec<Slot>)> = Vec::new();
    for def in &globals {
        let DefKind::Global(global) = &program.defs[*def].kind else { return Err(CompileError::internal("a global that is not a global")) };
        roots.push((cells[def], types.table.get(types.id(global.ty)).slots.clone()));
    }
    if let (Some(data), Some(ty)) = (held, result_ty) {
        roots.push((data, types.table.get(types.id(ty)).slots.clone()));
    }
    if let Some(data) = failure {
        roots.push((data, vec![Slot::Ref(8)]));
    }
    roots.retain(|(_, slots)| !slots.is_empty());
    Ok(Lowered { module: jit, run, faults, types, safepoints, debug, roots, link, native_slots, failure })
}

#[cfg(test)]
mod tests;
