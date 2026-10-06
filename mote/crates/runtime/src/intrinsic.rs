//! The allocation seam handed to natives as a [`NativeCtx`]: allocate an intrinsic header or backing, read a slot, write a slot with the barrier. It exposes no registers, call stack or scheduler.
//! A collection runs only at a safepoint, never inside a native, so a native may hold raw pointers across its allocations without rooting them.

use std::ptr::NonNull;

use isa::value::{
    backing_slot_count, bytes_backing_slot_count, is_intrinsic_type_id,
    map_backing_slot_count, string_slot_count, ObjectHeader,
    SlotLayout, TypeDescriptor, Value, BACKING_TYPE_ID, BYTES_BACKING_TYPE_ID, BYTES_TYPE_ID, CHANNEL_TYPE_ID,
    LIST_ITER_TYPE_ID, LIST_TYPE_ID, MAP_BACKING_TYPE_ID, MAP_TYPE_ID, SENDER_TYPE_ID, SHARED_TYPE_ID,
    SET_TYPE_ID, SOME_TYPE_ID, TASK_TYPE_ID, GENERATOR_TYPE_ID,
};

use contracts::{Heap, Mutator, NativeCtx};

const SEALED_WRITE: &str = "cannot change a collection: it is shared and read-only";

/// The shared `TypeDescriptor`s for the reserved intrinsic-collection ids, boxed so their addresses are stable.
#[derive(Debug)]
pub struct IntrinsicTypeTable {
    list: Box<TypeDescriptor>,
    list_iter: Box<TypeDescriptor>,
    map: Box<TypeDescriptor>,
    set: Box<TypeDescriptor>,
    bytes: Box<TypeDescriptor>,
    backing: Box<TypeDescriptor>,
    bytes_backing: Box<TypeDescriptor>,
    map_backing: Box<TypeDescriptor>,
    task: Box<TypeDescriptor>,
    channel: Box<TypeDescriptor>,
    sender: Box<TypeDescriptor>,
    shared: Box<TypeDescriptor>,
    generator: Box<TypeDescriptor>,
    some: Box<TypeDescriptor>,
}

impl Default for IntrinsicTypeTable {
    fn default() -> Self {
        let generic = |id| Box::new(TypeDescriptor::generic_intrinsic(id).expect("a generic intrinsic id"));
        Self {
            list: generic(LIST_TYPE_ID),
            list_iter: Box::new(TypeDescriptor::intrinsic_header(LIST_ITER_TYPE_ID, 2, Some(0))),
            map: generic(MAP_TYPE_ID),
            set: generic(SET_TYPE_ID),
            bytes: Box::new(TypeDescriptor::intrinsic_header(BYTES_TYPE_ID, 2, Some(1))),
            backing: Box::new(TypeDescriptor::intrinsic_backing(BACKING_TYPE_ID)),
            bytes_backing: Box::new(TypeDescriptor::intrinsic_backing(BYTES_BACKING_TYPE_ID)),
            map_backing: Box::new(TypeDescriptor::intrinsic_backing(MAP_BACKING_TYPE_ID)),
            task: generic(TASK_TYPE_ID),
            channel: generic(CHANNEL_TYPE_ID),
            sender: generic(SENDER_TYPE_ID),
            shared: generic(SHARED_TYPE_ID),
            generator: Box::new(TypeDescriptor::intrinsic_backing(GENERATOR_TYPE_ID)),
            some: Box::new(TypeDescriptor::some_cell()),
        }
    }
}

impl IntrinsicTypeTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn descriptor(&self, id: u64) -> Option<&TypeDescriptor> {
        Some(match id {
            LIST_TYPE_ID => &self.list,
            LIST_ITER_TYPE_ID => &self.list_iter,
            MAP_TYPE_ID => &self.map,
            SET_TYPE_ID => &self.set,
            BYTES_TYPE_ID => &self.bytes,
            BACKING_TYPE_ID => &self.backing,
            BYTES_BACKING_TYPE_ID => &self.bytes_backing,
            MAP_BACKING_TYPE_ID => &self.map_backing,
            TASK_TYPE_ID => &self.task,
            CHANNEL_TYPE_ID => &self.channel,
            SENDER_TYPE_ID => &self.sender,
            SHARED_TYPE_ID => &self.shared,
            GENERATOR_TYPE_ID => &self.generator,
            SOME_TYPE_ID => &self.some,
            _ => return None,
        })
    }
}

