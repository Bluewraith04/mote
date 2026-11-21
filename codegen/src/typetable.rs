//! Builds the `TypeTable` of a program: one entry for each type the IR names, and the entries the runtime needs behind them.

use std::collections::{HashMap, HashSet};

use mote_abi::{HandleKind, Kind, PairPayload, Scalar, TypeId, TypeTable};
use mote_base::Interner;
use mote_ir::{FuncId, FunctionKind, Module, Op, Operand};
use mote_semantics::{BuiltinTy, DefId, DefKind, FloatTy, GenericArg, IntTy, Payload, Program, TyId, TyKind};

/// The table of a program and the way from a `TyId` to its entry.
pub struct TypeMap {
    pub table: TypeTable,
    ids: HashMap<TyId, TypeId>,
    closures: HashMap<FuncId, TypeId>,
    boxes: HashMap<TyId, TypeId>,
    channels: HashMap<TyId, TypeId>,
    buffers: HashMap<TypeId, TypeId>,
    maps: HashSet<TypeId>,
}

impl TypeMap {
    /// The entry of a type the IR names.
    pub fn id(&self, ty: TyId) -> TypeId {
        *self.ids.get(&ty).unwrap_or_else(|| panic!("type {} is not in the table", ty.index()))
    }

    /// The object type of the closure, stream frame or function value that runs `func`.
    pub fn closure(&self, func: FuncId) -> TypeId {
        *self.closures.get(&func).unwrap_or_else(|| panic!("function {} has no closure type", func.index()))
    }

    /// The `Box` that holds a value of the type: a shared cell, a union payload or a recursive child.
    pub fn boxed(&self, ty: TyId) -> TypeId {
        *self.boxes.get(&ty).unwrap_or_else(|| panic!("type {} has no box", ty.index()))
    }

    /// Every type of the program and its entry.
    pub fn entries(&self) -> impl Iterator<Item = (TyId, TypeId)> + '_ {
        self.ids.iter().map(|(&ty, &id)| (ty, id))
    }

    /// The channel behind the `Sender` and `Receiver` of messages of the type.
    pub fn channel(&self, message: TyId) -> TypeId {
        *self.channels.get(&message).unwrap_or_else(|| panic!("type {} has no channel", message.index()))
    }

    /// Whether the type is a `Map` or a `Set`.
    pub fn is_map(&self, ty: TypeId) -> bool {
        self.maps.contains(&ty)
    }

    /// The `[T]` that a list of `T` holds its elements in.
    pub fn buffer(&self, list: TypeId) -> TypeId {
        *self.buffers.get(&list).unwrap_or_else(|| panic!("type {} has no buffer", list.raw()))
    }
}

/// Builds the table for every type the module names.
pub fn build(module: &Module, program: &mut Program, interner: &Interner) -> TypeMap {
    let mut builder = Builder { program, interner, table: TypeTable::new(), ids: HashMap::new(), closures: HashMap::new(), boxes: HashMap::new(), channels: HashMap::new(), buffers: HashMap::new(), maps: HashSet::new(), queue: Vec::new() };
    builder.roots(module);
    builder.drain();
    TypeMap { table: builder.table, ids: builder.ids, closures: builder.closures, boxes: builder.boxes, channels: builder.channels, buffers: builder.buffers, maps: builder.maps }
}

/// An object type that is reserved and waits for its members.
enum Work {
    Type(TyId),
    Box(TypeId, TyId),
    Channel(TypeId, TyId),
    /// A closure over members, each a type and whether it is held in a cell.
    Closure(TypeId, Vec<(TyId, bool)>),
}

struct Builder<'a> {
    program: &'a mut Program,
    interner: &'a Interner,
    table: TypeTable,
    ids: HashMap<TyId, TypeId>,
    closures: HashMap<FuncId, TypeId>,
    boxes: HashMap<TyId, TypeId>,
    channels: HashMap<TyId, TypeId>,
    buffers: HashMap<TypeId, TypeId>,
    maps: HashSet<TypeId>,
    queue: Vec<Work>,
}

