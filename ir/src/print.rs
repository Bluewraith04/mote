use std::fmt::Write;

use mote_base::Interner;
use mote_semantics::{BinaryOp, Conversion, Program, UnaryOp};

use crate::ir::{Callee, Const, EntryKind, Function, FunctionKind, Inst, Module, Op, Operand, RegId, Terminator};

/// Every function of the module as text, for tests.
pub fn print_module(module: &Module, program: &Program, interner: &Interner) -> String {
    let mut out = String::new();
    if let Some(start) = module.start {
        let _ = writeln!(out, "start f{}", start.index());
    }
    for entry in &module.entries {
        let what = match entry.kind {
            EntryKind::Main => "main".to_string(),
            EntryKind::Test(name) => format!("test \"{}\"", interner.resolve(name)),
        };
        let _ = writeln!(out, "entry {what} f{}", entry.func.index());
    }
    for (id, function) in module.functions.iter() {
        let _ = writeln!(out, "f{}:", id.index());
        out.push_str(&print_function(function, program, interner));
    }
    out
}

/// One function as text: the registers with their types, then each block.
pub fn print_function(function: &Function, program: &Program, interner: &Interner) -> String {
    let printer = Printer { program, interner };
    let mut out = String::new();
    let kind = match &function.kind {
        FunctionKind::Instance(instance) => format!("instance {}", interner.resolve(program.defs[instance.def].name)),
        FunctionKind::Closure { parent } => format!("closure of f{}", parent.index()),
        FunctionKind::Native(instance) => format!("native {}", interner.resolve(program.defs[instance.def].name)),
        FunctionKind::Resume { generator } => format!("resume of f{}", generator.index()),
        FunctionKind::Global(def) => format!("global {}", interner.resolve(program.defs[*def].name)),
        FunctionKind::TopLevel(module) => format!("top-level m{}", module.index()),
        FunctionKind::Start => "start".to_string(),
    };
    let generator = if function.is_generator { "generator " } else { "" };
    let _ = writeln!(out, "{generator}fn {kind}({}) -> {}", printer.regs(&function.params), program.render(function.ret, interner));
    if !function.captures.is_empty() {
        let _ = writeln!(out, "  captures {}", printer.regs(&function.captures));
    }
    for (at, slot) in function.frame.iter().enumerate() {
        let cell = if slot.cell { "cell " } else { "" };
        let _ = writeln!(out, "  slot {at}: {cell}{}", program.render(slot.ty, interner));
    }
    for (id, reg) in function.regs.iter() {
        let cell = if reg.cell { "cell " } else { "" };
        let _ = writeln!(out, "  r{}: {cell}{}", id.index(), program.render(reg.ty, interner));
    }
    for (id, block) in function.blocks.iter() {
        let _ = writeln!(out, "  b{}:", id.index());
        for inst in &block.insts {
            let _ = writeln!(out, "    {}", printer.inst(inst));
        }
        let _ = writeln!(out, "    {}", printer.term(&block.term));
    }
    out
}

/// One instruction as text.
pub(crate) fn print_inst(inst: &Inst, program: &Program, interner: &Interner) -> String {
    Printer { program, interner }.inst(inst)
}

struct Printer<'a> {
    program: &'a Program,
    interner: &'a Interner,
}

