use std::collections::HashMap;

use mote_base::{Arena, Id, Span, Symbol};
use mote_semantics::{BinaryOp, Conversion, DefId, Instance, ModuleId, TyId, UnaryOp};

pub type FuncId = Id<Function>;
pub type BlockId = Id<Block>;
pub type RegId = Id<Reg>;

/// Every function of a program.
#[derive(Clone, Default)]
pub struct Module {
    pub functions: Arena<Function>,
    pub instances: HashMap<Instance, FuncId>,
    /// Runs each start-up step once, before any entry.
    pub start: Option<FuncId>,
    /// The functions a run can begin at, in the order the program lists them.
    pub entries: Vec<Entry>,
}

/// A function the runtime calls after `start`.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub kind: EntryKind,
    pub func: FuncId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Main,
    /// A `test` block, by its name.
    Test(Symbol),
}

/// What a function was lowered from.
#[derive(Clone, Debug, PartialEq)]
pub enum FunctionKind {
    Instance(Instance),
    /// The body of a lambda, a nested `fn` or a `spawn` block.
    Closure { parent: FuncId },
    /// An external or intrinsic function used as a value: it calls the function.
    Native(Instance),
    /// The body of a generator after `lower_generators`: it runs on the frame and returns the next element, or `null` at the end.
    Resume { generator: FuncId },
    /// The initializer of a global.
    Global(DefId),
    /// A module's top-level statements.
    TopLevel(ModuleId),
    Start,
}

#[derive(Clone)]
pub struct Function {
    pub kind: FunctionKind,
    pub regs: Arena<Reg>,
    /// Values a closure carries, filled when it is called; empty for other functions.
    pub captures: Vec<RegId>,
    /// The receiver first, then the parameters.
    pub params: Vec<RegId>,
    /// Positions in `params` of the `var` parameters, copied back to the caller on return.
    pub out_params: Vec<usize>,
    pub ret: TyId,
    /// A call builds a `Stream` over the body, which runs on demand and suspends at each `Yield`; `lower_generators` removes it.
    pub is_generator: bool,
    /// The slots of the heap frame a `Resume` function runs on; empty for any other.
    pub frame: Vec<FrameSlot>,
    pub blocks: Arena<Block>,
    pub entry: BlockId,
}

/// One word of a generator frame: a value of `ty`, or the cell that holds one.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameSlot {
    pub ty: TyId,
    pub cell: bool,
}