impl Builder<'_> {
    fn roots(&mut self, module: &Module) {
        self.ensure(TyId::STRING);
        let globals: Vec<TyId> = self.program.defs.iter().filter_map(|(_, def)| if let DefKind::Global(global) = &def.kind { Some(global.ty) } else { None }).collect();
        for ty in globals {
            self.ensure(ty);
        }
        let valued: HashSet<FuncId> = module
            .functions
            .iter()
            .flat_map(|(_, function)| function.blocks.iter())
            .flat_map(|(_, block)| block.insts.iter())
            .filter_map(|inst| if let Op::Closure { func, .. } = &inst.op { Some(*func) } else { None })
            .collect();
        for (id, function) in module.functions.iter() {
            self.ensure(function.ret);
            for (_, reg) in function.regs.iter() {
                self.ensure(reg.ty);
                if reg.cell {
                    self.boxed(reg.ty);
                }
            }
            for (_, block) in function.blocks.iter() {
                for inst in &block.insts {
                    if let Op::IsType { ty, .. } | Op::UnionPayload { ty, .. } = inst.op {
                        self.ensure(ty);
                    }
                    if let Op::Convert { conversion: mote_semantics::Conversion::ToUnion, operand: Operand::Reg(reg) } = &inst.op {
                        let held = function.regs[*reg].ty;
                        if !matches!(self.program.types.kind(held), TyKind::Union(_) | TyKind::Any) {
                            let held_id = self.ensure(held);
                            if self.table.pair_payload(held_id) == PairPayload::Boxed {
                                self.boxed(held);
                            }
                        }
                    }
                    if let Op::Defer { resource: Operand::Reg(reg), .. } = inst.op {
                        self.boxed(function.regs[reg].ty);
                    }
                }
            }
            let members = match function.kind {
                FunctionKind::Closure { .. } => Some(function.captures.iter().map(|&capture| (function.regs[capture].ty, function.regs[capture].cell)).collect::<Vec<_>>()),
                FunctionKind::Native(_) => Some(Vec::new()),
                FunctionKind::Resume { .. } => Some(function.frame.iter().map(|slot| (slot.ty, slot.cell)).collect()),
                _ if valued.contains(&id) => Some(Vec::new()),
                _ => None,
            };
            if let Some(members) = members {
                let closure = self.table.reserve_object(format!("closure#{}", id.index()));
                self.closures.insert(id, closure);
                self.queue.push(Work::Closure(closure, members));
            }
        }
    }

    fn name(&self, ty: TyId) -> String {
        self.program.render_source(ty, self.interner)
    }

    fn scalar(&mut self, ty: TyId, scalar: Scalar) -> TypeId {
        let name = self.name(ty);
        self.table.add(name, Kind::Scalar(scalar))
    }

    /// The entry of the type, adding it and what it holds.
    fn ensure(&mut self, ty: TyId) -> TypeId {
        if let Some(&id) = self.ids.get(&ty) {
            return id;
        }
        let id = self.add(ty);
        self.ids.insert(ty, id);
        id
    }

    fn add(&mut self, ty: TyId) -> TypeId {
        let kind = self.program.types.kind(ty).clone();
        match kind {
            TyKind::Unit => self.table.add("Null", Kind::Unit),
            TyKind::Bool => self.scalar(ty, Scalar::I8),
            TyKind::Int(int) => self.scalar(ty, int_scalar(int)),
            TyKind::Float(FloatTy::F32) => self.scalar(ty, Scalar::F32),
            TyKind::Float(FloatTy::F64) => self.scalar(ty, Scalar::F64),
            TyKind::String => self.table.add("String", Kind::String),
            TyKind::Tuple(elements) => {
                let members = self.members(ty, &elements);
                let name = self.name(ty);
                self.table.add(name, Kind::Struct(members))
            }
            TyKind::FixedArray(element, count) => {
                let element = self.ensure(element);
                let count = count.as_u32().expect("concrete fixed array length");
                let name = self.name(ty);
                self.table.add(name, Kind::FixedArray(element, count))
            }
            TyKind::Optional(inner) => {
                let inner = self.ensure(inner);
                let name = self.name(ty);
                self.table.add(name, Kind::Optional(inner))
            }
            TyKind::Union(members) => {
                for member in members {
                    let id = self.ensure(member);
                    if self.table.pair_payload(id) == PairPayload::Boxed {
                        self.boxed(member);
                    }
                }
                let name = self.name(ty);
                self.table.add(name, Kind::Pair)
            }
            TyKind::Any => self.table.add("Any", Kind::Pair),
            TyKind::Array(_) | TyKind::Function(_) => self.reserve(ty),
            TyKind::Builtin { kind, .. } => match kind {
                BuiltinTy::Pointer => {
                    let address = self.ensure(TyId::UINT64);
                    let name = self.name(ty);
                    self.table.add(name, Kind::Struct(vec![address]))
                }
                _ => self.reserve(ty),
            },
            TyKind::Adt { def, args } => self.adt(ty, def, &args),
            TyKind::Error | TyKind::Param { .. } | TyKind::Infer(_) => panic!("type {} is not concrete", ty.index()),
        }
    }

    /// Reserves an object type to be defined once the value types around it are in the table.
    fn reserve(&mut self, ty: TyId) -> TypeId {
        let name = self.name(ty);
        let id = self.table.reserve_object(name);
        self.queue.push(Work::Type(ty));
        id
    }

    fn adt(&mut self, ty: TyId, def: DefId, args: &[GenericArg]) -> TypeId {
        let item = &self.program.defs[def];
        if item.is_class() {
            return self.reserve(ty);
        }
        match &item.kind {
            DefKind::Enum(_) => {
                let variants = self.variants(def, args);
                let mut built = Vec::new();
                for payload in variants {
                    built.push(self.members(ty, &payload));
                }
                let name = self.name(ty);
                self.table.add(name, Kind::Enum(built))
            }
            _ => {
                let fields = self.fields(def, args);
                let members = self.members(ty, &fields);
                let name = self.name(ty);
                self.table.add(name, Kind::Struct(members))
            }
        }
    }

    fn fields(&mut self, def: DefId, args: &[GenericArg]) -> Vec<TyId> {
        let declared: Vec<TyId> = self.program.defs[def].record().map(|record| record.fields.iter().map(|field| field.ty).collect()).unwrap_or_default();
        declared.into_iter().map(|field| self.program.types.subst_generic(field, args)).collect()
    }

    fn variants(&mut self, def: DefId, args: &[GenericArg]) -> Vec<Vec<TyId>> {
        let declared: Vec<Vec<TyId>> = self.program.defs[def]
            .enum_def()
            .map(|item| {
                item.variants
                    .iter()
                    .map(|variant| match &variant.payload {
                        Payload::Unit => Vec::new(),
                        Payload::Tuple(types) => types.clone(),
                        Payload::Struct(fields) => fields.iter().map(|field| field.ty).collect(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        declared.into_iter().map(|payload| payload.into_iter().map(|ty| self.program.types.subst_generic(ty, args)).collect()).collect()
    }

    /// The entries of the members of a value type; a member that holds the type itself is held in a box.
    fn members(&mut self, owner: TyId, types: &[TyId]) -> Vec<TypeId> {
        types
            .iter()
            .map(|&member| if self.holds(member, owner, &mut HashSet::new()) { self.boxed(member) } else { self.ensure(member) })
            .collect()
    }

    /// Whether a value of `ty` holds a value of `target` inline.
    fn holds(&mut self, ty: TyId, target: TyId, seen: &mut HashSet<TyId>) -> bool {
        if ty == target {
            return true;
        }
        if !seen.insert(ty) {
            return false;
        }
        let kind = self.program.types.kind(ty).clone();
        let inner: Vec<TyId> = match kind {
            TyKind::Tuple(elements) => elements,
            TyKind::FixedArray(element, _) | TyKind::Optional(element) => vec![element],
            TyKind::Adt { def, args } => {
                let item = &self.program.defs[def];
                if item.is_class() {
                    return false;
                }
                if matches!(item.kind, DefKind::Enum(_)) { self.variants(def, &args).concat() } else { self.fields(def, &args) }
            }
            _ => return false,
        };
        inner.into_iter().any(|member| self.holds(member, target, seen))
    }

    /// The `Box` of a type, reserved and defined later.
    fn boxed(&mut self, ty: TyId) -> TypeId {
        if let Some(&id) = self.boxes.get(&ty) {
            return id;
        }
        let name = format!("Box<{}>", self.name(ty));
        let id = self.table.reserve_object(name);
        self.boxes.insert(ty, id);
        self.queue.push(Work::Box(id, ty));
        id
    }

    fn channel(&mut self, message: TyId) -> TypeId {
        if let Some(&id) = self.channels.get(&message) {
            return id;
        }
        let name = format!("Channel<{}>", self.name(message));
        let id = self.table.reserve_object(name);
        self.channels.insert(message, id);
        self.queue.push(Work::Channel(id, message));
        id
    }

    /// Defines the reserved object types, which may reserve more.
    fn drain(&mut self) {
        while let Some(work) = self.queue.pop() {
            match work {
                Work::Type(ty) => self.define(ty),
                Work::Box(id, ty) => {
                    let inner = self.ensure(ty);
                    self.table.define(id, Kind::Box(inner));
                }
                Work::Channel(id, message) => {
                    let array = self.program.types.array(message);
                    let array = self.ensure(array);
                    self.table.define(id, Kind::Handle(HandleKind::Channel, vec![array]));
                }
                Work::Closure(id, members) => {
                    let members = members.into_iter().map(|(ty, cell)| if cell { self.boxed(ty) } else { self.ensure(ty) }).collect();
                    self.table.define(id, Kind::Closure(members));
                }
            }
        }
    }

    fn define(&mut self, ty: TyId) {
        let id = self.ids[&ty];
        let kind = self.program.types.kind(ty).clone();
        let defined = match kind {
            TyKind::Array(element) => Kind::Array(self.ensure(element)),
            TyKind::Function(_) => Kind::Closure(Vec::new()),
            TyKind::Builtin { kind, args } => self.define_builtin(id, kind, &args),
            TyKind::Adt { def, args } => {
                if self.program.std.map == Some(def) || self.program.std.set == Some(def) {
                    self.maps.insert(id);
                }
                let fields = self.fields(def, &args);
                Kind::Class(fields.into_iter().map(|field| self.ensure(field)).collect())
            }
            _ => unreachable!("only object types wait"),
        };
        self.table.define(id, defined);
    }

    fn define_builtin(&mut self, id: TypeId, kind: BuiltinTy, args: &[TyId]) -> Kind {
        let first = args.first().copied();
        match kind {
            BuiltinTy::List => {
                let element = first.expect("a list has an element type");
                let buffer = self.program.types.array(element);
                let buffer_id = self.ensure(buffer);
                self.buffers.insert(id, buffer_id);
                Kind::List(self.ensure(element))
            }
            BuiltinTy::Task => Kind::Handle(HandleKind::Task, vec![self.ensure(first.expect("a task has a result type"))]),
            BuiltinTy::Sender | BuiltinTy::Receiver => {
                self.channel(first.expect("an end has a message type"));
                Kind::Handle(if kind == BuiltinTy::Sender { HandleKind::Sender } else { HandleKind::Receiver }, Vec::new())
            }
            BuiltinTy::Mutex => Kind::Handle(HandleKind::Mutex, vec![self.ensure(first.expect("a mutex has a value type"))]),
            BuiltinTy::Once => Kind::Handle(HandleKind::Once, vec![self.ensure(first.expect("a once has a value type"))]),
            BuiltinTy::AtomicInt => Kind::Handle(HandleKind::AtomicInt, Vec::new()),
            BuiltinTy::AtomicBool => Kind::Handle(HandleKind::AtomicBool, Vec::new()),
            BuiltinTy::PortEnd => Kind::Handle(HandleKind::Port, Vec::new()),
            BuiltinTy::Stream => Kind::Closure(Vec::new()),
            BuiltinTy::Pointer => unreachable!("Pointer is a value type"),
        }
    }
}

fn int_scalar(int: IntTy) -> Scalar {
    match int.bits() {
        8 => Scalar::I8,
        16 => Scalar::I16,
        32 => Scalar::I32,
        _ => Scalar::I64,
    }
}

#[cfg(test)]
mod tests;