/// The [`NativeCtx`] over a running task's heap-owning fields, built per `CALLNATIVE`.
pub struct VmIntrinsicCtx<'a> {
    pub mutator: &'a mut dyn Mutator,
    pub heap: &'a dyn Heap,
    pub types: &'a IntrinsicTypeTable,
    /// The shared heap-string descriptor, used by `alloc_string`.
    pub string_type: &'a TypeDescriptor,
    /// The installed [`contracts::Platform`], if any.
    pub platform: Option<&'a dyn contracts::Platform>,
}

pub(crate) fn alloc_intrinsic_object(
    mutator: &mut dyn Mutator,
    desc: &TypeDescriptor,
    slot_count: usize,
) -> NonNull<ObjectHeader> {
    mutator.alloc(NonNull::from(desc), slot_count)
}

pub(crate) fn alloc_string_object(
    mutator: &mut dyn Mutator,
    string_type: &TypeDescriptor,
    bytes: &[u8],
) -> Value {
    let slots = string_slot_count(bytes.len());
    let mut obj = alloc_intrinsic_object(mutator, string_type, slots);
    // SAFETY: freshly allocated with `slots` covering the length slot + the
    // packed byte payload (the `NEWSTR` layout). No collection mid-allocation.
    unsafe {
        let header = obj.as_mut();
        header.set_field(0, Value::uint(bytes.len() as u64));
        if !bytes.is_empty() {
            let dst = header.field_ptr(1) as *mut u8;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
        }
    }
    Value::boxed(obj)
}

impl VmIntrinsicCtx<'_> {
    fn alloc_raw(
        &mut self,
        type_id: u64,
        slot_count: usize,
    ) -> Result<NonNull<ObjectHeader>, String> {
        let desc = self.types.descriptor(type_id).ok_or_else(|| {
            format!("intrinsic alloc: id {type_id:#018x} is not a reserved intrinsic type")
        })?;
        Ok(alloc_intrinsic_object(self.mutator, desc, slot_count))
    }

    fn alloc_capacity_backing(&mut self, type_id: u64, slots: usize, capacity: usize) -> Result<Value, String> {
        let mut obj = self.alloc_raw(type_id, slots)?;
        // SAFETY: freshly allocated with `slots >= 1`; slot 0 is the capacity
        // slot. No collection can run before this native returns (module docs).
        unsafe { obj.as_mut().set_field(0, Value::uint(capacity as u64)) };
        if self.types.descriptor(type_id).is_some_and(|d| d.layout == SlotLayout::RawBytes) {
            unsafe {
                let base = obj.as_ref().field_ptr(1) as *mut u8;
                std::ptr::write_bytes(base, 0, capacity);
            }
        }
        Ok(Value::boxed(obj))
    }
}

