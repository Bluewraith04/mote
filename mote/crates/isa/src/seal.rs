//! Deep-seal support for `freeze`.
//! `freeze(x)` sets a `sealed` bit on `x`'s [`ObjectHeader`] and on every object reachable from it, and never clears it.
//! `SETFIELD` checks the bit and rejects the write.

use std::ptr::NonNull;
use std::sync::atomic::Ordering;

use std::collections::HashMap;

use crate::value::{
    string_slot_count, ObjectHeader, SlotLayout, TypeDescriptor, Value, CHANNEL_TYPE_ID, FUNCTION_TYPE_ID,
    GENERATOR_TYPE_ID, SENDER_TYPE_ID, SHARED_TYPE_ID, STRING_TYPE_ID, TASK_TYPE_ID,
};

/// Bit position within `ObjectHeader.gc_state`; the collector never inspects it.
pub(crate) const SEALED_BIT: usize = 1 << 1;

/// Whether `header_ptr`'s object has been sealed by `freeze`.
#[inline(always)]
pub fn is_sealed(header_ptr: NonNull<ObjectHeader>) -> bool {
    // SAFETY: `header_ptr` points at a live object.
    unsafe { (header_ptr.as_ref().gc_state.load(Ordering::SeqCst) & SEALED_BIT) != 0 }
}

#[inline(always)]
fn seal_one(header_ptr: NonNull<ObjectHeader>) {
    // SAFETY: `header_ptr` points at a live object.
    unsafe {
        header_ptr.as_ref().gc_state.fetch_or(SEALED_BIT, Ordering::SeqCst);
    }
}

/// Marks an object of an `update` working copy; cleared when the copy is committed.
pub const WORKING_BIT: usize = 1 << 6;

fn has_bit(header_ptr: NonNull<ObjectHeader>, bit: usize) -> bool {
    // SAFETY: `header_ptr` points at a live object.
    unsafe { (header_ptr.as_ref().gc_state.load(Ordering::SeqCst) & bit) != 0 }
}

fn type_id(header_ptr: NonNull<ObjectHeader>) -> u64 {
    // SAFETY: a reachable header's `type_ptr` is a live descriptor.
    unsafe { header_ptr.as_ref().type_ptr.as_ref().id }
}

/// Seals `root` and every object transitively reachable from it, except the types shared by design.
/// The seal bit doubles as the visited marker; a no-op if `root` is already sealed.
pub fn seal_deep(root: NonNull<ObjectHeader>) {
    if is_sealed(root) || is_shared_by_design(type_id(root)) {
        return;
    }
    let mut stack = vec![root];
    seal_one(root);
    while let Some(header_ptr) = stack.pop() {
        visit_pointer_slots(header_ptr, |child| {
            if !is_sealed(child) && !is_shared_by_design(type_id(child)) {
                seal_one(child);
                stack.push(child);
            }
        });
    }
}

/// Whether an object of type `type_id` is reachable from `root` (including `root` itself).
pub fn reaches_type(root: NonNull<ObjectHeader>, type_id: u64) -> bool {
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![root];
    seen.insert(root.as_ptr() as usize);
    while let Some(header_ptr) = stack.pop() {
        // SAFETY: a reachable header's `type_ptr` is a live descriptor.
        if unsafe { header_ptr.as_ref().type_ptr.as_ref().id } == type_id {
            return true;
        }
        visit_pointer_slots(header_ptr, |child| {
            if seen.insert(child.as_ptr() as usize) {
                stack.push(child);
            }
        });
    }
    false
}

/// The objects of type `type_id` reachable from `root` through plain data, without entering the types shared by design.
pub fn collect_of_type(root: NonNull<ObjectHeader>, type_id_wanted: u64) -> Vec<NonNull<ObjectHeader>> {
    let mut found = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![root];
    seen.insert(root.as_ptr() as usize);
    while let Some(header_ptr) = stack.pop() {
        let id = type_id(header_ptr);
        if id == type_id_wanted {
            found.push(header_ptr);
        } else if !is_shared_by_design(id) {
            visit_pointer_slots(header_ptr, |child| {
                if seen.insert(child.as_ptr() as usize) {
                    stack.push(child);
                }
            });
        }
    }
    found
}

