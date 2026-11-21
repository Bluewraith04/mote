use mote_base::{Arena, Span, Symbol};
use mote_semantics::TyId;

use crate::ir::{Block, BlockId, FrameSlot, Function, FunctionKind, Inst, Op, Operand, Reg, RegId, Terminator};

/// Builds one function block by block.
pub struct FunctionBuilder {
    kind: FunctionKind,
    regs: Arena<Reg>,
    captures: Vec<RegId>,
    params: Vec<RegId>,
    out_params: Vec<usize>,
    ret: TyId,
    is_generator: bool,
    frame: Vec<FrameSlot>,
    blocks: Arena<Block>,
    terminated: Vec<bool>,
    entry: BlockId,
    current: BlockId,
}

impl FunctionBuilder {
    /// Starts a function with an entry block, which is current.
    pub fn new(kind: FunctionKind, ret: TyId) -> Self {
        let mut blocks = Arena::new();
        let entry = blocks.alloc(Block { insts: Vec::new(), term: Terminator::Unreachable });
        Self { kind, regs: Arena::new(), captures: Vec::new(), params: Vec::new(), out_params: Vec::new(), ret, is_generator: false, frame: Vec::new(), blocks, terminated: vec![false], entry, current: entry }
    }

    pub fn set_generator(&mut self) {
        self.is_generator = true;
    }

    pub fn reg(&mut self, ty: TyId, name: Option<Symbol>) -> RegId {
        self.regs.alloc(Reg { ty, name, cell: false })
    }

    /// A register holding a shared box around a value of the type.
    pub fn cell(&mut self, ty: TyId, name: Option<Symbol>) -> RegId {
        self.regs.alloc(Reg { ty, name, cell: true })
    }

    /// Adds a parameter; a `var` one is copied back on return.
    pub fn param(&mut self, ty: TyId, name: Option<Symbol>, is_var: bool) -> RegId {
        let reg = self.reg(ty, name);
        if is_var {
            self.out_params.push(self.params.len());
        }
        self.params.push(reg);
        reg
    }

    pub fn capture(&mut self, ty: TyId, name: Option<Symbol>, cell: bool) -> RegId {
        let reg = if cell { self.cell(ty, name) } else { self.reg(ty, name) };
        self.captures.push(reg);
        reg
    }

    pub fn block(&mut self) -> BlockId {
        self.terminated.push(false);
        self.blocks.alloc(Block { insts: Vec::new(), term: Terminator::Unreachable })
    }

    /// Adds a slot to the frame of a `Resume` function and gives its position.
    pub fn slot(&mut self, ty: TyId, cell: bool) -> u32 {
        self.frame.push(FrameSlot { ty, cell });
        (self.frame.len() - 1) as u32
    }

    pub fn switch_to(&mut self, block: BlockId) {
        self.current = block;
    }

    pub fn current(&self) -> BlockId {
        self.current
    }

    /// Whether the current block already ends.
    pub fn is_terminated(&self) -> bool {
        self.terminated[self.current.index()]
    }

    pub fn emit(&mut self, dest: Option<RegId>, op: Op, span: Span) {
        assert!(!self.is_terminated(), "emit into a terminated block");
        self.blocks[self.current].insts.push(Inst { dest, op, span });
    }

    /// Emits an operation into a fresh register of the type.
    pub fn value(&mut self, ty: TyId, op: Op, span: Span) -> Operand {
        let dest = self.reg(ty, None);
        self.emit(Some(dest), op, span);
        Operand::Reg(dest)
    }

    pub fn terminate(&mut self, term: Terminator) {
        assert!(!self.is_terminated(), "block terminated twice");
        self.blocks[self.current].term = term;
        self.terminated[self.current.index()] = true;
    }

    /// The function, or the first block that has no terminator.
    pub fn finish(self) -> Result<Function, BlockId> {
        if let Some(open) = self.terminated.iter().position(|&done| !done) {
            return Err(self.blocks.iter().nth(open).map(|(id, _)| id).expect("block index"));
        }
        Ok(Function { kind: self.kind, regs: self.regs, captures: self.captures, params: self.params, out_params: self.out_params, ret: self.ret, is_generator: self.is_generator, frame: self.frame, blocks: self.blocks, entry: self.entry })
    }
}