impl Printer<'_> {
    fn regs(&self, regs: &[RegId]) -> String {
        regs.iter().map(|reg| format!("r{}", reg.index())).collect::<Vec<_>>().join(", ")
    }

    fn operand(&self, operand: &Operand) -> String {
        match operand {
            Operand::Reg(reg) => format!("r{}", reg.index()),
            Operand::Const(Const::Int(n)) => n.to_string(),
            Operand::Const(Const::Float(x)) => format!("{x:?}"),
            Operand::Const(Const::Bool(b)) => b.to_string(),
            Operand::Const(Const::Null) => "null".to_string(),
            Operand::Const(Const::String(s)) => format!("{:?}", self.interner.resolve(*s)),
        }
    }

    fn list(&self, operands: &[Operand]) -> String {
        operands.iter().map(|operand| self.operand(operand)).collect::<Vec<_>>().join(", ")
    }

    fn optional(&self, operand: &Option<Operand>) -> String {
        operand.as_ref().map_or("_".to_string(), |operand| self.operand(operand))
    }

    /// A slice bound, empty when left out.
    fn bound(&self, operand: &Option<Operand>) -> String {
        operand.as_ref().map_or(String::new(), |operand| self.operand(operand))
    }

    fn callee(&self, callee: &Callee) -> String {
        match callee {
            Callee::Function(func) => format!("f{}", func.index()),
            Callee::Value(operand) => format!("value {}", self.operand(operand)),
            Callee::Intrinsic(name) => format!("intrinsic {}", self.interner.resolve(*name)),
            Callee::Native(def) => format!("native {}", self.interner.resolve(self.program.defs[*def].name)),
        }
    }

    fn inst(&self, inst: &Inst) -> String {
        let op = self.op(&inst.op);
        match inst.dest {
            Some(dest) => format!("r{} = {op}", dest.index()),
            None => op,
        }
    }

    fn op(&self, op: &Op) -> String {
        match op {
            Op::Copy(a) => format!("copy {}", self.operand(a)),
            Op::Unary { op, operand } => format!("{} {}", unary(*op), self.operand(operand)),
            Op::Binary { op, left, right } => format!("{} {}, {}", binary(*op), self.operand(left), self.operand(right)),
            Op::Cast(a) => format!("cast {}", self.operand(a)),
            Op::Show(a) => format!("show {}", self.operand(a)),
            Op::Concat(parts) => format!("concat {}", self.list(parts)),
            Op::Convert { conversion, operand } => format!("{} {}", conversion_name(*conversion), self.operand(operand)),
            Op::Tuple(items) => format!("tuple {}", self.list(items)),
            Op::List(items) => format!("list {}", self.list(items)),
            Op::Array(items) => format!("array {}", self.list(items)),
            Op::ArrayRepeat { value, len } => format!("array_repeat {}, {}", self.operand(value), self.operand(len)),
            Op::Record(fields) => format!("record {}", self.list(fields)),
            Op::Variant { variant, fields } if fields.is_empty() => format!("variant {variant}"),
            Op::Variant { variant, fields } => format!("variant {variant} {}", self.list(fields)),
            Op::Range { start, end, inclusive } => format!("range {}, {}{}", self.operand(start), self.operand(end), if *inclusive { " inclusive" } else { "" }),
            Op::Field { object, index } => format!("field {}.{index}", self.operand(object)),
            Op::SetField { object, index, value } => format!("set_field {}.{index}, {}", self.operand(object), self.operand(value)),
            Op::TupleField { object, index } => format!("tuple_field {}.{index}", self.operand(object)),
            Op::Len(a) => format!("len {}", self.operand(a)),
            Op::Index { object, index } => format!("index {}[{}]", self.operand(object), self.operand(index)),
            Op::SetIndex { object, index, value } => format!("set_index {}[{}], {}", self.operand(object), self.operand(index), self.operand(value)),
            Op::Slice { object, start, end, inclusive, step } => {
                format!("slice {}[{}..{}{}{}]", self.operand(object), self.bound(start), if *inclusive { "=" } else { "" }, self.bound(end), step.as_ref().map_or(String::new(), |s| format!("..{}", self.operand(s))))
            }
            Op::SetSlice { object, start, end, inclusive, value } => {
                format!("set_slice {}[{}..{}{}], {}", self.operand(object), self.bound(start), if *inclusive { "=" } else { "" }, self.bound(end), self.operand(value))
            }
            Op::NewCell(a) => format!("new_cell {}", self.operand(a)),
            Op::LoadCell(a) => format!("load_cell {}", self.operand(a)),
            Op::StoreCell { cell, value } => format!("store_cell {}, {}", self.operand(cell), self.operand(value)),
            Op::LoadGlobal(def) => format!("load_global {}", self.interner.resolve(self.program.defs[*def].name)),
            Op::StoreGlobal { def, value } => format!("store_global {}, {}", self.interner.resolve(self.program.defs[*def].name), self.operand(value)),
            Op::IsSome(a) => format!("is_some {}", self.operand(a)),
            Op::SomePayload(a) => format!("some_payload {}", self.operand(a)),
            Op::Unwrap(a) => format!("unwrap {}", self.operand(a)),
            Op::VariantTag(a) => format!("variant_tag {}", self.operand(a)),
            Op::IsVariant { object, variant } => format!("is_variant {}, {variant}", self.operand(object)),
            Op::VariantField { object, variant, index } => format!("variant_field {}, {variant}.{index}", self.operand(object)),
            Op::IsType { operand, ty } => format!("is_type {}, {}", self.operand(operand), self.program.render(*ty, self.interner)),
            Op::UnionPayload { operand, ty } => format!("union_payload {}, {}", self.operand(operand), self.program.render(*ty, self.interner)),
            Op::Call { callee, args, outs } => {
                let outs = if outs.is_empty() { String::new() } else { format!(" -> {}", self.regs(outs)) };
                format!("call {}({}){outs}", self.callee(callee), self.list(args))
            }
            Op::Closure { func, captures } => format!("closure f{} [{}]", func.index(), self.list(captures)),
            Op::Spawn { func, captures, scope } => {
                let scope = scope.as_ref().map_or(String::new(), |s| format!(" in {}", self.operand(s)));
                format!("spawn f{} [{}]{scope}", func.index(), self.list(captures))
            }
            Op::ScopeBegin => "scope_begin".to_string(),
            Op::Defer { resource, func } => format!("defer {}, f{}", self.operand(resource), func.index()),
            Op::Undefer => "undefer".to_string(),
            Op::ScopeEnd(a) => format!("scope_end {}", self.operand(a)),
            Op::Yield(a) => format!("yield {}", self.operand(a)),
            Op::Stream { resume, init } => format!("stream f{} [{}]", resume.index(), self.list(init)),
            Op::FrameLoad(slot) => format!("frame_load {slot}"),
            Op::FrameStore { slot, value } => format!("frame_store {slot}, {}", self.operand(value)),
            Op::Detach(a) => format!("detach {}", self.operand(a)),
        }
    }

    fn term(&self, term: &Terminator) -> String {
        match term {
            Terminator::Jump(target) => format!("jump b{}", target.index()),
            Terminator::Branch { condition, then, otherwise } => format!("branch {}, b{}, b{}", self.operand(condition), then.index(), otherwise.index()),
            Terminator::Switch { value, cases, default } => {
                let cases = cases.iter().map(|(n, block)| format!("{n} => b{}", block.index())).collect::<Vec<_>>().join(", ");
                format!("switch {} [{cases}] else b{}", self.operand(value), default.index())
            }
            Terminator::Return(value) => format!("return {}", self.optional(value)),
            Terminator::Unreachable => "unreachable".to_string(),
        }
    }
}