fn visit_pointer_slots(header_ptr: NonNull<ObjectHeader>, mut visit: impl FnMut(NonNull<ObjectHeader>)) {
    // SAFETY: `header_ptr` points at a live object whose slots match its descriptor.
    unsafe {
        let header = header_ptr.as_ref();
        let type_desc = header.type_ptr.as_ref();

        let mut visit_field = |i: usize| {
            let v: Value = header.get_field(i);
            if let Value::ObjectPtr(ptr) = v {
                visit(ptr);
            }
        };

        match type_desc.layout {
            SlotLayout::RawBytes => return,
            SlotLayout::ValueRun { values_per_unit } => {
                let count = header.get_field(0).as_len();
                for i in 1..=(count * values_per_unit) {
                    visit_field(i);
                }
                return;
            }
            SlotLayout::Fixed => {}
        }

        let field_count = type_desc.slots as usize;
        if type_desc.pointer_bitmap != 0 {
            let limit = field_count.min(64);
            let mut mask = if limit == 64 {
                type_desc.pointer_bitmap
            } else {
                type_desc.pointer_bitmap & ((1u64 << limit).wrapping_sub(1))
            };
            while mask != 0 {
                let idx = mask.trailing_zeros() as usize;
                visit_field(idx);
                mask &= mask - 1;
            }
            for i in 64..field_count {
                if type_desc.is_field_pointer(i) {
                    visit_field(i);
                }
            }
        } else {
            for i in 0..field_count {
                visit_field(i);
            }
        }
    }
}

fn slot_count(header_ptr: NonNull<ObjectHeader>) -> usize {
    // SAFETY: a reachable header's `type_ptr` is a live descriptor and slot 0 is its length slot.
    unsafe {
        let header = header_ptr.as_ref();
        let desc = header.type_ptr.as_ref();
        match desc.layout {
            SlotLayout::Fixed => desc.slots as usize,
            SlotLayout::ValueRun { values_per_unit } => 1 + header.get_field(0).as_len() * values_per_unit,
            SlotLayout::RawBytes if desc.id == STRING_TYPE_ID => string_slot_count(header.get_field(0).as_len()),
            SlotLayout::RawBytes => 1 + header.get_field(0).as_len().div_ceil(std::mem::size_of::<Value>()),
        }
    }
}

fn value_slots(header_ptr: NonNull<ObjectHeader>) -> std::ops::Range<usize> {
    // SAFETY: as `slot_count`.
    unsafe {
        match header_ptr.as_ref().type_ptr.as_ref().layout {
            SlotLayout::Fixed => 0..slot_count(header_ptr),
            SlotLayout::ValueRun { .. } => 1..slot_count(header_ptr),
            SlotLayout::RawBytes => 0..0,
        }
    }
}

fn is_shared_by_design(id: u64) -> bool {
    matches!(id, STRING_TYPE_ID | CHANNEL_TYPE_ID | SENDER_TYPE_ID | TASK_TYPE_ID | SHARED_TYPE_ID)
}

/// What a deep copy makes.
#[derive(Clone, Copy, PartialEq)]
pub enum CopyKind {
    /// Sealed objects; already-sealed substructure is shared.
    Sealed,
    /// Mutable objects marked with [`WORKING_BIT`], for an `update`.
    Working,
    /// Mutable objects (`.clone()`).
    Plain,
}

type Alloc<'a> = &'a mut dyn FnMut(NonNull<TypeDescriptor>, usize) -> NonNull<ObjectHeader>;

struct Copier<'a> {
    alloc: Alloc<'a>,
    kind: CopyKind,
    memo: HashMap<usize, NonNull<ObjectHeader>>,
    work: Vec<NonNull<ObjectHeader>>,
}

