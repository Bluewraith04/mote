//! Heap objects: `NEWOBJ GETFIELD SETFIELD TYPEOF`, and recorded types: `STAMP LOADTYPE STAMPT`. Field access and `TYPEOF`
//! delegate to the leaf helpers in [`super::three_reg`] / [`super::two_reg`];
//! `NEWOBJ` allocates through the worker's mutator.

use std::ptr::NonNull;

use isa::seal;
use isa::type_term::TypeTerm;
use isa::value::{ObjectHeader, TypeDescriptor, Value, SOME_TYPE_ID};

use crate::handlers::{three_reg, two_reg};
use crate::util::{decode_r2, decode_r3, decode_ri};
use crate::machine::{ObjectStore, TypeTable};
use crate::{Runtime, TaskContext, VmStatus};

/// Allocate an object of `type_descriptors[bx]` through the worker's mutator.
pub(crate) fn newobj<M: TypeTable + ObjectStore>(
    rt: &M,
    task: &mut TaskContext,
    operands: u32,
) -> Result<VmStatus, String> {
    let (dest_reg, type_idx) = decode_ri(operands);
    let type_idx = type_idx as usize;
    let types = rt.type_descriptors();
    if type_idx >= types.len() {
        return Err(format!("Invalid type descriptor index: {}", type_idx));
    }
    let field_count = types[type_idx].slots as usize;
    let type_ptr = NonNull::from(&types[type_idx]);
    let obj_ptr = rt.alloc_object(type_ptr, field_count);
    let dest = task.call_stack.last().unwrap().base + (dest_reg as usize);
    task.registers[dest] = Value::boxed(obj_ptr);
    task.pc += 1;
    Ok(VmStatus::Running)
}

fn object_in(task: &TaskContext, reg: u8) -> Option<NonNull<ObjectHeader>> {
    let v = task.registers[task.call_stack.last().unwrap().base + reg as usize];
    v.as_object_ptr()
}

fn record(obj: NonNull<ObjectHeader>, desc: &TypeDescriptor) {
    // SAFETY: a live object's header is writable and its descriptor outlives it; `desc` lives for the program.
    unsafe {
        let header = &mut *obj.as_ptr();
        let current = header.type_ptr.as_ref();
        if current.instance.is_none() && current.id == desc.id && current.fields.len() == desc.fields.len() {
            header.type_ptr = NonNull::from(desc);
        }
    }
}

/// `STAMP rA, bx`: the object in `rA` records `type_descriptors[bx]`'s type, unless it already records one.
pub fn stamp<M: TypeTable>(rt: &M, task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    let (reg, type_idx) = decode_ri(operands);
    let desc = rt.type_descriptors().get(type_idx as usize).ok_or_else(|| format!("Invalid type descriptor index: {type_idx}"))?;
    if let Some(obj) = object_in(task, reg) {
        record(obj, desc);
    }
    task.pc += 1;
    Ok(VmStatus::Running)
}

/// `LOADTYPE rA, bx`: `rA` = the id of the type on `type_descriptors[bx]`, its register forms resolved in this frame, or `null`.
pub(crate) fn loadtype(rt: &Runtime, task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    let (reg, idx) = decode_ri(operands);
    let term = rt
        .type_descriptors
        .get(idx as usize)
        .and_then(|d| d.instance.as_ref())
        .ok_or_else(|| format!("LOADTYPE: descriptor {idx} carries no type"))?;
    let id = if term.is_pattern() {
        let base = task.call_stack.last().unwrap().base;
        let lookup = |t: &TypeTerm| match t {
            TypeTerm::Reg(r) => {
                let id = task.registers[base + *r as usize].as_int().filter(|n| *n >= 0)?;
                rt.types.get(id as u32)
            }
            TypeTerm::ArgOf(r, i) => {
                // SAFETY: a live object's descriptor outlives it.
                let desc = unsafe { object_in(task, *r)?.as_ref().type_ptr.as_ref() };
                match desc.instance.as_ref()? {
                    TypeTerm::Named(_, args) => args.get(*i as usize).cloned(),
                    _ => None,
                }
            }
            _ => None,
        };
        term.resolve(&lookup).map(|t| rt.types.intern(&t))
    } else {
        Some(rt.types.static_id(idx as usize, term))
    };
    let base = task.call_stack.last().unwrap().base;
    task.registers[base + reg as usize] = id.map_or_else(Value::null, |id| Value::int(id as i64));
    task.pc += 1;
    Ok(VmStatus::Running)
}