impl NativeCtx for VmIntrinsicCtx<'_> {
    fn platform(&self) -> Result<&dyn contracts::Platform, String> {
        self.platform.ok_or_else(|| "no platform installed".to_string())
    }

    fn on_home_thread(&self) -> bool {
        crate::sched::on_home_thread()
    }

    fn alloc_header(&mut self, type_id: u64, slot_count: usize) -> Result<Value, String> {
        if !is_intrinsic_type_id(type_id) {
            return Err(format!(
                "alloc_header: id {type_id:#018x} is outside the intrinsic type range"
            ));
        }
        Ok(Value::boxed(self.alloc_raw(type_id, slot_count)?))
    }

    fn alloc_backing(&mut self, capacity: usize) -> Result<Value, String> {
        self.alloc_capacity_backing(BACKING_TYPE_ID, backing_slot_count(capacity), capacity)
    }

    fn alloc_bytes_backing(&mut self, capacity: usize) -> Result<Value, String> {
        self.alloc_capacity_backing(
            BYTES_BACKING_TYPE_ID,
            bytes_backing_slot_count(capacity),
            capacity,
        )
    }

    fn alloc_map_backing(&mut self, buckets: usize) -> Result<Value, String> {
        self.alloc_capacity_backing(
            MAP_BACKING_TYPE_ID,
            map_backing_slot_count(buckets),
            buckets,
        )
    }

    fn release_backing(&mut self, backing: Value) {
        if let Some(ptr) = backing.as_object_ptr() {
            self.mutator.release(ptr);
        }
    }

    fn release_on_collect(&mut self, obj: Value, kind: u8) -> Result<(), String> {
        let ptr = obj.as_object_ptr().ok_or_else(|| "release_on_collect: not a heap object".to_string())?;
        // SAFETY: a boxed value handed to a native points at a live object with at least one slot.
        let key = unsafe { ptr.as_ref().get_field(0) }.as_int().ok_or("release_on_collect: slot 0 is not an id")?;
        unsafe { ptr.as_ref().gc_state.fetch_or(isa::value::RELEASE_BIT, std::sync::atomic::Ordering::SeqCst) };
        self.mutator.release_on_collect(ptr, kind, key);
        Ok(())
    }

    fn bytes_ptr(&self, obj: Value) -> Result<*mut u8, String> {
        let ptr = obj
            .as_object_ptr()
            .ok_or_else(|| "bytes_ptr: receiver is not a heap object".to_string())?;
        // SAFETY: as `read_bytes`/`write_bytes` — slot 0 is the byte capacity,
        // the payload starts at `field_ptr(1)`.
        unsafe { Ok(ptr.as_ref().field_ptr(1) as *mut u8) }
    }

    fn get_slot(&self, obj: Value, i: usize) -> Result<Value, String> {
        let ptr = obj
            .as_object_ptr()
            .ok_or_else(|| "get_slot: receiver is not a heap object".to_string())?;
        // SAFETY: `ptr` is a live boxed object; slot bounds are the caller's contract.
        Ok(unsafe { ptr.as_ref().get_field(i) })
    }

    fn set_slot(&mut self, obj: Value, i: usize, val: Value) -> Result<(), String> {
        let mut ptr = obj
            .as_object_ptr()
            .ok_or_else(|| "set_slot: receiver is not a heap object".to_string())?;
        if isa::seal::is_sealed(ptr) {
            return Err(SEALED_WRITE.to_string());
        }
        // SAFETY: as `get_slot`.
        isa::value::check_store(unsafe { ptr.as_ref() }, val)?;
        unsafe { ptr.as_mut().set_field(i, val) };
        Ok(())
    }

    fn read_bytes(&self, backing: Value, start: usize, len: usize) -> Result<Vec<u8>, String> {
        let ptr = backing
            .as_object_ptr()
            .ok_or_else(|| "read_bytes: receiver is not a heap object".to_string())?;
        // SAFETY: `ptr` is a live byte backing; slot 0 is its byte capacity and
        // the payload starts at `field_ptr(1)` (the heap-string layout).
        unsafe {
            let header = ptr.as_ref();
            let cap = header.get_field(0).as_uint().unwrap_or(0) as usize;
            if start.checked_add(len).is_none_or(|end| end > cap) {
                return Err(format!("read_bytes: {start}+{len} exceeds capacity {cap}"));
            }
            let base = header.field_ptr(1) as *const u8;
            Ok(std::slice::from_raw_parts(base.add(start), len).to_vec())
        }
    }

    fn write_bytes(&mut self, backing: Value, start: usize, src: &[u8]) -> Result<(), String> {
        let ptr = backing
            .as_object_ptr()
            .ok_or_else(|| "write_bytes: receiver is not a heap object".to_string())?;
        if isa::seal::is_sealed(ptr) {
            return Err(SEALED_WRITE.to_string());
        }
        // SAFETY: as `read_bytes`.
        unsafe {
            let header = ptr.as_ref();
            let cap = header.get_field(0).as_uint().unwrap_or(0) as usize;
            if start.checked_add(src.len()).is_none_or(|end| end > cap) {
                return Err(format!(
                    "write_bytes: {start}+{} exceeds capacity {cap}",
                    src.len()
                ));
            }
            let base = header.field_ptr(1) as *mut u8;
            std::ptr::copy_nonoverlapping(src.as_ptr(), base.add(start), src.len());
        }
        Ok(())
    }

    fn alloc_string(&mut self, bytes: &[u8]) -> Result<Value, String> {
        Ok(alloc_string_object(self.mutator, self.string_type, bytes))
    }

    fn clone_value(&mut self, obj: Value) -> Result<Value, String> {
        let Some(ptr) = obj.as_object_ptr() else { return Ok(obj) };
        let mutator = &mut *self.mutator;
        let copy = isa::seal::copy_deep_as(ptr, &mut |type_ptr, slots| mutator.alloc(type_ptr, slots), isa::seal::CopyKind::Plain)?;
        Ok(Value::boxed(copy))
    }

    fn alloc_struct(&mut self, desc: &TypeDescriptor) -> Result<Value, String> {
        Ok(Value::boxed(alloc_intrinsic_object(self.mutator, desc, desc.slot_count())))
    }
}