impl Copier<'_> {
    fn copy(&mut self, root: NonNull<ObjectHeader>) -> Result<NonNull<ObjectHeader>, String> {
        let new_root = self.copy_one(root)?;
        while let Some(dst) = self.work.pop() {
            for i in value_slots(dst) {
                // SAFETY: `i` is inside `dst`'s slots; the slot still names the source child.
                let v = unsafe { dst.as_ref().get_field(i) };
                if let Value::ObjectPtr(src) = v {
                    let child = self.copy_one(src)?;
                    unsafe { (*dst.as_ptr()).set_field(i, Value::boxed(child)) };
                }
            }
        }
        Ok(new_root)
    }

    fn copy_one(&mut self, src: NonNull<ObjectHeader>) -> Result<NonNull<ObjectHeader>, String> {
        if has_bit(src, crate::value::REGION_BIT) {
            return Err("a value from an inner block cannot leave its task".to_string());
        }
        let id = type_id(src);
        if is_shared_by_design(id) || (self.kind == CopyKind::Sealed && is_sealed(src)) {
            return Ok(src);
        }
        match (id, self.kind) {
            (GENERATOR_TYPE_ID, CopyKind::Plain) => return Err("cannot clone a generator".to_string()),
            (FUNCTION_TYPE_ID, CopyKind::Plain) => return Ok(src),
            (FUNCTION_TYPE_ID | GENERATOR_TYPE_ID, _) => return Err("cannot share a function or generator: it is not copyable".to_string()),
            _ => {}
        }
        if let Some(&dst) = self.memo.get(&(src.as_ptr() as usize)) {
            return Ok(dst);
        }
        let slots = slot_count(src);
        // SAFETY: `src` is live; `alloc` returns an object of `slots` slots, so the payloads are the same size.
        let dst = unsafe {
            let dst = (self.alloc)(src.as_ref().type_ptr, slots);
            std::ptr::copy_nonoverlapping(src.as_ref().field_ptr(0) as *const u8, dst.as_ref().field_ptr(0) as *mut u8, slots * std::mem::size_of::<Value>());
            dst
        };
        if has_bit(src, crate::value::RELEASE_BIT) {
            // SAFETY: `dst` is a live object just allocated by this copy.
            unsafe { dst.as_ref().gc_state.fetch_or(crate::value::RELEASE_BIT, Ordering::SeqCst) };
        }
        match self.kind {
            CopyKind::Sealed => seal_one(dst),
            // SAFETY: `dst` is a live object just allocated by this copy.
            CopyKind::Working => unsafe { dst.as_ref().gc_state.fetch_or(WORKING_BIT, Ordering::SeqCst); },
            CopyKind::Plain => {}
        }
        self.memo.insert(src.as_ptr() as usize, dst);
        self.work.push(dst);
        Ok(dst)
    }
}

/// Deep-copies `root` into fresh sealed objects from `alloc`, keeping shared substructure and cycles.
pub fn copy_deep(root: NonNull<ObjectHeader>, alloc: Alloc<'_>) -> Result<NonNull<ObjectHeader>, String> {
    copy_deep_as(root, alloc, CopyKind::Sealed)
}

/// Deep-copies `root` into fresh objects of `kind`.
pub fn copy_deep_as(root: NonNull<ObjectHeader>, alloc: Alloc<'_>, kind: CopyKind) -> Result<NonNull<ObjectHeader>, String> {
    Copier { alloc, kind, memo: HashMap::new(), work: Vec::new() }.copy(root)
}

/// Seals an `update`'s result: its working objects in place, anything else unsealed as a sealed copy.
pub fn claim(root: NonNull<ObjectHeader>, alloc: Alloc<'_>) -> Result<NonNull<ObjectHeader>, String> {
    let mut copier = Copier { alloc, kind: CopyKind::Sealed, memo: HashMap::new(), work: Vec::new() };
    let mut stack = Vec::new();
    let resolve = |obj: NonNull<ObjectHeader>, copier: &mut Copier<'_>, stack: &mut Vec<NonNull<ObjectHeader>>| {
        if is_sealed(obj) || is_shared_by_design(type_id(obj)) {
            return Ok(obj);
        }
        if has_bit(obj, WORKING_BIT) {
            // SAFETY: `obj` is a live object.
            unsafe { obj.as_ref().gc_state.fetch_and(!WORKING_BIT, Ordering::SeqCst) };
            seal_one(obj);
            stack.push(obj);
            return Ok(obj);
        }
        copier.copy(obj)
    };
    let new_root = resolve(root, &mut copier, &mut stack)?;
    while let Some(obj) = stack.pop() {
        for i in value_slots(obj) {
            // SAFETY: `i` is inside `obj`'s slots.
            let v = unsafe { obj.as_ref().get_field(i) };
            if let Value::ObjectPtr(child) = v {
                let got = resolve(child, &mut copier, &mut stack)?;
                if got != child {
                    unsafe { (*obj.as_ptr()).set_field(i, Value::boxed(got)) };
                }
            }
        }
    }
    Ok(new_root)
}