/// A mutable slot; the code reads and writes it any number of times.
#[derive(Clone, Debug, PartialEq)]
pub struct Reg {
    pub ty: TyId,
    pub name: Option<Symbol>,
    /// Holds a shared box around a value of `ty`, for a `var` a closure captures.
    pub cell: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub insts: Vec<Inst>,
    pub term: Terminator,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Inst {
    pub dest: Option<RegId>,
    pub op: Op,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Const {
    Int(i128),
    Float(f64),
    Bool(bool),
    Null,
    String(Symbol),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Operand {
    Reg(RegId),
    Const(Const),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Callee {
    Function(FuncId),
    /// A function value.
    Value(Operand),
    /// A compiler intrinsic operation named by `@name`.
    Intrinsic(Symbol),
    /// An `extern` function bound to a library.
    Native(DefId),
}

/// One step. The type of `dest` is the type of the result.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Copy(Operand),
    Unary { op: UnaryOp, operand: Operand },
    /// Arithmetic and shifts fault on overflow, division by zero and a shift out of range; `Add` on `String` joins. `And` and `Or` are on `Bool` values already computed, never short-circuit.
    Binary { op: BinaryOp, left: Operand, right: Operand },
    /// A scalar cast to the type of `dest`.
    Cast(Operand),
    /// A number or `Bool` as text.
    Show(Operand),
    Concat(Vec<Operand>),
    Convert { conversion: Conversion, operand: Operand },

    Tuple(Vec<Operand>),
    List(Vec<Operand>),
    Array(Vec<Operand>),
    ArrayRepeat { value: Operand, len: Operand },
    /// Every field, in declaration order.
    Record(Vec<Operand>),
    /// The payload in order.
    Variant { variant: u32, fields: Vec<Operand> },
    Range { start: Operand, end: Operand, inclusive: bool },

    Field { object: Operand, index: u32 },
    /// Writes a field of a record or class, or an element of a tuple, in the object the register holds.
    SetField { object: Operand, index: u32, value: Operand },
    TupleField { object: Operand, index: u32 },
    /// The number of elements of a list or array.
    Len(Operand),
    /// Faults when the position is out of range or the key is missing.
    Index { object: Operand, index: Operand },
    SetIndex { object: Operand, index: Operand, value: Operand },
    Slice { object: Operand, start: Option<Operand>, end: Option<Operand>, inclusive: bool, step: Option<Operand> },
    SetSlice { object: Operand, start: Option<Operand>, end: Option<Operand>, inclusive: bool, value: Operand },

    /// A cell holding the value; `null` makes an empty cell of any type, filled by `StoreCell`.
    NewCell(Operand),
    LoadCell(Operand),
    StoreCell { cell: Operand, value: Operand },
    LoadGlobal(DefId),
    StoreGlobal { def: DefId, value: Operand },

    /// `true` when the optional holds a value.
    IsSome(Operand),
    /// The value of an optional already known to hold one.
    SomePayload(Operand),
    /// The value of an optional or `Result`, or a fault.
    Unwrap(Operand),
    /// The position of the variant an enum value holds.
    VariantTag(Operand),
    /// `true` when the enum value is the variant.
    IsVariant { object: Operand, variant: u32 },
    VariantField { object: Operand, variant: u32, index: u32 },
    /// `true` when the run-time type of a union value fits the type.
    IsType { operand: Operand, ty: TyId },
    /// The member of a union already known to be the type.
    UnionPayload { operand: Operand, ty: TyId },

    /// `outs` receive the final value of each `var` parameter, in `out_params` order.
    Call { callee: Callee, args: Vec<Operand>, outs: Vec<RegId> },
    /// A function value over the captured values.
    Closure { func: FuncId, captures: Vec<Operand> },
    /// Starts a task; `scope` is the token of the enclosing `scope`.
    Spawn { func: FuncId, captures: Vec<Operand>, scope: Option<Operand> },
    /// Opens a `scope` and yields its token, an `Int` the runtime owns; a task cancelled before `ScopeEnd` cancels the scope's tasks and waits for them.
    ScopeBegin,
    /// Waits for every task spawned in the scope.
    ScopeEnd(Operand),
    /// Enters a `with`: if the task is cancelled before `Undefer`, `close` is called on the resource.
    Defer { resource: Operand, func: FuncId },
    /// Leaves the innermost open `with`; its `close` is called by the code that follows.
    Undefer,
    /// Hands a value to the consumer of a generator and suspends.
    Yield(Operand),
    /// A `Stream` over a new frame for `resume`: slot 0 holds `0`, and slots 1 on the `init` values in order.
    Stream { resume: FuncId, init: Vec<Operand> },
    /// The value or cell in a slot of the frame the `Resume` function runs on.
    FrameLoad(u32),
    FrameStore { slot: u32, value: Operand },
    /// A copy of every `[T]` the value holds, for a value that enters another task.
    Detach(Operand),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Terminator {
    Jump(BlockId),
    Branch { condition: Operand, then: BlockId, otherwise: BlockId },
    Switch { value: Operand, cases: Vec<(i128, BlockId)>, default: BlockId },
    /// The `var` parameters go back to the caller with the value.
    Return(Option<Operand>),
    Unreachable,
}

impl Terminator {
    /// The blocks control can go to next.
    pub fn targets(&self) -> Vec<BlockId> {
        match self {
            Terminator::Jump(target) => vec![*target],
            Terminator::Branch { then, otherwise, .. } => vec![*then, *otherwise],
            Terminator::Switch { cases, default, .. } => cases.iter().map(|&(_, block)| block).chain([*default]).collect(),
            Terminator::Return(_) | Terminator::Unreachable => Vec::new(),
        }
    }
}

impl Module {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, function: Function) -> FuncId {
        let id = self.functions.alloc(function);
        if let FunctionKind::Instance(instance) = &self.functions[id].kind {
            self.instances.insert(instance.clone(), id);
        }
        id
    }

    pub fn function(&self, instance: &Instance) -> Option<FuncId> {
        self.instances.get(instance).copied()
    }
}