fn unary(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Neg => "neg",
        UnaryOp::Not => "not",
        UnaryOp::BitNot => "bit_not",
    }
}

fn binary(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "add",
        BinaryOp::Sub => "sub",
        BinaryOp::Mul => "mul",
        BinaryOp::Div => "div",
        BinaryOp::Rem => "rem",
        BinaryOp::Eq => "eq",
        BinaryOp::NotEq => "ne",
        BinaryOp::Lt => "lt",
        BinaryOp::LtEq => "le",
        BinaryOp::Gt => "gt",
        BinaryOp::GtEq => "ge",
        BinaryOp::And => "and",
        BinaryOp::Or => "or",
        BinaryOp::BitAnd => "bit_and",
        BinaryOp::BitOr => "bit_or",
        BinaryOp::BitXor => "bit_xor",
        BinaryOp::Shl => "shl",
        BinaryOp::Shr => "shr",
        BinaryOp::Coalesce => "coalesce",
    }
}

pub(crate) fn conversion_name(conversion: Conversion) -> &'static str {
    match conversion {
        Conversion::ToOptional => "to_optional",
        Conversion::ToUnion => "to_union",
        Conversion::WidenUnion => "widen_union",
        Conversion::Narrow => "narrow",
        Conversion::Adapt => "adapt",
        Conversion::ToView => "to_view",
    }
}
