use std::collections::{HashMap, HashSet, VecDeque};
use isa::code::CodeObject;
use isa::encoding::*;
use isa::opcode::Opcode;
use isa::value::{CHANNEL_TYPE_ID, GENERATOR_TYPE_ID, TypeDescriptor, Value};
use crate::ast::*;
use crate::reg_alloc::RegisterAllocator;
use crate::span::Span;
use crate::types::Type;

mod program;
mod defaults;
pub use program::{CompiledProgram, SourceFile};
mod lambda_scan;
mod cells;
mod constructors;
mod expr;
mod builtin_methods;
mod stmt;
mod patterns;
mod variadic;
mod type_args;
mod shared;
mod var_params;
mod option;
mod null_safe;
mod layout;
mod abi;
mod window;
mod unions;

fn may_hold_value_object(ty: Option<&Type>) -> bool {
    match ty {
        Some(Type::Nullable(inner)) => may_hold_value_object(Some(inner)),
        Some(
            Type::Int | Type::Float | Type::Bool | Type::Char | Type::String | Type::Null | Type::Bytes
            | Type::Class { .. } | Type::List(_) | Type::Map(..) | Type::Set(_)
            | Type::Receiver(_) | Type::Sender(_) | Type::Stream(_) | Type::Shared(_) | Type::Tuple(_),
        ) => false,
        _ => true,
    }
}

fn iterated_element(ty: Option<&Type>) -> Option<&Type> {
    match ty {
        Some(Type::List(e) | Type::Set(e) | Type::Receiver(e) | Type::Stream(e)) => Some(e),
        _ => None,
    }
}

fn binop_opcode(op: BinaryOp) -> Opcode {
    match op {
        BinaryOp::Add => Opcode::ADD,
        BinaryOp::Sub => Opcode::SUB,
        BinaryOp::Mul => Opcode::MUL,
        BinaryOp::Div => Opcode::DIV,
        BinaryOp::Mod => Opcode::MOD,
        BinaryOp::Eq => Opcode::EQ,
        BinaryOp::NotEq => Opcode::NE,
        BinaryOp::Lt => Opcode::LT,
        BinaryOp::LtEq => Opcode::LE,
        BinaryOp::Gt => Opcode::GT,
        BinaryOp::GtEq => Opcode::GE,
        BinaryOp::And => Opcode::AND,
        BinaryOp::Or => Opcode::OR,
        BinaryOp::BitAnd => Opcode::BAND,
        BinaryOp::BitOr => Opcode::BOR,
        BinaryOp::BitXor => Opcode::BXOR,
        BinaryOp::Shl => Opcode::SHL,
        BinaryOp::Shr => Opcode::SHR,
    }
}

#[derive(Clone, Copy)]
pub(crate) enum CallTarget {
    Code(u16),
    Value(u8),
}

#[derive(Clone)]
pub(crate) enum Callee {
    Builtin(&'static str),
    Intrinsic(u8),
}

impl From<&'static str> for Callee {
    fn from(name: &'static str) -> Self {
        Callee::Builtin(name)
    }
}

impl From<u8> for Callee {
    fn from(idx: u8) -> Self {
        Callee::Intrinsic(idx)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum IterFlag {
    Yes,
    No,
    Reg(u8),
}

/// What a function does when a `scope` closes over a child's fault that nobody observed: answer an `Err`, or panic.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScopeFault {
    ErrText,
    ErrError,
    Panic,
}

#[derive(Default)]
struct LoopCtx {
    break_sites: Vec<usize>,
    continue_sites: Vec<usize>,
    region_depth: usize,
    with_depth: usize,
}

struct PendingLambda {
    code_idx: usize,
    params: Vec<String>,
    captures: Vec<String>,
    cell_captures: Vec<bool>,
    return_type: Option<TypeNode>,
    body: Vec<Stmt>,
    span: Span,
    returns_param: bool,
    prebuilt: Option<CodeObject>,
    module: String,
}

impl PendingLambda {
    fn prebuilt(code_idx: usize, code: CodeObject) -> Self {
        Self {
            code_idx,
            params: Vec::new(),
            captures: Vec::new(),
            cell_captures: Vec::new(),
            return_type: None,
            body: Vec::new(),
            span: Span::default(),
            returns_param: false,
            prebuilt: Some(code),
            module: String::new(),
        }
    }
}

use isa::type_term::{display_type_name, TypeTerm};
use type_args::HiddenTypeParams;
use option::Fallible;

#[derive(Clone, Debug)]
pub(crate) struct VariantInfo {
    type_idx: u16,
    arity: usize,
    enum_name: String,
}

/// Generates bytecode from a checked program.
pub struct CodeGenerator {
    code_objects: Vec<CodeObject>,
    type_descriptors: Vec<TypeDescriptor>,
    type_map: HashMap<String, u16>,
    class_names: std::collections::HashSet<String>,
    variant_map: HashMap<String, VariantInfo>,
    enum_variant_map: HashMap<(String, String), VariantInfo>,
    func_map: HashMap<String, usize>,
    default_fns: HashMap<String, Vec<Option<String>>>,
    variadic_fns: HashMap<String, usize>,
    global_map: HashMap<String, u16>,
    types: crate::stage::TypeTable,
    bind_copies: bool,
    match_enum: Option<String>,
    test_marks: Vec<TestMark>,
    test_mode: bool,
    test_filter: Option<(String, String)>,
    native_table: Vec<String>,
    loop_stack: Vec<LoopCtx>,
    regions: HashSet<Span>,
    region_heap: HashMap<Span, crate::regions::HeapReason>,
    region_vars: HashSet<String>,
    param_names: HashSet<String>,
    safe_uses: HashSet<Span>,
    region_sigs: HashMap<String, Vec<bool>>,
    value_structs: HashSet<String>,
    window_types: HashMap<String, (u16, usize)>,
    window_lets: HashSet<Span>,
    abi: HashMap<String, abi::AbiSig>,
    current_sig: Option<abi::AbiSig>,
    current_owner: Option<String>,
    window_params: HashSet<String>,
    keep_wr: Option<Span>,
    wr_result: Option<u8>,
    report_memory: bool,
    memory_sites: Vec<crate::regions::MemorySite>,
    reported_inits: HashSet<Span>,
    region_stack: Vec<usize>,
    with_stack: Vec<(usize, String, Span)>,
    current_insts: Vec<u32>,
    current_constants: Vec<Value>,
    current_strings: Vec<String>,
    current_spans: Vec<isa::code::SourceSpan>,
    function_type_cache: HashMap<usize, u16>,
    tuple_type_cache: HashMap<usize, u16>,
    cell_type: Option<u16>,
    instances: HashMap<(u64, usize, TypeTerm), u16>,
    hidden_params: HashMap<String, HiddenTypeParams>,
    class_type_params: HashMap<String, Vec<String>>,
    type_scope: HashMap<String, TypeTerm>,
    generic_fn_wrappers: HashMap<(String, Option<Vec<TypeTerm>>), usize>,
    recording_calls: HashSet<Span>,
    value_call_vars: Vec<bool>,
    cell_names: HashSet<String>,
    writeback: HashMap<String, Vec<usize>>,
    var_self_methods: HashSet<String>,
    var_args: HashMap<String, Vec<usize>>,
    fn_writeback: Vec<(u8, bool)>,
    ret_param: Option<String>,
    next_lambda_returns_param: bool,
    pending_lambdas: VecDeque<PendingLambda>,
    next_code_idx: usize,
    current_fn_fallible: bool,
    current_scope_fault: ScopeFault,
    module_prefixes: Vec<String>,
    module: String,
    reg_alloc: RegisterAllocator,
}

impl Default for CodeGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeGenerator {
    pub fn new() -> Self {
        Self {
            code_objects: Vec::new(),
            type_descriptors: Vec::new(),
            type_map: HashMap::new(),
            class_names: std::collections::HashSet::new(),
            variant_map: option::lang_variants().map(|(v, info)| (v.to_string(), info)).collect(),
            enum_variant_map: option::lang_variants().map(|(v, info)| (("Option".to_string(), v.to_string()), info)).collect(),
            func_map: HashMap::new(),
            default_fns: HashMap::new(),
            variadic_fns: HashMap::new(),
            global_map: HashMap::new(),
            types: crate::stage::TypeTable::default(),
            bind_copies: false,
            match_enum: None,
            test_marks: Vec::new(),
            test_mode: false,
            test_filter: None,
            native_table: Vec::new(),
            loop_stack: Vec::new(),
            regions: HashSet::new(),
            region_heap: HashMap::new(),
            region_vars: HashSet::new(),
            param_names: HashSet::new(),
            safe_uses: HashSet::new(),
            region_sigs: HashMap::new(),
            value_structs: HashSet::new(),
            window_types: HashMap::new(),
            window_lets: HashSet::new(),
            abi: HashMap::new(),
            current_sig: None,
            current_owner: None,
            window_params: HashSet::new(),
            keep_wr: None,
            wr_result: None,
            report_memory: false,
            memory_sites: Vec::new(),
            reported_inits: HashSet::new(),
            region_stack: Vec::new(),
            with_stack: Vec::new(),
            current_insts: Vec::new(),
            current_constants: Vec::new(),
            current_strings: Vec::new(),
            current_spans: Vec::new(),
            function_type_cache: HashMap::new(),
            tuple_type_cache: HashMap::new(),
            cell_type: None,
            instances: HashMap::new(),
            hidden_params: HashMap::new(),
            class_type_params: HashMap::new(),
            type_scope: HashMap::new(),
            generic_fn_wrappers: HashMap::new(),
            recording_calls: HashSet::new(),
            value_call_vars: Vec::new(),
            cell_names: HashSet::new(),
            writeback: HashMap::new(),
            var_self_methods: HashSet::new(),
            var_args: HashMap::new(),
            fn_writeback: Vec::new(),
            ret_param: None,
            next_lambda_returns_param: false,
            pending_lambdas: VecDeque::new(),
            next_code_idx: 0,
            current_fn_fallible: false,
            current_scope_fault: ScopeFault::Panic,
            module_prefixes: Vec::new(),
            module: String::new(),
            reg_alloc: RegisterAllocator::new(),
        }
    }