/// `STAMPT rA, rB`: the object in `rA` records the type whose id `rB` holds, unless it already records one.
pub(crate) fn stampt(rt: &Runtime, task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    let (a, b) = decode_r2(operands);
    let base = task.call_stack.last().unwrap().base;
    let id = task.registers[base + b as usize].as_int().filter(|n| *n >= 0);
    if let (Some(obj), Some(id)) = (object_in(task, a), id) {
        // SAFETY: a live object's descriptor outlives it.
        let template = unsafe { obj.as_ref().type_ptr.as_ref() };
        if template.instance.is_none()
            && let Some(desc) = rt.types.instance(template, id as u32) {
                record(obj, desc);
            }
    }
    task.pc += 1;
    Ok(VmStatus::Running)
}

fn value_term(v: Value) -> TypeTerm {
    let scalar = match v {
        Value::Null => Some("Null"),
        Value::Bool(_) => Some("Bool"),
        Value::Int(_) | Value::UInt(_) => Some("Int"),
        Value::Float(_) => Some("Float"),
        Value::Char(_) => Some("Char"),
        _ if v.as_short_str().is_some() => Some("String"),
        _ => None,
    };
    if let Some(name) = scalar {
        return TypeTerm::named(name);
    }
    if is_some_cell(v) && let Some(cell) = v.as_object_ptr() {
        // SAFETY: a cell has one slot.
        return TypeTerm::Nullable(Box::new(value_term(unsafe { cell.as_ref().get_field(0) })));
    }
    let Some(obj) = v.as_object_ptr() else { return TypeTerm::named("?") };
    // SAFETY: a live object's descriptor outlives it.
    let desc = unsafe { obj.as_ref().type_ptr.as_ref() };
    if let Some(term) = &desc.instance {
        return term.clone();
    }
    let name = TypeDescriptor::intrinsic_name(desc.id).or(desc.name.as_deref()).unwrap_or("?");
    TypeTerm::named(name)
}

fn type_in(rt: &Runtime, task: &TaskContext, r: u8) -> Option<TypeTerm> {
    let base = task.call_stack.last().unwrap().base;
    task.registers[base + r as usize].as_int().filter(|n| *n >= 0).and_then(|id| rt.types.get(id as u32))
}

/// `ISTYPE rA, rB, rC`: `rA` = whether the value in `rB` fits the type whose id `rC` holds.
pub(crate) fn istype(rt: &Runtime, task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    let (a, b, c) = decode_r3(operands);
    let target = type_in(rt, task, c).ok_or("an `is` test names a type argument not known here")?;
    let base = task.call_stack.last().unwrap().base;
    task.registers[base + a as usize] = if value_term(task.registers[base + b as usize]).fits(&target) { Value::true_() } else { Value::false_() };
    task.pc += 1;
    Ok(VmStatus::Running)
}

/// `ASTYPE rA, rB`: faults unless the value in `rA` fits the type whose id `rB` holds; a `null` `rB` passes.
pub(crate) fn astype(rt: &Runtime, task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    let (a, b) = decode_r2(operands);
    let base = task.call_stack.last().unwrap().base;
    if let Some(target) = type_in(rt, task, b) {
        let value = task.registers[base + a as usize];
        let actual = value_term(value);
        if !actual.fits(&target) {
            return Err(format!("type mismatch: expected `{target}`, found `{actual}`"));
        }
        if actual.has_wildcard()
            && let Some(obj) = value.as_object_ptr() {
                let id = rt.types.intern(&actual.refined(&target));
                // SAFETY: a live object's header is writable and its descriptor outlives it.
                unsafe {
                    let header = &mut *obj.as_ptr();
                    if let Some(desc) = rt.types.instance(header.type_ptr.as_ref(), id) {
                        header.type_ptr = NonNull::from(desc);
                    }
                }
            }
    }
    task.pc += 1;
    Ok(VmStatus::Running)
}

fn is_some_cell(v: Value) -> bool {
    // SAFETY: a live object's descriptor outlives it.
    v.as_object_ptr().is_some_and(|p| unsafe { p.as_ref().type_ptr.as_ref().id } == SOME_TYPE_ID)
}

/// `SOME rA, rB`: `rA` = `rB`, or a new `Some` cell around it when `rB` is `None` or a cell.
pub fn some(rt: &Runtime, task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    let (a, b) = decode_r2(operands);
    let base = task.call_stack.last().unwrap().base;
    let v = task.registers[base + b as usize];
    task.registers[base + a as usize] = if v.is_null() || is_some_cell(v) {
        let desc = rt.intrinsic_types.descriptor(SOME_TYPE_ID).expect("the Some cell is registered");
        let cell = rt.alloc_object(NonNull::from(desc), 1);
        // SAFETY: a fresh one-slot object.
        unsafe { (*cell.as_ptr()).set_field(0, v) };
        Value::boxed(cell)
    } else {
        v
    };
    task.pc += 1;
    Ok(VmStatus::Running)
}