    /// Records where every struct literal lives, into `CompiledProgram::memory_sites`.
    pub fn with_memory_report(mut self) -> Self {
        self.report_memory = true;
        self
    }

    /// Names each module by its mangle prefix, in the order `compile_modules` receives them.
    pub fn with_module_prefixes(mut self, prefixes: Vec<String>) -> Self {
        self.module_prefixes = prefixes;
        self
    }

    fn enter_module(&mut self, i: usize) {
        self.module = self.module_prefixes.get(i).cloned().unwrap_or_default();
    }

    fn function_type_idx(&mut self, captures: usize) -> u16 {
        if let Some(&idx) = self.function_type_cache.get(&captures) {
            return idx;
        }
        let idx = self.type_descriptors.len() as u16;
        self.type_descriptors.push(TypeDescriptor::function_type(captures));
        self.function_type_cache.insert(captures, idx);
        idx
    }

    fn tuple_type_idx(&mut self, arity: usize) -> u16 {
        if let Some(&idx) = self.tuple_type_cache.get(&arity) {
            return idx;
        }
        let idx = self.type_descriptors.len() as u16;
        let mut td = TypeDescriptor::with_field_count(idx as u64, arity);
        td.by_content = true;
        self.type_descriptors.push(td);
        self.tuple_type_cache.insert(arity, idx);
        idx
    }