/// `UNSOME rA, rB`: `rA` = a cell's payload, or `rB` itself.
pub(crate) fn unsome(task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    let (a, b) = decode_r2(operands);
    let base = task.call_stack.last().unwrap().base;
    let v = task.registers[base + b as usize];
    // SAFETY: a cell has one slot.
    task.registers[base + a as usize] = match v.as_object_ptr() {
        // SAFETY: a cell has one slot.
        Some(cell) if is_some_cell(v) => unsafe { cell.as_ref().get_field(0) },
        _ => v,
    };
    task.pc += 1;
    Ok(VmStatus::Running)
}

fn expect_object(v: Value) -> Result<(), String> {
    if v.as_object_ptr().is_some() {
        return Ok(());
    }
    let name = match v {
        Value::Null => "Null",
        Value::Bool(_) => "Bool",
        Value::Int(_) | Value::UInt(_) => "Int",
        Value::Float(_) => "Float",
        Value::Char(_) => "Char",
        _ => "non-object",
    };
    Err(format!("type mismatch: `{name}` has no fields"))
}

/// `rA = rB.field[c]`.
pub(crate) fn getfield(task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    let (a, b, c) = decode_r3(operands);
    let base = task.call_stack.last().unwrap().base;
    let dest = base + (a as usize);
    let obj_reg = base + (b as usize);
    let field_slot = c as usize;
    expect_object(task.registers[obj_reg])?;
    three_reg::getfield(&mut task.registers, obj_reg, field_slot, dest);
    task.pc += 1;
    Ok(VmStatus::Running)
}

/// `rA.field[b] = rC`; a runtime fault when `rA` is sealed.
pub fn setfield(
    task: &mut TaskContext,
    operands: u32,
) -> Result<VmStatus, String> {
    let (a, b, c) = decode_r3(operands);
    let base = task.call_stack.last().unwrap().base;
    let obj_reg = base + (a as usize);
    let field_slot = b as usize;
    let val_reg = base + (c as usize);
    expect_object(task.registers[obj_reg])?;
    if let Some(ptr) = task.registers[obj_reg].as_object_ptr()
        && seal::is_sealed(ptr) {
            return Err(
                "cannot write a field: this object is shared and read-only"
                    .to_string(),
            );
        }
    three_reg::setfield(&mut task.registers, obj_reg, field_slot, val_reg)?;
    task.pc += 1;
    Ok(VmStatus::Running)
}

/// `rA` = a small type code for scalars, or the object's `TypeDescriptor.id`.
pub(crate) fn type_of(task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    let (a, b) = decode_r2(operands);
    let base = task.call_stack.last().unwrap().base;
    let dest = base + (a as usize);
    let src = base + (b as usize);
    two_reg::typeof_(&mut task.registers, src, dest);
    task.pc += 1;
    Ok(VmStatus::Running)
}

fn copy_value<M: ObjectStore>(rt: &M, v: Value) -> Value {
    let Some(ptr) = v.as_object_ptr() else { return v };
    // SAFETY: a boxed pointer held in a register points at a live object header.
    let header = unsafe { ptr.as_ref() };
    let type_ptr = header.type_ptr;
    // SAFETY: descriptors are set up before any worker starts and never freed.
    let desc = unsafe { type_ptr.as_ref() };
    if !desc.is_value_type {
        return v;
    }
    let field_count = desc.slots as usize;
    let new_ptr = rt.alloc_object(type_ptr, field_count);
    for i in 0..field_count {
        // SAFETY: both objects have `field_count` slots.
        let slot = unsafe { header.get_field(i) };
        let copied = copy_value(rt, slot);
        unsafe { (*new_ptr.as_ptr()).set_field(i, copied) };
    }
    Value::boxed(new_ptr)
}

/// `rA = copy(rB)`: a plain move unless `rB` is a value-type object.
pub(crate) fn copyval<M: ObjectStore>(
    rt: &M,
    task: &mut TaskContext,
    operands: u32,
) -> Result<VmStatus, String> {
    let (a, b) = decode_r2(operands);
    let base = task.call_stack.last().unwrap().base;
    let src = task.registers[base + (b as usize)];
    task.registers[base + (a as usize)] = copy_value(rt, src);
    task.pc += 1;
    Ok(VmStatus::Running)
}