    fn emit_function_value(
        &mut self,
        code_idx: usize,
        capture_regs: &[u8],
        span: Span,
    ) -> Result<u8, (String, Span)> {
        if code_idx > u16::MAX as usize {
            return Err(("too many code objects for a function value".into(), span));
        }
        if capture_regs.len() > u8::MAX as usize {
            return Err(("a lambda captures more than 255 variables".into(), span));
        }
        let type_idx = self.function_type_idx(capture_regs.len());
        let dest = self.reg_alloc.alloc_temp();
        if matches!(self.types.at(span), Some(Type::Function { .. })) {
            self.emit_new_object(dest, type_idx, span);
        } else {
            self.current_insts.push(encode_ri(Opcode::NEWOBJ, dest, type_idx));
        }
        let idx_reg = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::LOADI, idx_reg, code_idx as u16));
        self.current_insts.push(encode_r3(Opcode::SETFIELD, dest, 0, idx_reg));
        self.reg_alloc.free_temp(idx_reg);
        for (i, &src) in capture_regs.iter().enumerate() {
            self.current_insts
                .push(encode_r3(Opcode::SETFIELD, dest, (i + 1) as u8, src));
        }
        Ok(dest)
    }

    fn compile_spawn(&mut self, body: &[Stmt], span: Span) -> Result<u8, (String, Span)> {
        let mut bound: HashSet<String> = HashSet::new();
        let mut captures: Vec<String> = Vec::new();
        self.lambda_free_vars(body, &mut bound, &mut captures);
        captures.retain(|n| self.reg_alloc.get_var(n).is_some());
        let mut loaded = Vec::new();
        let mut capture_regs: Vec<u8> = Vec::new();
        for n in &captures {
            let mut reg = self.reg_alloc.get_var(n).expect("filtered to bound names");
            if self.reg_alloc.is_cell(n) {
                let tmp = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r3(Opcode::GETFIELD, tmp, reg, 0));
                loaded.push(tmp);
                reg = tmp;
            }
            let copy = self.reg_alloc.alloc_temp();
            self.current_insts.push(encode_r2(Opcode::COPYVAL, copy, reg));
            loaded.push(copy);
            capture_regs.push(copy);
        }
        for (name, reg) in self.type_scope_captures() {
            captures.push(name);
            capture_regs.push(reg);
            loaded.push(reg);
        }

        let code_idx = self.next_code_idx;
        self.next_code_idx += 1;
        self.pending_lambdas.push_back(PendingLambda {
            code_idx,
            params: Vec::new(),
            cell_captures: vec![false; captures.len()],
            captures,
            return_type: None,
            body: body.to_vec(),
            span,
            returns_param: false,
            prebuilt: None,
            module: self.module.clone(),
        });
        let closure = self.emit_function_value(code_idx, &capture_regs, span)?;
        for tmp in loaded {
            self.reg_alloc.free_temp(tmp);
        }
        let handle = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r2(Opcode::SPAWN, handle, closure));
        self.reg_alloc.free_temp(closure);
        Ok(handle)
    }

    fn native_frame(&mut self, callee: &Callee, n: usize) -> (u8, u8) {
        match callee {
            Callee::Builtin(_) => {
                let dest = self.reg_alloc.alloc_block(n + 1);
                (dest.wrapping_add(1), dest)
            }
            Callee::Intrinsic(_) => {
                let arg_base = self.reg_alloc.alloc_block(n);
                (arg_base, self.reg_alloc.alloc_temp())
            }
        }
    }

    fn emit_native_op(&mut self, callee: &Callee, dest: u8, arg_base: u8) {
        match callee {
            Callee::Builtin(name) => {
                let slot = match self.native_table.iter().position(|n| n == name) {
                    Some(slot) => slot,
                    None => {
                        self.native_table.push(name.to_string());
                        self.native_table.len() - 1
                    }
                };
                self.current_insts.push(encode_callnativew(dest, slot as u16));
            }
            Callee::Intrinsic(id) => self.current_insts.push(encode_callintrinsic(dest, *id, arg_base)),
        }
    }

    fn emit_native_call(
        &mut self,
        callee: impl Into<Callee>,
        args: &[Expr],
        span: Span,
    ) -> Result<u8, (String, Span)> {
        let callee = callee.into();
        let n = args.len();
        let (arg_base, dest) = self.native_frame(&callee, n);
        if self.reg_alloc.overflowed() {
            return Err(("call expression needs more than 256 registers".into(), span));
        }
        for (i, arg) in args.iter().enumerate() {
            let r = self.compile_value(arg)?;
            let target = arg_base + i as u8;
            if r != target {
                self.current_insts.push(encode_r2(Opcode::MOVE, target, r));
            }
            self.reg_alloc.free_temp(r);
        }
        self.emit_native_op(&callee, dest, arg_base);
        self.reg_alloc.free_block(arg_base, n);
        Ok(dest)
    }

    fn emit_fallible_native_call(&mut self, name: &'static str, args: &[Expr], span: Span) -> Result<u8, (String, Span)> {
        let ok_type = self.variant_map.get("Ok").map(|v| v.type_idx).ok_or(("a fallible native needs `Result` in scope".to_string(), span))?;
        let err_type = self.variant_map.get("Err").map(|v| v.type_idx).ok_or(("a fallible native needs `Result` in scope".to_string(), span))?;
        let error_fn = self
            .func_map
            .iter()
            .find(|(k, _)| k.ends_with("__native_error"))
            .map(|(_, idx)| *idx)
            .ok_or(("a fallible native needs `std.error` in the program".to_string(), span))?;

        let n = args.len();
        let window = n.max(2);
        let result = self.reg_alloc.alloc_temp();
        let dest = self.reg_alloc.alloc_block(window + 1);
        if self.reg_alloc.overflowed() {
            return Err(("call expression needs more than 256 registers".into(), span));
        }
        for (i, arg) in args.iter().enumerate() {
            let r = self.compile_value(arg)?;
            let target = dest + 1 + i as u8;
            if r != target {
                self.current_insts.push(encode_r2(Opcode::MOVE, target, r));
            }
            self.reg_alloc.free_temp(r);
        }
        let slot = match self.native_table.iter().position(|n| n == name) {
            Some(slot) => slot,
            None => {
                self.native_table.push(name.to_string());
                self.native_table.len() - 1
            }
        };
        self.current_insts.push(encode_callnativef(dest, slot as u16));

        let zero = self.reg_alloc.alloc_temp();
        let is_ok = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::LOADI, zero, 0));
        self.current_insts.push(encode_r3(Opcode::LT, is_ok, dest + 1, zero));
        let to_fail = self.current_insts.len();
        self.current_insts.push(0);

        self.emit_new_object(result, ok_type, span);
        self.current_insts.push(encode_r3(Opcode::SETFIELD, result, 0, dest));
        let to_end = self.current_insts.len();
        self.current_insts.push(0);

        let fail_at = self.current_insts.len() as i32;
        self.current_insts[to_fail] = encode_jc(Opcode::JMPIFNOT, is_ok, (fail_at - to_fail as i32) as i16);
        self.reg_alloc.free_temp(is_ok);
        self.reg_alloc.free_temp(zero);
        let (arg_base, error) = self.call_frame(CallTarget::Code(0), 2);
        let error = error.expect("a code call has a destination");
        self.current_insts.push(encode_r2(Opcode::MOVE, arg_base, dest + 1));
        self.current_insts.push(encode_r2(Opcode::MOVE, arg_base + 1, dest + 2));
        self.emit_call_op(Self::code_target(error_fn, span)?, error, arg_base);
        self.emit_new_object(result, err_type, span);
        self.current_insts.push(encode_r3(Opcode::SETFIELD, result, 0, error));
        self.reg_alloc.free_block(error, 3);
        let end_at = self.current_insts.len() as i32;
        self.current_insts[to_end] = encode_ju(Opcode::JMP, end_at - to_end as i32);

        self.reg_alloc.free_block(dest, window + 1);
        Ok(result)
    }

    pub(crate) fn call_frame(&mut self, target: CallTarget, n: usize) -> (u8, Option<u8>) {
        match target {
            CallTarget::Code(_) => {
                let dest = self.reg_alloc.alloc_block(n + 1);
                (dest.wrapping_add(1), Some(dest))
            }
            CallTarget::Value(_) => (self.reg_alloc.alloc_block(n), None),
        }
    }

    pub(crate) fn code_target(idx: usize, span: Span) -> Result<CallTarget, (String, Span)> {
        u16::try_from(idx)
            .map(CallTarget::Code)
            .map_err(|_| ("too many code objects: a program holds at most 65,536".into(), span))
    }

    pub(crate) fn emit_load_null(&mut self, reg: u8) {
        let const_idx = self.add_constant(Value::null());
        self.current_insts.push(encode_ri(Opcode::LOADK, reg, const_idx));
    }

    pub(crate) fn emit_call_op(&mut self, target: CallTarget, dest: u8, arg_base: u8) {
        self.current_insts.push(match target {
            CallTarget::Code(idx) => encode_call(dest, idx),
            CallTarget::Value(reg) => encode_r3(Opcode::CALLV, dest, reg, arg_base),
        });
    }

    fn emit_call(
        &mut self,
        target: CallTarget,
        args: &[Expr],
        callee: Option<&str>,
        span: Span,
    ) -> Result<u8, (String, Span)> {
        if let Some(&fixed) = callee.and_then(|c| self.variadic_fns.get(c)) {
            self.recording_calls.insert(span);
            return self.emit_variadic_call(target, args, fixed, callee, span);
        }
        let given = args.len();
        let sig = callee.and_then(|c| self.abi.get(c)).filter(|_| matches!(target, CallTarget::Code(_))).cloned();
        let user = match &sig {
            Some(s) => self.abi_width(s, given),
            None => callee.map_or(given, |c| self.call_arity(c, given)),
        };
        let n = user + self.hidden_type_args(callee);
        if matches!(target, CallTarget::Code(_)) {
            self.recording_calls.insert(span);
        }
        let var_args = std::mem::take(&mut self.value_call_vars);
        let ret = sig.as_ref().and_then(|s| s.ret.clone());
        let block = ret.as_ref().map_or(0, |ty| (n + 1).max(self.window_types[ty].1));
        let (arg_base, dest) = match &ret {
            Some(_) => {
                let d = self.reg_alloc.alloc_block(block);
                (d.wrapping_add(1), Some(d))
            }
            None => self.call_frame(target, n),
        };
        if self.reg_alloc.overflowed() {
            return Err(("call expression needs more than 256 registers".into(), span));
        }
        let user_code = matches!(target, CallTarget::Code(_));
        let mut at = 0usize;
        for (i, arg) in args.iter().enumerate() {
            match sig.as_ref().and_then(|s| s.params.get(i).cloned().flatten()) {
                Some(ty) => {
                    self.emit_window_arg(arg, &ty, arg_base + at as u8)?;
                    at += self.window_types[&ty].1;
                }
                None => {
                    let r = self.compile_arg(arg, user_code || var_args.get(i) == Some(&true))?;
                    let slot = arg_base + at as u8;
                    if r != slot {
                        self.current_insts.push(encode_r2(Opcode::MOVE, slot, r));
                    }
                    self.reg_alloc.free_temp(r);
                    at += 1;
                }
            }
        }
        if let Some(c) = callee {
            self.emit_default_args(c, given, arg_base).map_err(|(m, _)| (m, span))?;
        }
        self.emit_type_args(callee, arg_base + user as u8, span);
        let dest = dest.unwrap_or_else(|| self.reg_alloc.alloc_temp());
        self.emit_call_op(target, dest, arg_base);
        if let Some(ty) = &ret {
            return Ok(self.finish_wr_call(ty, dest, block, span));
        }
        self.reg_alloc.free_block(arg_base, n);
        self.store_back(callee, args, dest, span)?;
        Ok(dest)
    }

    /// Builds the `mote test` harness over `test_marks` instead of calling `main`.
    pub fn with_test_marks(mut self, test_marks: Vec<TestMark>) -> Self {
        self.test_marks = test_marks;
        self.test_mode = true;
        self
    }

    /// `mote test --filter`: the harness runs a test only when `fn_name(pattern, test_name)` is true.
    pub fn with_test_filter(mut self, fn_name: String, pattern: String) -> Self {
        self.test_filter = Some((fn_name, pattern));
        self
    }

    /// How a function with return type `ty` reports a child task's unobserved fault when a `scope` closes.
    fn scope_fault_of(ty: Option<&TypeNode>) -> ScopeFault {
        if let Some(TypeNode::Named(name, _)) = ty {
            return if display_type_name(name) == "Result" { ScopeFault::ErrText } else { ScopeFault::Panic };
        }
        let Some(TypeNode::Generic(name, args, _)) = ty else { return ScopeFault::Panic };
        if display_type_name(name) != "Result" || args.len() != 2 {
            return ScopeFault::Panic;
        }
        match &args[1] {
            TypeNode::Named(e, _) => match display_type_name(e) {
                "String" => ScopeFault::ErrText,
                "Error" => ScopeFault::ErrError,
                _ => ScopeFault::Panic,
            },
            _ => ScopeFault::Panic,
        }
    }

    fn is_fallible_type_node(ty: &TypeNode) -> bool {
        let name = match ty {
            TypeNode::Named(name, _) => name,
            TypeNode::Generic(name, _, _) => name,
            _ => return false,
        };
        let bare = display_type_name(name);
        bare == "Option" || bare == "Result"
    }

    fn static_class_name(&self, object: &Expr) -> Option<&str> {
        match self.types.of(object) {
            Some(Type::Struct { name, .. }) | Some(Type::Class { name, .. }) => Some(name.as_str()),
            _ => None,
        }
    }

    fn method_owner(&self, object: &Expr) -> Option<&str> {
        match self.types.of(object) {
            Some(Type::Struct { name, .. } | Type::Class { name, .. } | Type::Enum { name, .. }) => Some(name.as_str()),
            _ => None,
        }
    }

    fn field_index(&self, object: &Expr, member: &str) -> Option<u8> {
        let ty_name = self.static_class_name(object)?;
        let type_idx = *self.type_map.get(ty_name)? as usize;
        let td = self.type_descriptors.get(type_idx)?;
        td.fields
            .iter()
            .find(|f| f.name.as_deref() == Some(member))
            .map(|f| f.slot as u8)
    }

    fn emit_named_field(&mut self, obj_reg: u8, member: &str, val_reg: Option<u8>) -> u8 {
        let name_idx = self.add_string(member.to_string());
        let name_reg = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::NEWSTR, name_reg, name_idx));
        let module_idx = self.add_string(self.module.clone());
        let module_reg = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::NEWSTR, module_reg, module_idx));
        let dest = match val_reg {
            Some(v) => self.emit_native_regs("__field_set", &[obj_reg, name_reg, v, module_reg]),
            None => self.emit_native_regs("__field_get", &[obj_reg, name_reg, module_reg]),
        };
        self.reg_alloc.free_temp(module_reg);
        self.reg_alloc.free_temp(name_reg);
        dest
    }

    fn patch_jumps(&mut self, sites: &[usize], target: i32) {
        for &site in sites {
            let offset = target - site as i32;
            self.current_insts[site] = encode_ju(Opcode::JMP, offset);
        }
    }

    /// Codegen for a checked single-file program (one module): see [`Self::compile`].
    pub(crate) fn compile_single(mut self, mut checked: crate::stage::Checked) -> Result<CompiledProgram, (String, Span)> {
        self.types = std::mem::take(&mut checked.types);
        let program = checked.modules.pop().expect("a checked single-file program has one module");
        self.compile(&program)
    }

    /// Codegen for checked linked modules in dependency order: see [`Self::compile_modules`].
    pub fn compile_linked(mut self, mut checked: crate::stage::Checked) -> Result<CompiledProgram, (String, Span)> {
        self.types = std::mem::take(&mut checked.types);
        let modules = checked
            .modules
            .into_iter()
            .map(|p| p.items.into_iter().filter(|it| !matches!(it, Item::Import(_))).collect())
            .collect();
        self.compile_modules(modules)
    }

    /// Whole-program entry point: every top-level statement lands in `code_objects[0]`.
    pub fn compile(mut self, program: &Program) -> Result<CompiledProgram, (String, Span)> {
        self.collect_variadic_fns(&program.items);
        let mut items = self.add_default_functions(program.items.clone());
        items.push(crate::prelude::stream_of_item());
        self.register_types(&items);
        self.layout_inline_fields(&[&items]);
        self.reserve_function_indices(&items);
        self.layout_globals(&items);
        self.summarize_regions(&[&items]);
        self.plan_window_abi(&[&items]);
        self.next_code_idx = 1 + self.func_map.len();

        self.compile_entry_script(&items, &[], program.span)?;
        self.compile_named_functions(&items)?;
        self.drain_pending_lambdas()?;

        let sources = program::collect_sources(&mut self.code_objects);
        Ok(CompiledProgram {
            code_objects: self.code_objects,
            type_descriptors: self.type_descriptors,
            global_count: self.global_map.len() as u32,
            sources,
            native_table: self.native_table,
            memory_sites: self.memory_sites,
        })
    }

    /// Multi-file entry point: `modules` in dependency order, entry module last. Each non-entry module with top-level statements gets an init code object that `code_objects[0]` calls first.
    pub(crate) fn compile_modules(
        mut self,
        mut modules: Vec<Vec<Item>>,
    ) -> Result<CompiledProgram, (String, Span)> {
        for m in &modules {
            self.collect_variadic_fns(m);
        }
        modules = modules.into_iter().map(|m| self.add_default_functions(m)).collect();
        if let Some(first) = modules.first_mut() {
            first.push(crate::prelude::stream_of_item());
        }
        for (i, m) in modules.iter().enumerate() {
            self.enter_module(i);
            self.register_types(m);
        }
        let all_items: Vec<&[Item]> = modules.iter().map(|m| m.as_slice()).collect();
        self.layout_inline_fields(&all_items);
        for m in &modules {
            self.reserve_function_indices(m);
        }
        for m in &modules {
            self.layout_globals(m);
        }
        let all: Vec<&[Item]> = modules.iter().map(|m| m.as_slice()).collect();
        self.summarize_regions(&all);
        self.plan_window_abi(&all);

        let (entry_items, dep_modules) = match modules.split_last() {
            Some((last, rest)) => (last.as_slice(), rest),
            None => (&[][..], &[][..]),
        };

        let named = self.func_map.len();
        let init_module_idx: Vec<usize> = dep_modules
            .iter()
            .enumerate()
            .filter(|(_, m)| m.iter().any(|it| matches!(it, Item::TopLevelStmt(_))))
            .map(|(i, _)| i)
            .collect();
        let init_indices: Vec<usize> =
            (0..init_module_idx.len()).map(|k| 1 + named + k).collect();
        if 1 + named + init_indices.len() > u16::MAX as usize + 1 {
            return Err((
                "too many code objects: a program holds at most 65,536".into(),
                entry_items.first().map(|it| it.span()).unwrap_or(Span::new(0, 0, 1, 1)),
            ));
        }
        self.next_code_idx = 1 + named + init_indices.len();

        let entry_span = entry_items
            .first()
            .map(|it| it.span())
            .unwrap_or(Span::new(0, 0, 1, 1));
        self.enter_module(modules.len().saturating_sub(1));
        self.compile_entry_script(entry_items, &init_indices, entry_span)?;

        for (i, m) in modules.iter().enumerate() {
            self.enter_module(i);
            self.compile_named_functions(m)?;
        }
        for (&mi, &code_idx) in init_module_idx.iter().zip(&init_indices) {
            debug_assert_eq!(self.code_objects.len(), code_idx);
            self.enter_module(mi);
            self.compile_init_function(&dep_modules[mi])?;
        }
        self.drain_pending_lambdas()?;

        let sources = program::collect_sources(&mut self.code_objects);
        Ok(CompiledProgram {
            code_objects: self.code_objects,
            type_descriptors: self.type_descriptors,
            global_count: self.global_map.len() as u32,
            sources,
            native_table: self.native_table,
            memory_sites: self.memory_sites,
        })
    }

    fn note_field_privacy(&self, desc: &mut TypeDescriptor, fields: &[FieldDecl]) {
        desc.module = Some(self.module.clone());
        for (slot, f) in desc.fields.iter_mut().zip(fields) {
            slot.is_pub = f.is_pub;
        }
    }

    fn register_types(&mut self, items: &[Item]) {
        for item in items {
            match item {
                Item::Struct(s) => {
                    let type_idx = self.type_descriptors.len() as u16;
                    let fields: Vec<Option<String>> =
                        s.fields.iter().map(|f| Some(f.name.clone())).collect();
                    let mut type_desc = TypeDescriptor::new(type_idx as u64, fields);
                    type_desc.is_value_type = true;
                    type_desc.is_trivial = true;
                    type_desc.name = Some(display_type_name(&s.name).to_string());
                    self.note_field_privacy(&mut type_desc, &s.fields);
                    self.type_descriptors.push(type_desc);
                    self.type_map.insert(s.name.clone(), type_idx);
                }
                Item::Class(c) => {
                    let type_idx = self.type_descriptors.len() as u16;
                    let fields: Vec<Option<String>> =
                        c.fields.iter().map(|f| Some(f.name.clone())).collect();
                    let mut type_desc = TypeDescriptor::new(type_idx as u64, fields);
                    type_desc.name = Some(display_type_name(&c.name).to_string());
                    self.note_field_privacy(&mut type_desc, &c.fields);
                    self.type_descriptors.push(type_desc);
                    self.type_map.insert(c.name.clone(), type_idx);
                    self.class_names.insert(c.name.clone());
                }
                Item::Enum(e) => {
                    for v in &e.variants {
                        let type_idx = self.type_descriptors.len() as u16;
                        let fields: Vec<Option<String>> = match &v.kind {
                            EnumVariantKind::Unit { .. } => Vec::new(),
                            EnumVariantKind::Tuple(tys) => tys.iter().map(|_| None).collect(),
                            EnumVariantKind::Struct(fs) => {
                                fs.iter().map(|f| Some(f.name.clone())).collect()
                            }
                        };
                        let info = VariantInfo {
                            type_idx,
                            arity: fields.len(),
                            enum_name: e.name.clone(),
                        };
                        let mut vd = TypeDescriptor::new(type_idx as u64, fields);
                        vd.name = Some(v.name.clone());
                        vd.by_content = true;
                        self.type_descriptors.push(vd);
                        self.enum_variant_map
                            .insert((e.name.clone(), v.name.clone()), info.clone());
                        match self.variant_map.entry(v.name.clone()) {
                            std::collections::hash_map::Entry::Vacant(slot) => {
                                slot.insert(info);
                            }
                            std::collections::hash_map::Entry::Occupied(mut existing) => {
                                let owner = &existing.get().enum_name;
                                if *owner != e.name && !existing.get().is_option() && display_type_name(owner) != "Result" {
                                    if display_type_name(&e.name) == "Result" {
                                        existing.insert(info);
                                    } else {
                                        existing.remove();
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn reserve_function_indices(&mut self, items: &[Item]) {
        for item in items {
            match item {
                Item::Function(f) => {
                    let func_idx = self.func_map.len() + 1;
                    self.func_map.insert(f.name.clone(), func_idx);
                    self.note_hidden_params(&f.name, f, &[]);
                    self.note_writeback(&f.name, f);
                }
                Item::Class(c) => {
                    self.class_type_params.insert(c.name.clone(), c.generic_params.iter().map(|p| p.name.clone()).collect());
                    self.reserve_methods(&c.name, &c.methods);
                }
                Item::Struct(st) => {
                    self.class_type_params.insert(st.name.clone(), st.generic_params.iter().map(|p| p.name.clone()).collect());
                    self.reserve_methods(&st.name, &st.methods);
                }
                Item::Enum(e) => {
                    self.class_type_params.insert(e.name.clone(), e.generic_params.iter().map(|p| p.name.clone()).collect());
                    self.reserve_methods(&e.name, &e.methods);
                }
                _ => {}
            }
        }
    }

    fn reserve_methods(&mut self, type_name: &str, methods: &[FunctionDecl]) {
        for m in methods {
            let func_idx = self.func_map.len() + 1;
            let key = Self::mangle_method_name(type_name, &m.name);
            self.func_map.insert(key.clone(), func_idx);
            let class_params = self.class_type_params.get(type_name).cloned().unwrap_or_default();
            self.note_hidden_params(&key, m, &class_params);
            self.note_writeback(&key, m);
            if matches!(m.params.first().and_then(|p| p.kind.as_ref()), Some(ParameterKind::SelfValue { is_var: true })) {
                self.var_self_methods.insert(key);
            }
        }
    }

    pub(crate) fn mangle_method_name(class_name: &str, method_name: &str) -> String {
        format!("{class_name}::{method_name}")
    }

    fn layout_globals(&mut self, items: &[Item]) {
        for item in items {
            if let Item::TopLevelStmt(Stmt::Let { name, .. } | Stmt::Var { name, .. }) = item
                && !self.global_map.contains_key(name) {
                    let idx = self.global_map.len() as u16;
                    self.global_map.insert(name.clone(), idx);
                }
        }
    }

    fn compile_named_functions(&mut self, items: &[Item]) -> Result<(), (String, Span)> {
        for item in items {
            match item {
                Item::Function(f) => self.compile_function(f)?,
                Item::Class(c) => {
                    for m in &c.methods {
                        self.compile_class_method(&c.name, m)?;
                    }
                }
                Item::Struct(st) => {
                    for m in &st.methods {
                        self.compile_class_method(&st.name, m)?;
                    }
                }
                Item::Enum(e) => {
                    for m in &e.methods {
                        self.compile_class_method(&e.name, m)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn drain_pending_lambdas(&mut self) -> Result<(), (String, Span)> {
        while let Some(pl) = self.pending_lambdas.pop_front() {
            self.module = pl.module.clone();
            self.compile_lambda(pl)?;
        }
        Ok(())
    }

    fn compile_top_level_stmt(&mut self, stmt: &Stmt) -> Result<bool, (String, Span)> {
        match stmt {
            Stmt::Let { name, init, .. } | Stmt::Var { name, init, .. }
                if self.global_map.contains_key(name) =>
            {
                self.compile_toplevel_global(name, init)?;
                Ok(false)
            }
            _ => {
                self.compile_stmt(stmt)?;
                Ok(matches!(stmt, Stmt::Return { .. }))
            }
        }
    }

    fn compile_entry_script(
        &mut self,
        items: &[Item],
        init_indices: &[usize],
        span: Span,
    ) -> Result<(), (String, Span)> {
        self.current_insts.clear();
        self.current_constants.clear();
        self.current_strings.clear();
        self.current_spans.clear();
        self.reg_alloc = RegisterAllocator::new();
        self.type_scope.clear();
        self.cell_names.clear();
        self.fn_writeback.clear();
        self.ret_param = None;
        self.loop_stack.clear();
        self.regions.clear();
        self.region_heap.clear();
        self.region_stack.clear();
        self.with_stack.clear();

        for &idx in init_indices {
            let dest = self.reg_alloc.alloc_temp();
            let target = Self::code_target(idx, span)?;
            self.emit_call_op(target, dest, dest.wrapping_add(1));
            self.reg_alloc.free_temp(dest);
        }

        if self.test_mode {
            self.emit_test_harness(span)?;
            self.current_insts.push(encode_none(Opcode::HALT));
            return self.finish_entry_script(span);
        }

        let mut has_explicit_halt = false;
        for item in items {
            if let Item::TopLevelStmt(stmt) = item
                && self.compile_top_level_stmt(stmt)? {
                    has_explicit_halt = true;
                }
        }

        if !has_explicit_halt {
            let has_effectful_top_level = items.iter().any(|it| match it {
                Item::TopLevelStmt(Stmt::Let { name, .. } | Stmt::Var { name, .. })
                    if self.global_map.contains_key(name) =>
                {
                    false
                }
                Item::TopLevelStmt(_) => true,
                _ => false,
            });

            if let Some(&main_idx) = self.func_map.get("main") {
                if has_effectful_top_level {
                    let dest = self.reg_alloc.alloc_temp();
                    let target = Self::code_target(main_idx, span)?;
                    self.emit_call_op(target, dest, dest.wrapping_add(1));
                    self.reg_alloc.free_temp(dest);
                } else {
                    let target = Self::code_target(main_idx, span)?;
                    self.emit_call_op(target, 0, 1);
                }
            } else if !self.global_map.is_empty() {
                self.current_insts.push(encode_ri(Opcode::GETGLOBAL, 0, 0));
            }
            self.current_insts.push(encode_none(Opcode::HALT));
        }

        self.finish_entry_script(span)
    }

    fn finish_entry_script(&mut self, span: Span) -> Result<(), (String, Span)> {
        if self.reg_alloc.overflowed() {
            return Err((
                "top-level code is too complex: it needs more than 256 registers".into(),
                span,
            ));
        }

        let reg_count = self.reg_alloc.total_registers();
        let main_code = CodeObject::new(
            std::mem::take(&mut self.current_insts),
            std::mem::take(&mut self.current_constants),
            reg_count,
            0,
        )
        .with_string_table(std::mem::take(&mut self.current_strings))
        .with_spans(std::mem::take(&mut self.current_spans));
        self.code_objects.push(main_code);
        Ok(())
    }

    fn compile_init_function(&mut self, items: &[Item]) -> Result<(), (String, Span)> {
        self.current_insts.clear();
        self.current_constants.clear();
        self.current_strings.clear();
        self.current_spans.clear();
        self.reg_alloc = RegisterAllocator::new();
        self.type_scope.clear();
        self.cell_names.clear();
        self.fn_writeback.clear();
        self.ret_param = None;
        self.loop_stack.clear();
        self.regions.clear();
        self.region_heap.clear();
        self.region_stack.clear();
        self.with_stack.clear();

        let mut span = Span::new(0, 0, 1, 1);
        for item in items {
            if let Item::TopLevelStmt(stmt) = item {
                span = stmt.span();
                self.compile_top_level_stmt(stmt)?;
            }
        }

        if self.current_insts.last().map(|i| (i & 0xFF) as u8) != Some(Opcode::RET as u8) {
            let ret_reg = self.reg_alloc.alloc_temp();
            self.current_insts.push(encode_ri(Opcode::LOADI, ret_reg, 0));
            self.current_insts.push(encode_r2(Opcode::RET, ret_reg, 0));
        }

        if self.reg_alloc.overflowed() {
            return Err((
                "a module's top-level code is too complex: it needs more than 256 registers"
                    .into(),
                span,
            ));
        }

        let reg_count = self.reg_alloc.total_registers();
        let code = CodeObject::new(
            std::mem::take(&mut self.current_insts),
            std::mem::take(&mut self.current_constants),
            reg_count,
            0,
        )
        .with_string_table(std::mem::take(&mut self.current_strings))
        .with_spans(std::mem::take(&mut self.current_spans));
        self.code_objects.push(code);
        Ok(())
    }

    fn compile_toplevel_global(&mut self, name: &str, init: &Expr) -> Result<(), (String, Span)> {
        let gidx = self.global_map[name];
        let r = self.compile_value(init)?;
        self.current_insts.push(encode_ri(Opcode::SETGLOBAL, r, gidx));
        self.reg_alloc.free_temp(r);
        Ok(())
    }

    fn compile_function(&mut self, f: &FunctionDecl) -> Result<(), (String, Span)> {
        let Some(table) = self.types.take_instance(&f.name) else { return self.compile_function_body(f) };
        let outer = std::mem::replace(&mut self.types, table);
        let result = self.compile_function_body(f);
        self.types = outer;
        result
    }

    fn analyze_regions(&mut self, body: &[Stmt], params: &[crate::regions::ParamInfo]) {
        let env = crate::regions::Env { value_structs: &self.value_structs, sigs: &self.region_sigs };
        let analysis = crate::regions::analyze(body, params, &env);
        self.regions = analysis.placed;
        self.region_heap = analysis.heap;
        self.region_vars = analysis.placed_names;
        self.safe_uses = analysis.safe_uses;
        self.param_names = params.iter().map(|p| p.name.clone()).collect();
        let names: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
        self.plan_windows(body, &names);
    }

    fn summarize_regions(&mut self, modules: &[&[Item]]) {
        let mut fns: Vec<(String, &FunctionDecl, Option<String>)> = Vec::new();
        for item in modules.iter().flat_map(|m| m.iter()) {
            match item {
                Item::Struct(s) => {
                    self.value_structs.insert(s.name.clone());
                    fns.extend(s.methods.iter().filter(|m| crate::regions::summarizable(m, !s.generic_params.is_empty())).map(|m| (Self::mangle_method_name(&s.name, &m.name), m, Some(s.name.clone()))));
                }
                Item::Class(c) => {
                    fns.extend(c.methods.iter().filter(|m| crate::regions::summarizable(m, !c.generic_params.is_empty())).map(|m| (Self::mangle_method_name(&c.name, &m.name), m, Some(c.name.clone()))));
                }
                Item::Enum(e) => {
                    fns.extend(e.methods.iter().filter(|m| crate::regions::summarizable(m, !e.generic_params.is_empty())).map(|m| (Self::mangle_method_name(&e.name, &m.name), m, Some(e.name.clone()))));
                }
                Item::Function(f) if crate::regions::summarizable(f, false) => fns.push((f.name.clone(), f, None)),
                _ => {}
            }
        }
        self.region_sigs = crate::regions::summarize(&fns, &self.value_structs);
    }

    fn compile_function_body(&mut self, f: &FunctionDecl) -> Result<(), (String, Span)> {
        self.current_insts.clear();
        self.current_constants.clear();
        self.current_strings.clear();
        self.current_spans.clear();
        self.current_sig = self.abi.get(&f.name).cloned();
        self.current_owner = None;
        let param_names = match &self.current_sig {
            Some(sig) => self.abi_params(f, sig),
            None => Self::physical_params(f, &[]),
        };
        self.reg_alloc = RegisterAllocator::with_params(&param_names);
        self.enter_type_scope(f, &[]);
        self.loop_stack.clear();
        self.analyze_regions(&f.body, &crate::regions::param_infos(f, None));
        self.bind_window_params(f);
        self.cell_names = self.cell_names(&f.body);
        self.enter_writeback(f)?;
        self.region_stack.clear();
        self.with_stack.clear();
        self.current_fn_fallible = f.return_type.as_ref().is_some_and(Self::is_fallible_type_node);
        self.current_scope_fault = Self::scope_fault_of(f.return_type.as_ref());

        if stmts_yield(&f.body) {
            self.current_insts.push(encode_none(Opcode::MKGEN));
        }
        for stmt in &f.body {
            self.compile_stmt(stmt)?;
        }

        let last = self.current_insts.last().map(|i| (i & 0xFF) as u8);
        if last != Some(Opcode::RET as u8) && last != Some(Opcode::RETN as u8) {
            let ret_reg = self.reg_alloc.alloc_temp();
            self.exit_regions_above(0);
            self.emit_load_null(ret_reg);
            self.emit_ret(ret_reg);
        }

        if self.reg_alloc.overflowed() {
            return Err((
                format!("function `{}` is too complex: it needs more than 256 registers", f.name),
                f.span,
            ));
        }

        self.current_sig = None;
        self.current_owner = None;
        let reg_count = self.reg_alloc.total_registers();
        let func_code = CodeObject::new(
            std::mem::take(&mut self.current_insts),
            std::mem::take(&mut self.current_constants),
            reg_count,
            param_names.len() as u8,
        )
        .with_string_table(std::mem::take(&mut self.current_strings))
        .with_spans(std::mem::take(&mut self.current_spans));
        self.code_objects.push(func_code);
        Ok(())
    }

    fn compile_class_method(&mut self, class_name: &str, m: &FunctionDecl) -> Result<(), (String, Span)> {
        self.current_insts.clear();
        self.current_constants.clear();
        self.current_strings.clear();
        self.current_spans.clear();
        let class_params = self.class_type_params.get(class_name).cloned().unwrap_or_default();
        self.current_sig = self.abi.get(&Self::mangle_method_name(class_name, &m.name)).cloned();
        self.current_owner = Some(class_name.to_string());
        let param_names = match &self.current_sig {
            Some(sig) => self.abi_params(m, sig),
            None => Self::physical_params(m, &class_params),
        };
        self.reg_alloc = RegisterAllocator::with_params(&param_names);
        self.enter_type_scope(m, &class_params);
        self.loop_stack.clear();
        self.analyze_regions(&m.body, &crate::regions::param_infos(m, Some(class_name)));
        self.bind_window_params(m);
        self.cell_names = self.cell_names(&m.body);
        self.enter_writeback(m)?;
        self.region_stack.clear();
        self.with_stack.clear();
        self.current_fn_fallible = m.return_type.as_ref().is_some_and(Self::is_fallible_type_node);
        self.current_scope_fault = Self::scope_fault_of(m.return_type.as_ref());

        if stmts_yield(&m.body) {
            self.current_insts.push(encode_none(Opcode::MKGEN));
        }
        for stmt in &m.body {
            self.compile_stmt(stmt)?;
        }

        let last = self.current_insts.last().map(|i| (i & 0xFF) as u8);
        if last != Some(Opcode::RET as u8) && last != Some(Opcode::RETN as u8) {
            let ret_reg = self.reg_alloc.alloc_temp();
            self.exit_regions_above(0);
            self.emit_load_null(ret_reg);
            self.emit_ret(ret_reg);
        }

        if self.reg_alloc.overflowed() {
            return Err((
                format!(
                    "method `{}` is too complex: it needs more than 256 registers",
                    Self::mangle_method_name(class_name, &m.name)
                ),
                m.span,
            ));
        }

        self.current_sig = None;
        self.current_owner = None;
        let reg_count = self.reg_alloc.total_registers();
        let func_code = CodeObject::new(
            std::mem::take(&mut self.current_insts),
            std::mem::take(&mut self.current_constants),
            reg_count,
            param_names.len() as u8,
        )
        .with_string_table(std::mem::take(&mut self.current_strings))
        .with_spans(std::mem::take(&mut self.current_spans));
        self.code_objects.push(func_code);
        Ok(())
    }

    fn compile_lambda(&mut self, pl: PendingLambda) -> Result<(), (String, Span)> {
        debug_assert_eq!(
            self.code_objects.len(),
            pl.code_idx,
            "lambda code objects must be appended in reserved-index order"
        );
        if let Some(code) = pl.prebuilt {
            self.code_objects.push(code);
            return Ok(());
        }
        self.current_insts.clear();
        self.current_constants.clear();
        self.current_strings.clear();
        self.current_spans.clear();
        self.reg_alloc = RegisterAllocator::with_params(&pl.params);
        self.loop_stack.clear();
        let visible: Vec<crate::regions::ParamInfo> =
            pl.params.iter().chain(&pl.captures).map(|name| crate::regions::ParamInfo { name: name.clone(), ty: None }).collect();
        self.analyze_regions(&pl.body, &visible);
        self.region_stack.clear();
        self.with_stack.clear();
        self.current_fn_fallible = pl.return_type.as_ref().is_some_and(Self::is_fallible_type_node);
        self.current_scope_fault = Self::scope_fault_of(pl.return_type.as_ref());

        self.cell_names = self.cell_names(&pl.body);
        self.fn_writeback.clear();
        self.ret_param = pl.params.first().filter(|_| pl.returns_param).cloned();
        for (i, name) in pl.captures.iter().enumerate() {
            let reg = self.reg_alloc.alloc_var(name);
            self.current_insts
                .push(encode_ri(Opcode::GETCAPTURE, reg, i as u16));
            if pl.cell_captures[i] {
                self.reg_alloc.mark_cell(name);
            } else if self.cell_names.contains(name) {
                let cell = self.emit_new_cell(reg);
                self.current_insts.push(encode_r2(Opcode::MOVE, reg, cell));
                self.reg_alloc.free_temp(cell);
                self.reg_alloc.mark_cell(name);
            }
        }
        self.bind_captured_type_scope(&pl.captures);

        let mut stmts = pl.body;
        let tail_expr = match stmts.last() {
            Some(Stmt::Expr { .. }) => match stmts.pop() {
                Some(Stmt::Expr { expr, .. }) => Some(expr),
                _ => unreachable!(),
            },
            _ => None,
        };
        for stmt in &stmts {
            self.compile_stmt(stmt)?;
        }
        match tail_expr {
            Some(expr) if self.ret_param.is_some() => {
                let r = self.compile_expr(&expr)?;
                self.exit_regions_above(0);
                self.emit_ret(r);
                self.reg_alloc.free_temp(r);
            }
            Some(expr) => {
                let r = self.compile_returned(&expr)?;
                self.exit_regions_above(0);
                self.current_insts.push(encode_r2(Opcode::RET, r, 0));
                self.reg_alloc.free_temp(r);
            }
            None if self.ret_param.is_some() => {
                let r = self.reg_alloc.alloc_temp();
                self.exit_regions_above(0);
                self.emit_ret(r);
                self.reg_alloc.free_temp(r);
            }
            None => {
                if self.current_insts.last().map(|i| (i & 0xFF) as u8) != Some(Opcode::RET as u8) {
                    let ret_reg = self.reg_alloc.alloc_temp();
                    self.emit_load_null(ret_reg);
                    self.exit_regions_above(0);
                    self.current_insts.push(encode_r2(Opcode::RET, ret_reg, 0));
                }
            }
        }

        if self.reg_alloc.overflowed() {
            return Err((
                "a lambda is too complex: it needs more than 256 registers".into(),
                pl.span,
            ));
        }

        let reg_count = self.reg_alloc.total_registers();
        let code = CodeObject::new(
            std::mem::take(&mut self.current_insts),
            std::mem::take(&mut self.current_constants),
            reg_count,
            pl.params.len() as u8,
        )
        .with_string_table(std::mem::take(&mut self.current_strings))
        .with_spans(std::mem::take(&mut self.current_spans));
        self.code_objects.push(code);
        Ok(())
    }

    fn qualified_variant(&self, e: &Expr) -> Option<VariantInfo> {
        let (head, member) = match e {
            Expr::MemberAccess { object, member, .. } => match object.as_ref() {
                Expr::Ident(n, _) => (n.clone(), member.clone()),
                _ => return None,
            },
            Expr::StaticAccess { target: TypeNode::Named(n, _), member, .. } => {
                (n.clone(), member.clone())
            }
            _ => return None,
        };
        self.enum_variant_map.get(&(head, member)).cloned()
    }

    fn emit_call_value(&mut self, callee: u8, arg: Option<u8>) -> u8 {
        let n = if arg.is_some() { 1 } else { 0 };
        let arg_base = self.reg_alloc.alloc_block(n);
        if let Some(a) = arg
            && a != arg_base {
                self.current_insts.push(encode_r2(Opcode::MOVE, arg_base, a));
            }
        let dest = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(Opcode::CALLV, dest, callee, arg_base));
        self.reg_alloc.free_block(arg_base, n);
        dest
    }

    fn success_variant_type_idx(&self, object: &Expr) -> Option<u16> {
        let Type::Enum { variants, .. } = self.types.of(object)? else { return None };
        let (succ_name, _) = variants.iter().find(|(v, _)| v != "None" && v != "Err")?;
        self.variant_map.get(succ_name).map(|v| v.type_idx)
    }

    fn try_lower_class_method_call(
        &mut self,
        object: &Expr,
        method: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<u8>, (String, Span)> {
        let Some(class_name) = self.method_owner(object).map(|s| s.to_string()) else {
            return Ok(None);
        };
        self.lower_class_method_call(class_name, object, method, args, span)
    }

    pub(crate) fn lower_class_method_call(
        &mut self,
        class_name: String,
        object: &Expr,
        method: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<u8>, (String, Span)> {
        let Some(&func_idx) = self.func_map.get(&Self::mangle_method_name(&class_name, method)) else {
            return Ok(None);
        };
        let key = Self::mangle_method_name(&class_name, method);
        let hidden = self.hidden_type_args(Some(&key));
        if let Some(&fixed) = self.variadic_fns.get(&key) {
            let n = 1 + fixed + 1 + hidden;
            let call = Self::code_target(func_idx, span)?;
            let (arg_base, dest) = self.call_frame(call, n);
            if self.reg_alloc.overflowed() {
                return Err(("method call needs more than 256 registers".into(), span));
            }
            let recv = self.compile_expr(object)?;
            if recv != arg_base {
                self.current_insts.push(encode_r2(Opcode::MOVE, arg_base, recv));
            }
            self.reg_alloc.free_temp(recv);
            for (i, arg) in args.iter().take(fixed).enumerate() {
                let r = self.compile_expr(arg)?;
                let target = arg_base + 1 + i as u8;
                if r != target {
                    self.current_insts.push(encode_r2(Opcode::MOVE, target, r));
                }
                self.reg_alloc.free_temp(r);
            }
            let list = self.pack_variadic_tail(&args[fixed.min(args.len())..], span)?;
            let target = arg_base + 1 + fixed as u8;
            if list != target {
                self.current_insts.push(encode_r2(Opcode::MOVE, target, list));
            }
            self.reg_alloc.free_temp(list);
            self.emit_type_args(Some(&key), target + 1, span);
            let dest = dest.unwrap_or_else(|| self.reg_alloc.alloc_temp());
            self.emit_call_op(call, dest, arg_base);
            self.reg_alloc.free_block(arg_base, n);
            self.store_back(Some(&key), args, dest, span)?;
            return Ok(Some(dest));
        }
        let sig = self.abi.get(&key).cloned();
        let user = match &sig {
            Some(s) => self.abi_width(s, 1 + args.len()),
            None => 1 + self.call_arity(&key, args.len()),
        };
        let n = user + hidden;
        let call = Self::code_target(func_idx, span)?;
        let ret = sig.as_ref().and_then(|s| s.ret.clone());
        let block = ret.as_ref().map_or(0, |ty| (n + 1).max(self.window_types[ty].1));
        let (arg_base, dest) = match &ret {
            Some(_) => {
                let d = self.reg_alloc.alloc_block(block);
                (d.wrapping_add(1), Some(d))
            }
            None => self.call_frame(call, n),
        };
        if self.reg_alloc.overflowed() {
            return Err(("method call needs more than 256 registers".into(), span));
        }
        let mut at = 0usize;
        for (pos, e) in std::iter::once(object).chain(args.iter()).enumerate() {
            match sig.as_ref().and_then(|s| s.params.get(pos).cloned().flatten()) {
                Some(ty) => {
                    self.emit_window_arg(e, &ty, arg_base + at as u8)?;
                    at += self.window_types[&ty].1;
                }
                None => {
                    let r = self.compile_expr(e)?;
                    let slot = arg_base + at as u8;
                    if r != slot {
                        self.current_insts.push(encode_r2(Opcode::MOVE, slot, r));
                    }
                    self.reg_alloc.free_temp(r);
                    at += 1;
                }
            }
        }
        self.emit_default_args(&key, args.len(), arg_base + 1).map_err(|(m, _)| (m, span))?;
        self.emit_type_args(Some(&key), arg_base + user as u8, span);
        let dest = dest.unwrap_or_else(|| self.reg_alloc.alloc_temp());
        self.emit_call_op(call, dest, arg_base);
        if let Some(ty) = &ret {
            return Ok(Some(self.finish_wr_call(ty, dest, block, span)));
        }
        self.reg_alloc.free_block(arg_base, n);
        self.store_back(Some(&key), args, dest, span)?;
        Ok(Some(dest))
    }

    fn method_variant_test(&mut self, object: &Expr, variant: &str) -> Result<u8, (String, Span)> {
        let idx = self
            .variant_map
            .get(variant)
            .map(|v| v.type_idx)
            .ok_or_else(|| {
                (format!("`.is_*()` needs the `{variant}` variant in scope"), object.span())
            })?;
        let r = self.compile_expr(object)?;
        let dest = self.emit_typeof_eq(r, idx).expect("typeof_eq yields a register");
        self.reg_alloc.free_temp(r);
        Ok(dest)
    }

    fn emit_native_regs(&mut self, native: impl Into<Callee>, arg_regs: &[u8]) -> u8 {
        let callee = native.into();
        let n = arg_regs.len();
        let (arg_base, dest) = self.native_frame(&callee, n);
        for (i, &r) in arg_regs.iter().enumerate() {
            let target = arg_base + i as u8;
            if r != target {
                self.current_insts.push(encode_r2(Opcode::MOVE, target, r));
            }
        }
        self.emit_native_op(&callee, dest, arg_base);
        self.reg_alloc.free_block(arg_base, n);
        dest
    }

    fn emit_len(&mut self, obj_reg: u8) -> u8 {
        self.emit_native_regs("len", &[obj_reg])
    }

    fn emit_panic(&mut self, message: &str) {
        let callee = Callee::Builtin("panic");
        let (arg_base, dest) = self.native_frame(&callee, 1);
        let str_idx = self.add_string(message.to_string());
        self.current_insts.push(encode_ri(Opcode::NEWSTR, arg_base, str_idx));
        self.emit_native_op(&callee, dest, arg_base);
        self.reg_alloc.free_temp(dest);
        self.reg_alloc.free_block(arg_base, 1);
    }

    fn emit_assert_eq(&mut self, args: &[Expr], span: Span) -> Result<u8, (String, Span)> {
        if args.len() != 2 {
            return Err(("`assert_eq` needs exactly two arguments".to_string(), span));
        }
        let a = self.compile_expr(&args[0])?;
        let b = self.compile_expr(&args[1])?;
        let eq = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(Opcode::EQ, eq, a, b));

        let to_end = self.current_insts.len();
        self.current_insts.push(0);

        let sa = self.emit_native_regs("str_from", &[a]);
        let sb = self.emit_native_regs("str_from", &[b]);
        self.reg_alloc.free_temp(a);
        self.reg_alloc.free_temp(b);

        let prefix_idx = self.add_string("assertion failed: ".to_string());
        let prefix = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::NEWSTR, prefix, prefix_idx));
        let m1 = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(Opcode::ADD, m1, prefix, sa));
        self.reg_alloc.free_temp(prefix);
        self.reg_alloc.free_temp(sa);

        let mid_idx = self.add_string(" != ".to_string());
        let mid = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::NEWSTR, mid, mid_idx));
        let m2 = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(Opcode::ADD, m2, m1, mid));
        self.reg_alloc.free_temp(mid);
        self.reg_alloc.free_temp(m1);

        let message = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(Opcode::ADD, message, m2, sb));
        self.reg_alloc.free_temp(m2);
        self.reg_alloc.free_temp(sb);

        let panic_dest = self.emit_native_regs("panic", &[message]);
        self.reg_alloc.free_temp(panic_dest);
        self.reg_alloc.free_temp(message);

        let end_at = self.current_insts.len() as i32;
        self.current_insts[to_end] = encode_jc(Opcode::JMPIF, eq, (end_at - to_end as i32) as i16);
        self.reg_alloc.free_temp(eq);

        let dest = self.reg_alloc.alloc_temp();
        let null_idx = self.add_constant(Value::null());
        self.current_insts.push(encode_ri(Opcode::LOADK, dest, null_idx));
        Ok(dest)
    }

    fn patch_test_skip(&mut self, skip: Option<(usize, u8)>) {
        if let Some((at, hit)) = skip {
            let here = self.current_insts.len() as i32;
            self.current_insts[at] = encode_jc(Opcode::JMPIFNOT, hit, (here - at as i32) as i16);
        }
    }

    fn emit_test_harness(&mut self, span: Span) -> Result<(), (String, Span)> {
        use isa::intrinsics::{STATUS_OK, TASK_JOIN_INTRINSIC, TASK_SLOT_RESULT, TASK_SLOT_STATUS};

        let tests: Vec<(String, usize, bool)> = self
            .test_marks
            .iter()
            .map(|m| {
                let idx = *self.func_map.get(&m.fn_name).unwrap_or_else(|| {
                    panic!("test '{}' resolves to a compiled function ('{}')", m.display_name, m.fn_name)
                });
                (m.display_name.clone(), idx, m.ignored)
            })
            .collect();

        let pass_count = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::LOADI, pass_count, 0));
        let fail_count = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::LOADI, fail_count, 0));
        let ignored_count = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::LOADI, ignored_count, 0));
        let filter = match self.test_filter.clone() {
            Some((fn_name, pattern)) => {
                let idx = *self.func_map.get(&fn_name).expect("the --filter matcher is compiled");
                let f = self.emit_function_value(idx, &[], span)?;
                let p = self.reg_alloc.alloc_temp();
                let p_idx = self.add_string(pattern);
                self.current_insts.push(encode_ri(Opcode::NEWSTR, p, p_idx));
                Some((f, p))
            }
            None => None,
        };

        for (name, code_idx, ignored) in &tests {
            let skip = filter.map(|(f, p)| {
                let base = self.reg_alloc.alloc_block(2);
                self.current_insts.push(encode_r2(Opcode::MOVE, base, p));
                let name_idx = self.add_string(name.clone());
                self.current_insts.push(encode_ri(Opcode::NEWSTR, base + 1, name_idx));
                let hit = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_r3(Opcode::CALLV, hit, f, base));
                self.reg_alloc.free_block(base, 2);
                self.reg_alloc.free_temp(hit);
                self.current_insts.push(0);
                (self.current_insts.len() - 1, hit)
            });
            if *ignored {
                let one = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_ri(Opcode::LOADI, one, 1));
                self.current_insts.push(encode_r3(Opcode::ADD, ignored_count, ignored_count, one));
                self.reg_alloc.free_temp(one);
                let msg_idx = self.add_string(format!("  ignored  {name}"));
                let msg = self.reg_alloc.alloc_temp();
                self.current_insts.push(encode_ri(Opcode::NEWSTR, msg, msg_idx));
                let d = self.emit_native_regs("println", &[msg]);
                self.reg_alloc.free_temp(d);
                self.reg_alloc.free_temp(msg);
                self.patch_test_skip(skip);
                continue;
            }

            let closure = self.emit_function_value(*code_idx, &[], span)?;
            let handle = self.reg_alloc.alloc_temp();
            self.current_insts.push(encode_r2(Opcode::SPAWN, handle, closure));
            self.reg_alloc.free_temp(closure);

            let wait = self.emit_native_regs(TASK_JOIN_INTRINSIC, &[handle]);
            self.reg_alloc.free_temp(wait);

            let status = self.reg_alloc.alloc_temp();
            self.current_insts
                .push(encode_r3(Opcode::GETFIELD, status, handle, TASK_SLOT_STATUS as u8));
            let ok_const = self.reg_alloc.alloc_temp();
            self.current_insts.push(encode_ri(Opcode::LOADI, ok_const, STATUS_OK as u16));
            let is_ok = self.reg_alloc.alloc_temp();
            self.current_insts.push(encode_r3(Opcode::EQ, is_ok, status, ok_const));
            self.reg_alloc.free_temp(ok_const);
            self.reg_alloc.free_temp(status);

            let payload = self.reg_alloc.alloc_temp();
            self.current_insts
                .push(encode_r3(Opcode::GETFIELD, payload, handle, TASK_SLOT_RESULT as u8));
            self.reg_alloc.free_temp(handle);

            let to_fail = self.current_insts.len();
            self.current_insts.push(0);

            let one = self.reg_alloc.alloc_temp();
            self.current_insts.push(encode_ri(Opcode::LOADI, one, 1));
            self.current_insts.push(encode_r3(Opcode::ADD, pass_count, pass_count, one));
            self.reg_alloc.free_temp(one);
            let ok_idx = self.add_string(format!("  ok      {name}"));
            let ok_msg = self.reg_alloc.alloc_temp();
            self.current_insts.push(encode_ri(Opcode::NEWSTR, ok_msg, ok_idx));
            let d1 = self.emit_native_regs("println", &[ok_msg]);
            self.reg_alloc.free_temp(d1);
            self.reg_alloc.free_temp(ok_msg);

            let to_end = self.current_insts.len();
            self.current_insts.push(0);

            let fail_at = self.current_insts.len() as i32;
            self.current_insts[to_fail] =
                encode_jc(Opcode::JMPIFNOT, is_ok, (fail_at - to_fail as i32) as i16);
            self.reg_alloc.free_temp(is_ok);

            let one2 = self.reg_alloc.alloc_temp();
            self.current_insts.push(encode_ri(Opcode::LOADI, one2, 1));
            self.current_insts.push(encode_r3(Opcode::ADD, fail_count, fail_count, one2));
            self.reg_alloc.free_temp(one2);

            let payload_str = self.emit_native_regs("str_from", &[payload]);
            self.reg_alloc.free_temp(payload);

            let prefix_idx = self.add_string(format!("  FAIL    {name}: "));
            let prefix = self.reg_alloc.alloc_temp();
            self.current_insts.push(encode_ri(Opcode::NEWSTR, prefix, prefix_idx));
            let fail_msg = self.reg_alloc.alloc_temp();
            self.current_insts.push(encode_r3(Opcode::ADD, fail_msg, prefix, payload_str));
            self.reg_alloc.free_temp(prefix);
            self.reg_alloc.free_temp(payload_str);

            let d2 = self.emit_native_regs("println", &[fail_msg]);
            self.reg_alloc.free_temp(d2);
            self.reg_alloc.free_temp(fail_msg);

            let end_at = self.current_insts.len() as i32;
            self.current_insts[to_end] = encode_ju(Opcode::JMP, end_at - to_end as i32);
            self.patch_test_skip(skip);
        }
        if let Some((f, p)) = filter {
            self.reg_alloc.free_temp(p);
            self.reg_alloc.free_temp(f);
        }

        let pass_str = self.emit_native_regs("str_from", &[pass_count]);
        let fail_str = self.emit_native_regs("str_from", &[fail_count]);

        let sep1_idx = self.add_string(" passed, ".to_string());
        let sep1 = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::NEWSTR, sep1, sep1_idx));
        let s1 = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(Opcode::ADD, s1, pass_str, sep1));
        self.reg_alloc.free_temp(sep1);
        self.reg_alloc.free_temp(pass_str);

        let s2 = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(Opcode::ADD, s2, s1, fail_str));
        self.reg_alloc.free_temp(s1);
        self.reg_alloc.free_temp(fail_str);

        let sep2_idx = self.add_string(" failed, ".to_string());
        let sep2 = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::NEWSTR, sep2, sep2_idx));
        let s3 = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(Opcode::ADD, s3, s2, sep2));
        self.reg_alloc.free_temp(sep2);
        self.reg_alloc.free_temp(s2);

        let ignored_str = self.emit_native_regs("str_from", &[ignored_count]);
        self.reg_alloc.free_temp(ignored_count);
        let s4 = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(Opcode::ADD, s4, s3, ignored_str));
        self.reg_alloc.free_temp(s3);
        self.reg_alloc.free_temp(ignored_str);

        let suffix_idx = self.add_string(" ignored".to_string());
        let suffix = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::NEWSTR, suffix, suffix_idx));
        let summary = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(Opcode::ADD, summary, s4, suffix));
        self.reg_alloc.free_temp(s4);
        self.reg_alloc.free_temp(suffix);

        let d3 = self.emit_native_regs("println", &[summary]);
        self.reg_alloc.free_temp(d3);
        self.reg_alloc.free_temp(summary);

        let zero = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::LOADI, zero, 0));
        let any_failed = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_r3(Opcode::GT, any_failed, fail_count, zero));
        self.reg_alloc.free_temp(zero);
        self.reg_alloc.free_temp(fail_count);
        self.reg_alloc.free_temp(pass_count);

        let to_skip_exit = self.current_insts.len();
        self.current_insts.push(0);

        let code = self.reg_alloc.alloc_temp();
        self.current_insts.push(encode_ri(Opcode::LOADI, code, 1));
        let d4 = self.emit_native_regs("process_exit", &[code]);
        self.reg_alloc.free_temp(d4);
        self.reg_alloc.free_temp(code);

        let skip_at = self.current_insts.len() as i32;
        self.current_insts[to_skip_exit] =
            encode_jc(Opcode::JMPIFNOT, any_failed, (skip_at - to_skip_exit as i32) as i16);
        self.reg_alloc.free_temp(any_failed);

        Ok(())
    }

    fn emit_unit_variant(&mut self, info: &VariantInfo, span: Span) -> Result<u8, (String, Span)> {
        if info.arity != 0 {
            return Err((
                format!("variant '{}' takes {} value(s)", info.enum_name, info.arity),
                span,
            ));
        }
        if info.is_option() {
            return Ok(self.emit_null());
        }
        let dest = self.reg_alloc.alloc_temp();
        self.emit_new_object(dest, info.type_idx, span);
        Ok(dest)
    }

    pub(crate) fn struct_init_target(&self, name: &str) -> Option<u16> {
        if let Some(&idx) = self.type_map.get(name) {
            return Some(idx);
        }
        let info = match name.rsplit_once('.') {
            Some((en, v)) => self.enum_variant_map.get(&(en.to_string(), v.to_string())),
            None => self.variant_map.get(name),
        };
        info.map(|i| i.type_idx)
    }

    pub(crate) fn field_slot(&self, type_idx: u16, field: &str, fallback: usize) -> u8 {
        self.type_descriptors[type_idx as usize]
            .fields
            .iter()
            .find(|f| f.name.as_deref() == Some(field))
            .map_or(fallback, |f| f.slot as usize) as u8
    }

    fn emit_variant_call(
        &mut self,
        info: &VariantInfo,
        args: &[Expr],
        span: Span,
    ) -> Result<u8, (String, Span)> {
        if args.len() != info.arity {
            return Err((
                format!("variant expects {} value(s), got {}", info.arity, args.len()),
                span,
            ));
        }
        if info.is_option() {
            if info.arity == 0 {
                return Ok(self.emit_null());
            }
            let v = self.compile_value(&args[0])?;
            let dest = self.emit_some(v);
            self.reg_alloc.free_temp(v);
            return Ok(dest);
        }
        let dest = self.reg_alloc.alloc_temp();
        self.emit_new_object(dest, info.type_idx, span);
        for (i, a) in args.iter().enumerate() {
            let r = self.compile_value(a)?;
            self.current_insts.push(encode_r3(Opcode::SETFIELD, dest, i as u8, r));
            self.reg_alloc.free_temp(r);
        }
        Ok(dest)
    }

    fn needs_copy(&self, e: &Expr) -> bool {
        match e {
            Expr::Int(..) | Expr::Float(..) | Expr::Bool(..) | Expr::String(..) | Expr::Char(..)
            | Expr::Null(..) | Expr::Binary { .. } | Expr::Unary { .. } | Expr::StructInit { .. }
            | Expr::Lambda { .. } | Expr::ListLiteral { .. } | Expr::MapLiteral { .. }
            | Expr::TupleLiteral { .. } => false,
            Expr::Ident(..) => !self.static_class_name(e).is_some_and(|t| self.class_names.contains(t)),
            Expr::MemberAccess { .. } if self.reads_embedded(e) => false,
            Expr::Call { callee, .. } => match callee.as_ref() {
                Expr::Ident(n, _) => !self.func_map.contains_key(n),
                Expr::MemberAccess { object, member, .. } => {
                    let user_method = self
                        .static_class_name(object)
                        .is_some_and(|c| self.func_map.contains_key(&Self::mangle_method_name(c, member)));
                    let static_call = matches!(object.as_ref(), Expr::Ident(c, _)
                        if self.reg_alloc.get_var(c).is_none()
                            && self.func_map.contains_key(&Self::mangle_method_name(c, member)));
                    !(user_method || static_call)
                }
                _ => true,
            },
            _ => true,
        }
    }

    fn copy_if_needed(&mut self, e: &Expr, r: u8) {
        if self.needs_copy(e) {
            self.current_insts.push(encode_r2(Opcode::COPYVAL, r, r));
        }
    }

    fn compile_value(&mut self, e: &Expr) -> Result<u8, (String, Span)> {
        let r = self.compile_expr(e)?;
        self.copy_if_needed(e, r);
        Ok(r)
    }

    fn expr_kind(expr: &Expr) -> &'static str {
        match expr {
            Expr::SelfValue(_) => "`self`",
            Expr::StaticAccess { .. } => "a `Type.member` access",
            Expr::Range { .. } => "range expressions",
            Expr::Lambda { .. } => "lambda expressions",
            Expr::If { .. } => "`if` as an expression",
            Expr::Match { .. } => "`match` as an expression",
            Expr::Try { .. } => "the `?` operator",
            Expr::Unwrap { .. } => "the `!` unwrap operator",
            Expr::Block { .. } => "a block as an expression",
            _ => "this expression",
        }
    }

    fn add_constant(&mut self, val: Value) -> u16 {
        let idx = self.current_constants.len() as u16;
        self.current_constants.push(val);
        idx
    }

    fn add_string(&mut self, s: String) -> u16 {
        if let Some(i) = self.current_strings.iter().position(|e| *e == s) {
            return i as u16;
        }
        let idx = self.current_strings.len() as u16;
        self.current_strings.push(s);
        idx
    }

}
