use std::collections::HashMap;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The byte that names a `Value` kind in a `.mbc` constant; the numbers are the file format and never change.
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ValueTag {
    Null = 0,
    False = 1,
    True = 2,
    Int = 3,
    UInt = 4,
    Float = 5,
    Char = 6,
    Symbol = 7,
    NativeFn = 8,
    ObjectPtr = 10,
    InlinePayload = 11,
}

/// An 8-byte inline payload: a discriminator byte and seven data bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InlinePayload {
    pub discriminator: u8,
    pub data: [u8; 7],
}

impl InlinePayload {
    pub fn new(discriminator: u8, data: [u8; 7]) -> Self {
        Self { discriminator, data }
    }

    #[cfg(test)]
    pub fn from_bytes(discriminator: u8, slice: &[u8]) -> Self {
        let mut data = [0u8; 7];
        let copy_len = slice.len().min(7);
        data[..copy_len].copy_from_slice(&slice[..copy_len]);
        Self { discriminator, data }
    }
}

/// Reserved `TypeDescriptor::id` for the heap string object: slot 0 holds the byte length, then the UTF-8 bytes packed into 16-byte slots.
/// The collector sizes and scans strings specially by this id.
pub(crate) const STRING_TYPE_ID: u64 = 0xFFFF_FFFF_FFFF_F5D2;

/// Reserved `TypeDescriptor::id` for a function value: slot 0 holds the code-object index, then the captured values.
pub const FUNCTION_TYPE_ID: u64 = 0xFFFF_FFFF_FFFF_F0C7;

/// Slot count for a heap string of `byte_len` bytes: one length slot plus enough
/// 16-byte slots to hold the payload.
#[inline(always)]
pub fn string_slot_count(byte_len: usize) -> usize {
    1 + byte_len.div_ceil(std::mem::size_of::<Value>())
}

/// Reserved id: a `List` header — `{ len@0, backing:ptr@1 }` (capacity lives in the backing).
pub const LIST_TYPE_ID: u64 = 0xFFFF_FFFF_FFFF_F100;
/// Reserved id: a `Map` header — `{ len@0, entries:ptr@1, index:ptr@2, used@3 }`. The entries are a
/// `MAP_BACKING_TYPE_ID` run in insertion order; the index is a raw `u32` hash table over them.
pub const MAP_TYPE_ID: u64 = 0xFFFF_FFFF_FFFF_F101;
/// Reserved id: a `Set` header — `{ len@0, entries:ptr@1, index:ptr@2, used@3 }`, with a
/// `BACKING_TYPE_ID` key-only entries run (a `Map` with the value half elided).
pub const SET_TYPE_ID: u64 = 0xFFFF_FFFF_FFFF_F102;
/// Reserved id: a `Bytes` header — `{ len@0, backing:ptr@1 }`.
pub const BYTES_TYPE_ID: u64 = 0xFFFF_FFFF_FFFF_F103;
/// Reserved id: a `Task<T>` handle `{ task_id@0, status@1 (0 Pending, 1 Ok, 2 Err), result@2, waiter@3, observed@4 }`.
pub const TASK_TYPE_ID: u64 = 0xFFFF_FFFF_FFFF_F104;
/// Reserved id: a channel, which is also the `Receiver<T>`: `{ channel_id@0, capacity@1, head@2, len@3, closed@4, senders@5, taken@6, backing@7, rendezvous@8 }`.
/// The backing is a ring buffer; a rendezvous channel (`Channel(0)`) has one slot and `send` waits for its value to be taken.
pub const CHANNEL_TYPE_ID: u64 = 0xFFFF_FFFF_FFFF_F105;
/// Reserved id: a `Sender<T>` handle — `{ channel:ptr@0, closed:int@1 }`. Each handle counts as one of its channel's senders.
pub const SENDER_TYPE_ID: u64 = 0xFFFF_FFFF_FFFF_F106;
/// Reserved id: a growable pointer backing shared by `List` and `Set`.
/// Slot 0 holds the slot capacity; elements are in `1..=capacity`, unused ones `null`.
pub const BACKING_TYPE_ID: u64 = 0xFFFF_FFFF_FFFF_F110;
/// Reserved id: a growable raw-byte backing for `Bytes`. Slot 0 holds the byte capacity; bytes are packed from slot 1 and never traced.
pub const BYTES_BACKING_TYPE_ID: u64 = 0xFFFF_FFFF_FFFF_F111;
/// Reserved id: a `Map` entries backing. Slot 0 = `Value::uint(entry capacity)`; entry `i` occupies
/// slots `1 + 2i` (key) and `2 + 2i` (value). A removed entry has a `null` key. Both halves are traced `Value`s.
pub const MAP_BACKING_TYPE_ID: u64 = 0xFFFF_FFFF_FFFF_F112;
/// Reserved id: a `List` iterator cursor `{ list:ptr@0, idx:int@1 }`.
pub const LIST_ITER_TYPE_ID: u64 = 0xFFFF_FFFF_FFFF_F120;
/// Reserved id: a suspended generator. Slot 0 holds the slot count, then `state@1` (0 fresh, 1 suspended, 2 running, 3 done), `code_idx@2`, `pc@3`, `closure@4` and the saved registers from `@5`. Never `Send`.
pub const GENERATOR_TYPE_ID: u64 = 0xFFFF_FFFF_FFFF_F109;

/// The most slots an object may have to live in a region (1 KiB with its header).
pub const MAX_REGION_SLOTS: usize = 63;
/// Reserved id: a `Some` cell, slot 0 = a payload that is `None` or another cell. Any other `Some(v)` is `v` itself.
pub const SOME_TYPE_ID: u64 = 0xFFFF_FFFF_FFFF_F10A;
/// Reserved id: a `Shared<T>` cell: `{ cell_id:uint@0, version@1, holder@2 }`; `holder` is the writing task id or `null`. The version is sealed.
pub const SHARED_TYPE_ID: u64 = 0xFFFF_FFFF_FFFF_F10B;

/// Reserved id: a descriptor that only carries a `LOADTYPE` type term; never allocated.
pub const TYPE_TERM_ID: u64 = 0xFFFF_FFFF_FFFF_FE00;

/// Inclusive bounds of the reserved intrinsic-collection id block. A user
/// `struct` / `class` (codegen hands those small sequential ids) never lands here.
pub(crate) const INTRINSIC_TYPE_ID_LO: u64 = 0xFFFF_FFFF_FFFF_F100;
pub(crate) const INTRINSIC_TYPE_ID_HI: u64 = 0xFFFF_FFFF_FFFF_F1FF;

/// Whether `id` is in the reserved intrinsic-collection block.
#[inline(always)]
pub fn is_intrinsic_type_id(id: u64) -> bool {
    (INTRINSIC_TYPE_ID_LO..=INTRINSIC_TYPE_ID_HI).contains(&id)
}

/// How an object's slots are sized and which of them are traced; carried by its `TypeDescriptor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotLayout {
    /// `fields.len()` slots; references per the descriptor's pointer bits.
    Fixed,
    /// Slot 0 is a byte length, followed by that many raw bytes; never traced.
    RawBytes,
    /// Slot 0 is a count `n`, followed by `n * values_per_unit` traced `Value`s.
    ValueRun { values_per_unit: usize },
}

impl SlotLayout {
    /// Total slots of `header`'s object; `declared` is the descriptor's field count.
    ///
    /// # Safety
    /// `header` must be a live object of this layout with slot 0 initialised.
    #[inline(always)]
    pub unsafe fn slot_count(self, header: &ObjectHeader, declared: usize) -> usize {
        match self {
            SlotLayout::Fixed => declared,
            SlotLayout::RawBytes => {
                let len = unsafe { header.get_field(0) }.as_len();
                1 + len.div_ceil(std::mem::size_of::<Value>())
            }
            SlotLayout::ValueRun { values_per_unit } => {
                let n = unsafe { header.get_field(0) }.as_len();
                1 + n * values_per_unit
            }
        }
    }
}

/// Total slot count for a pointer backing holding `capacity` elements:
/// one capacity slot plus one slot per element.
#[inline(always)]
pub fn backing_slot_count(capacity: usize) -> usize {
    1 + capacity
}

/// Slots per `Map` entry: key and value.
pub const MAP_ENTRY_STRIDE: usize = 2;
/// `Map` / `Set` header slot 2: the raw `u32` index over the entries.
pub const TABLE_INDEX_SLOT: usize = 2;
/// `Map` / `Set` header slot 3: entries appended so far, removed ones included.
pub const TABLE_USED_SLOT: usize = 3;
/// `Map` / `Set` header slot count: live length, entries, index, entries used.
pub const TABLE_HEADER_SLOTS: usize = 4;

/// Total slot count for a `Map` entries backing holding `entries` entries:
/// one count slot plus `MAP_ENTRY_STRIDE` slots per entry.
#[inline(always)]
pub fn map_backing_slot_count(entries: usize) -> usize {
    1 + MAP_ENTRY_STRIDE * entries
}

/// `List` / `Bytes` header slot 0: the live element / byte count.
pub const HEADER_LEN_SLOT: usize = 0;
/// `List` / `Bytes` header slot 1: the backing object pointer.
pub const HEADER_BACKING_SLOT: usize = 1;
/// Backing slot 0: the element / byte capacity. Elements / bytes start at slot 1.
pub const BACKING_CAP_SLOT: usize = 0;
/// First element slot in a backing (element `i` lives at `BACKING_DATA_BASE + i`).
pub const BACKING_DATA_BASE: usize = 1;

/// Total slot count for a raw-byte backing holding `capacity` bytes:
/// one capacity slot plus enough 16-byte slots for the payload.
#[inline(always)]
pub fn bytes_backing_slot_count(capacity: usize) -> usize {
    1 + capacity.div_ceil(std::mem::size_of::<Value>())
}

#[repr(C)]
#[derive(Debug)]
/// The header every heap object starts with.
pub struct ObjectHeader {
    pub type_ptr: NonNull<TypeDescriptor>,
    /// Mark, pinned and old-generation bits, the `freeze` seal bit and the other flags listed at [`REGION_BIT`]. Atomic because two OS threads may reach one object.
    pub gc_state: AtomicUsize,
}

/// `gc_state` bits: `1 << 0` mark, `1 << 1` sealed ([`crate::seal::SEALED_BIT`]), `1 << 2` pinned, `1 << 3` old
/// generation, `1 << 4` free block (the collector's constants are in `gc`), `1 << 5` region object, `1 << 6` `update`
/// working copy ([`crate::seal::WORKING_BIT`]), `1 << 7` a handle object the collector releases ([`RELEASE_BIT`]); bits 8–15 hold
/// a region object's depth.
pub const REGION_BIT: usize = 1 << 5;

/// Set on a file, socket or library handle object: the collector closes the handle once no object holds it.
pub const RELEASE_BIT: usize = 1 << 7;
const REGION_DEPTH_SHIFT: usize = 8;
const REGION_DEPTH_MASK: usize = 0xFF << REGION_DEPTH_SHIFT;

/// The deepest region an object is placed in; a site nested deeper goes to the heap.
pub const MAX_REGION_DEPTH: usize = 250;

/// The `gc_state` of a new object in the region opened at `depth` (1 is the outermost).
pub fn region_state(depth: usize) -> usize {
    debug_assert!((1..=MAX_REGION_DEPTH).contains(&depth));
    REGION_BIT | (depth << REGION_DEPTH_SHIFT)
}

/// Fails when `value` points to a region object deeper than `target`: an object may point to its own depth
/// or an outer one, never an inner one, and a heap object (depth 0) never points into a region.
#[inline]
pub fn check_store(target: &ObjectHeader, value: Value) -> Result<(), String> {
    check_store_at(target.region_depth(), value)
}

/// [`check_store`] for a target of the given depth; a global or a capture is depth 0.
#[inline]
pub fn check_store_at(target_depth: usize, value: Value) -> Result<(), String> {
    let Value::ObjectPtr(ptr) = value else {
        return Ok(());
    };
    // SAFETY: a boxed pointer names a live object header.
    if unsafe { ptr.as_ref() }.region_depth() > target_depth {
        return Err("a value from an inner block would outlive it".to_string());
    }
    Ok(())
}

impl ObjectHeader {
    /// 0 for a heap object; `n` for a region object placed in the `n`th open region.
    #[inline]
    pub fn region_depth(&self) -> usize {
        let state = self.gc_state.load(Ordering::Relaxed);
        if state & REGION_BIT == 0 {
            0
        } else {
            (state & REGION_DEPTH_MASK) >> REGION_DEPTH_SHIFT
        }
    }

    /// # Safety
    /// `field_idx` is below the object's slot count.
    pub unsafe fn field_ptr(&self, field_idx: usize) -> *mut Value {
        unsafe {
            let header_ptr = self as *const ObjectHeader as *mut u8;
            let offset = std::mem::size_of::<ObjectHeader>();
            (header_ptr.add(offset) as *mut Value).add(field_idx)
        }
    }

    /// # Safety
    /// `field_idx` is below the object's slot count.
    pub unsafe fn get_field(&self, field_idx: usize) -> Value {
        unsafe { *self.field_ptr(field_idx) }
    }

    /// # Safety
    /// `field_idx` is below the object's slot count.
    pub unsafe fn set_field(&mut self, field_idx: usize, val: Value) {
        unsafe { *self.field_ptr(field_idx) = val; }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One field of a type descriptor.
pub struct FieldDescriptor {
    pub name: Option<String>,
    pub is_pointer: bool,
    /// Readable from another module through `Any`.
    pub is_pub: bool,
    /// The index of the field's first `Value` slot in the object.
    pub slot: u32,
    /// The struct embedded in the parent's slots from `slot`, instead of a pointer to it.
    pub inline: Option<Box<TypeDescriptor>>,
}

impl FieldDescriptor {
    fn at(slot: usize, name: Option<String>, is_pointer: bool) -> Self {
        Self { name, is_pointer, is_pub: true, slot: slot as u32, inline: None }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// The layout and flags shared by every object of a type.
pub struct TypeDescriptor {
    pub id: u64,
    pub fields: Vec<FieldDescriptor>,
    /// The object's size in `Value` slots.
    pub slots: u32,
    /// Bit `i` marks slot `i` as a pointer slot.
    pub pointer_bitmap: u64,
    pub is_value_type: bool,
    pub is_trivial: bool,
    /// Tuples and enum variants: `==` compares their slots.
    pub by_content: bool,
    /// The type's source name (`Point`, `Some`, …), shown by `print` and `format`; `None` for anonymous descriptors.
    pub name: Option<String>,
    pub layout: SlotLayout,
    /// The full type of a generic value built with this descriptor (`List<Int>`, `Box<String>`); `None` when not recorded.
    pub instance: Option<crate::type_term::TypeTerm>,
    /// The module that declares the type, as the driver names it; `None` for a built-in.
    pub module: Option<String>,
}

impl TypeDescriptor {
    fn build(id: u64, fields: Vec<FieldDescriptor>, pointer_bitmap: u64, is_value_type: bool, is_trivial: bool, layout: SlotLayout) -> Self {
        Self {
            id,
            slots: fields.len() as u32,
            fields,
            pointer_bitmap,
            is_value_type,
            is_trivial,
            by_content: false,
            name: None,
            layout,
            instance: None,
            module: None,
        }
    }

    fn plain_fields(names: Vec<Option<String>>) -> Vec<FieldDescriptor> {
        names.into_iter().enumerate().map(|(i, name)| FieldDescriptor::at(i, name, false)).collect()
    }

    pub fn new(id: u64, field_names: Vec<Option<String>>) -> Self {
        Self::build(id, Self::plain_fields(field_names), 0, false, false, SlotLayout::Fixed)
    }

    pub fn with_pointer_mask(id: u64, field_names: Vec<Option<String>>, pointer_bitmap: u64) -> Self {
        let fields = field_names
            .into_iter()
            .enumerate()
            .map(|(i, name)| FieldDescriptor::at(i, name, (pointer_bitmap & (1 << i)) != 0))
            .collect();
        Self::build(id, fields, pointer_bitmap, false, false, SlotLayout::Fixed)
    }

    pub fn with_field_count(id: u64, count: usize) -> Self {
        Self::build(id, Self::plain_fields(vec![None; count]), 0, false, false, SlotLayout::Fixed)
    }

    /// The shared descriptor for every heap string object; it declares no fields.
    pub fn string_type() -> Self {
        Self::build(STRING_TYPE_ID, Vec::new(), 0, false, true, SlotLayout::RawBytes)
    }

    #[inline(always)]
    pub fn is_string(&self) -> bool {
        self.id == STRING_TYPE_ID
    }

    /// A descriptor for a function value with `capture_count` captured slots; field 0 holds the code-object index.
    pub fn function_type(capture_count: usize) -> Self {
        Self::with_field_count(FUNCTION_TYPE_ID, capture_count + 1)
    }

    #[inline(always)]
    pub fn is_function(&self) -> bool {
        self.id == FUNCTION_TYPE_ID
    }

    /// Shared descriptor for an intrinsic collection header with `slot_count` fixed `Value` slots; the slot at `backing_slot`, if given, is its single pointer field.
    pub fn intrinsic_header(id: u64, slot_count: usize, backing_slot: Option<usize>) -> Self {
        let pointer_bitmap = backing_slot.map_or(0, |s| 1u64 << s);
        let fields = (0..slot_count).map(|i| FieldDescriptor::at(i, None, Some(i) == backing_slot)).collect();
        Self::build(id, fields, pointer_bitmap, false, false, SlotLayout::Fixed)
    }

    /// The `Map` / `Set` header: its entries backing and index are both pointer slots.
    pub(crate) fn table_header(id: u64) -> Self {
        let pointers = [HEADER_BACKING_SLOT, TABLE_INDEX_SLOT];
        let fields = (0..TABLE_HEADER_SLOTS).map(|i| FieldDescriptor::at(i, None, pointers.contains(&i))).collect();
        let bitmap = pointers.iter().fold(0u64, |m, s| m | 1 << s);
        Self::build(id, fields, bitmap, false, false, SlotLayout::Fixed)
    }

    /// The header of a generic intrinsic (`List`, `Map`, `Set`, `Task`, `Channel`, `Shared`); the runtime and codegen share it.
    pub fn generic_intrinsic(id: u64) -> Option<Self> {
        Some(match id {
            LIST_TYPE_ID => Self::intrinsic_header(id, 2, Some(1)),
            MAP_TYPE_ID | SET_TYPE_ID => Self::table_header(id),
            TASK_TYPE_ID => Self::intrinsic_header(id, 5, None),
            CHANNEL_TYPE_ID => Self::intrinsic_header(id, 9, None),
            SENDER_TYPE_ID => Self::intrinsic_header(id, 2, None),
            SHARED_TYPE_ID => Self::intrinsic_header(id, 3, None),
            _ => return None,
        })
    }

    /// The `Some` cell: one dynamically scanned slot, compared and printed by content.
    pub fn some_cell() -> Self {
        let mut desc = Self::intrinsic_header(SOME_TYPE_ID, 1, None);
        desc.name = Some("Some".to_string());
        desc.by_content = true;
        desc
    }

    /// The source name of a reserved std type id (`List`, `Stream`, `Bytes`, …).
    pub fn intrinsic_name(id: u64) -> Option<&'static str> {
        Some(match id {
            LIST_TYPE_ID => "List",
            MAP_TYPE_ID => "Map",
            SET_TYPE_ID => "Set",
            BYTES_TYPE_ID => "Bytes",
            CHANNEL_TYPE_ID => "Receiver",
            SENDER_TYPE_ID => "Sender",
            TASK_TYPE_ID => "Task",
            GENERATOR_TYPE_ID => "Stream",
            SHARED_TYPE_ID => "Shared",
            STRING_TYPE_ID => "String",
            _ => return None,
        })
    }

    /// Descriptor for a growable backing; its layout is the one place a backing id maps to a shape.
    pub fn intrinsic_backing(id: u64) -> Self {
        let layout = match id {
            BACKING_TYPE_ID | GENERATOR_TYPE_ID => SlotLayout::ValueRun { values_per_unit: 1 },
            MAP_BACKING_TYPE_ID => SlotLayout::ValueRun { values_per_unit: MAP_ENTRY_STRIDE },
            BYTES_BACKING_TYPE_ID => SlotLayout::RawBytes,
            _ => SlotLayout::Fixed,
        };
        Self::build(id, Vec::new(), 0, false, layout == SlotLayout::RawBytes, layout)
    }

    pub fn new_value_type(id: u64, field_names: Vec<Option<String>>, is_trivial: bool) -> Self {
        Self::build(id, Self::plain_fields(field_names), 0, true, is_trivial, SlotLayout::Fixed)
    }

    #[inline(always)]
    pub fn slot_count(&self) -> usize {
        (self.slots as usize).max(1)
    }

    #[inline(always)]
    pub fn is_field_pointer(&self, field_idx: usize) -> bool {
        if field_idx < 64 {
            (self.pointer_bitmap & (1 << field_idx)) != 0
        } else if field_idx < self.fields.len() {
            self.fields[field_idx].is_pointer
        } else {
            false
        }
    }
}

/// Fills each field's `inline` from the table: `ids[t][f]` is the table index of the struct embedded in field `f` of type `t`.
pub fn link_inline_fields(types: &mut [TypeDescriptor], ids: &[Vec<Option<u32>>]) {
    fn resolve(types: &mut [TypeDescriptor], ids: &[Vec<Option<u32>>], t: usize, done: &mut [bool]) {
        if std::mem::replace(&mut done[t], true) {
            return;
        }
        for (f, id) in ids[t].iter().enumerate() {
            let Some(n) = id.map(|n| n as usize) else { continue };
            resolve(types, ids, n, done);
            let nested = Box::new(types[n].clone());
            types[t].fields[f].inline = Some(nested);
        }
    }
    let mut done = vec![false; types.len()];
    for t in 0..types.len() {
        resolve(types, ids, t, &mut done);
    }
}

/// Centralized type registry for sharing and resolving type descriptors.
#[derive(Debug, Clone, Default)]
pub struct TypeRegistry {
    types: Vec<TypeDescriptor>,
    name_to_index: HashMap<String, usize>,
    id_to_index: HashMap<u64, usize>,
}

impl TypeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, type_desc: TypeDescriptor, name: Option<String>) -> Result<usize, String> {
        if type_desc.is_value_type && (type_desc.pointer_bitmap != 0 || type_desc.fields.iter().any(|f| f.is_pointer)) {
            return Err(format!(
                "Structural stickiness violation: Value type (id={}) cannot contain heap pointer fields",
                type_desc.id
            ));
        }

        let index = self.types.len();
        if let Some(n) = name {
            self.name_to_index.insert(n, index);
        }
        self.id_to_index.insert(type_desc.id, index);
        self.types.push(type_desc);
        Ok(index)
    }

    #[cfg(test)]
    pub(crate) fn get_by_index(&self, index: usize) -> Option<&TypeDescriptor> {
        self.types.get(index)
    }

    pub fn get_by_name(&self, name: &str) -> Option<&TypeDescriptor> {
        self.name_to_index.get(name).and_then(|&idx| self.types.get(idx))
    }

    pub fn len(&self) -> usize {
        self.types.len()
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    pub fn types(&self) -> &[TypeDescriptor] {
        &self.types
    }
}

/// A register or slot value: 16 bytes, `Copy`. `Null` is discriminant 0, so zeroed memory reads as `null`.
#[repr(u8)]
#[derive(Copy, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    Char(char),
    Symbol(u64),
    NativeFn(*const ()),
    ObjectPtr(NonNull<ObjectHeader>),
    Inline(InlinePayload),
}

// SAFETY: a `ObjectPtr` pointer is only followed under the runtime's ownership rules (sealing, region depth); a `NativeFn` is a code address.
unsafe impl Send for Value {}
unsafe impl Sync for Value {}

impl Value {
    pub fn boxed(ptr: NonNull<ObjectHeader>) -> Self {
        Value::ObjectPtr(ptr)
    }

    pub fn as_object_ptr(&self) -> Option<NonNull<ObjectHeader>> {
        match *self {
            Value::ObjectPtr(ptr) => Some(ptr),
            _ => None,
        }
    }

    pub fn int(int: i64) -> Self {
        Value::Int(int)
    }

    /// Alias for backwards compatibility
    #[inline(always)]
    pub fn small_int(int: i64) -> Self {
        Self::int(int)
    }

    pub fn uint(uint: u64) -> Self {
        Value::UInt(uint)
    }

    pub fn float(float: f64) -> Self {
        Value::Float(float)
    }

    pub fn char(c: char) -> Self {
        Value::Char(c)
    }

    pub fn symbol(id: u64) -> Self {
        Value::Symbol(id)
    }

    pub fn native_fn(f: *const ()) -> Self {
        Value::NativeFn(f)
    }

    pub fn null() -> Self {
        Value::Null
    }

    pub fn true_() -> Self {
        Value::Bool(true)
    }

    pub fn false_() -> Self {
        Value::Bool(false)
    }

    /// The `.mbc` tag of this value's kind.
    pub(crate) fn wire_tag(&self) -> ValueTag {
        match self {
            Value::Null => ValueTag::Null,
            Value::Bool(false) => ValueTag::False,
            Value::Bool(true) => ValueTag::True,
            Value::Int(_) => ValueTag::Int,
            Value::UInt(_) => ValueTag::UInt,
            Value::Float(_) => ValueTag::Float,
            Value::Char(_) => ValueTag::Char,
            Value::Symbol(_) => ValueTag::Symbol,
            Value::NativeFn(_) => ValueTag::NativeFn,
            Value::ObjectPtr(_) => ValueTag::ObjectPtr,
            Value::Inline(_) => ValueTag::InlinePayload,
        }
    }

    /// The `.mbc` form of a constant: its tag byte and eight payload bytes.
    pub fn to_wire(&self) -> (u8, [u8; 8]) {
        let bytes = match *self {
            Value::Null => [0; 8],
            Value::Bool(b) => (b as u64).to_le_bytes(),
            Value::Int(n) => n.to_le_bytes(),
            Value::UInt(n) | Value::Symbol(n) => n.to_le_bytes(),
            Value::Float(f) => f.to_le_bytes(),
            Value::Char(c) => (c as u64).to_le_bytes(),
            Value::NativeFn(p) => (p as usize as u64).to_le_bytes(),
            Value::ObjectPtr(p) => (p.as_ptr() as usize as u64).to_le_bytes(),
            Value::Inline(p) => {
                let mut raw = [0u8; 8];
                raw[0] = p.discriminator;
                raw[1..].copy_from_slice(&p.data);
                raw
            }
        };
        (self.wire_tag() as u8, bytes)
    }

    /// Reads a `.mbc` constant back; a tag or payload that names no value is an error.
    pub fn from_wire(tag: u8, raw: [u8; 8]) -> Result<Self, String> {
        let bits = u64::from_le_bytes(raw);
        Ok(match tag {
            0 => Value::Null,
            1 => Value::Bool(false),
            2 => Value::Bool(true),
            3 => Value::Int(bits as i64),
            4 => Value::UInt(bits),
            5 => Value::Float(f64::from_bits(bits)),
            6 => Value::Char(char::from_u32(bits as u32).ok_or_else(|| format!("Invalid Char constant: {}", bits as u32))?),
            7 => Value::Symbol(bits),
            8 => Value::NativeFn(bits as usize as *const ()),
            10 => Value::ObjectPtr(NonNull::new(bits as usize as *mut ObjectHeader).ok_or("Null pointer constant")?),
            11 => {
                let mut data = [0u8; 7];
                data.copy_from_slice(&raw[1..]);
                Value::Inline(InlinePayload::new(raw[0], data))
            }
            other => return Err(format!("Unknown ValueTag byte: {}", other)),
        })
    }

    #[cfg(test)]
    pub(crate) fn short_str(s: &str) -> Option<Self> {
        if s.len() > 7 {
            return None;
        }
        let mut data = [0u8; 7];
        data[..s.len()].copy_from_slice(s.as_bytes());
        Some(Value::inline_raw(s.len() as u8, data))
    }

    pub fn as_short_str(&self) -> Option<&str> {
        match self {
            Value::Inline(payload) if payload.discriminator as usize <= 7 => {
                std::str::from_utf8(&payload.data[..payload.discriminator as usize]).ok()
            }
            _ => None,
        }
    }

    /// If this is a `ObjectPtr` to a heap string object (`STRING_TYPE_ID`), its
    /// UTF-8 content as an owned `String`. Reads the byte length from slot 0 and
    /// the payload from the slots after it.
    pub fn as_heap_string(&self) -> Option<String> {
        let ptr = self.as_object_ptr()?;
        // SAFETY: a boxed pointer points at a live object, and a string's slots hold its length and bytes.
        unsafe {
            let header = ptr.as_ref();
            if header.type_ptr.as_ref().id != STRING_TYPE_ID {
                return None;
            }
            let len = header.get_field(0).as_len();
            let bytes = std::slice::from_raw_parts(header.field_ptr(1) as *const u8, len);
            Some(String::from_utf8_lossy(bytes).into_owned())
        }
    }

    /// If this is a `ObjectPtr` to a function value (`FUNCTION_TYPE_ID`), the
    /// callee code-object index stored in slot 0.
    pub fn as_function_code_idx(&self) -> Option<usize> {
        let ptr = self.as_object_ptr()?;
        // SAFETY: a boxed pointer points at a live object, and a function object has its code index in slot 0.
        unsafe {
            let header = ptr.as_ref();
            if header.type_ptr.as_ref().id != FUNCTION_TYPE_ID {
                return None;
            }
            Some(header.get_field(0).as_len())
        }
    }

    /// This value's text, whether it's stored as a 7-byte inline short string or
    /// as a heap string object.
    pub fn as_any_string(&self) -> Option<String> {
        self.as_short_str()
            .map(|s| s.to_string())
            .or_else(|| self.as_heap_string())
    }

    /// A length, count or code index the runtime keeps in a slot as a `UInt` (or a non-negative `Int`).
    #[inline(always)]
    pub fn as_len(&self) -> usize {
        match *self {
            Value::UInt(n) => n as usize,
            Value::Int(n) if n >= 0 => n as usize,
            other => panic!("expected a length, found {other:?}"),
        }
    }

    #[cfg(test)]
    pub(crate) fn inline_raw(discriminator: u8, data: [u8; 7]) -> Self {
        Value::Inline(InlinePayload::new(discriminator, data))
    }

    fn inline_eight(x: [u8; 4], y: [u8; 4]) -> Self {
        Value::Inline(InlinePayload::new(x[0], [x[1], x[2], x[3], y[0], y[1], y[2], y[3]]))
    }

    pub fn inline_pair_i32(x: i32, y: i32) -> Self {
        Self::inline_eight(x.to_le_bytes(), y.to_le_bytes())
    }

    #[cfg(test)]
    pub(crate) fn inline_pair_f32(x: f32, y: f32) -> Self {
        Self::inline_eight(x.to_le_bytes(), y.to_le_bytes())
    }

    #[cfg(test)]
    pub(crate) fn inline_bytes(discriminator: u8, slice: &[u8]) -> Self {
        Value::Inline(InlinePayload::from_bytes(discriminator, slice))
    }

    pub(crate) fn as_inline(&self) -> Option<InlinePayload> {
        match *self {
            Value::Inline(p) => Some(p),
            _ => None,
        }
    }

    fn inline_eight_bytes(&self) -> Option<[u8; 8]> {
        let p = self.as_inline()?;
        let d = p.data;
        Some([p.discriminator, d[0], d[1], d[2], d[3], d[4], d[5], d[6]])
    }

    pub fn as_inline_pair_i32(&self) -> Option<(i32, i32)> {
        let raw = self.inline_eight_bytes()?;
        Some((i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]), i32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]])))
    }

    #[cfg(test)]
    pub(crate) fn as_inline_pair_f32(&self) -> Option<(f32, f32)> {
        let raw = self.inline_eight_bytes()?;
        Some((f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]), f32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]])))
    }

    #[cfg(test)]
    pub(crate) fn extract_inline_i32(&self, byte_offset: usize) -> Option<i32> {
        let raw = self.inline_eight_bytes()?;
        let bytes = raw.get(byte_offset..byte_offset + 4)?;
        Some(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    #[cfg(test)]
    pub(crate) fn inline_discriminator(&self) -> Option<u8> {
        self.as_inline().map(|p| p.discriminator)
    }

    pub fn is_truthy(&self) -> bool {
        match *self {
            Value::Null | Value::Bool(false) => false,
            Value::Bool(true) => true,
            Value::Int(n) => n != 0,
            Value::UInt(n) => n != 0,
            Value::Float(f) => f != 0.0 && !f.is_nan(),
            Value::Char(c) => c != '\0',
            Value::Symbol(_) | Value::ObjectPtr(_) | Value::Inline(_) => true,
            Value::NativeFn(p) => !p.is_null(),
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match *self {
            Value::Int(n) => Some(n),
            _ => None,
        }
    }

    pub fn as_uint(&self) -> Option<u64> {
        match *self {
            Value::UInt(n) => Some(n),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match *self {
            Value::Float(f) => Some(f),
            _ => None,
        }
    }

    pub fn as_char(&self) -> Option<char> {
        match *self {
            Value::Char(c) => Some(c),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn as_symbol(&self) -> Option<u64> {
        match *self {
            Value::Symbol(id) => Some(id),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            Value::Bool(b) => Some(b),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Int(n) => write!(f, "Value::Int({n})"),
            Value::UInt(n) => write!(f, "Value::UInt({n})"),
            Value::Float(x) => write!(f, "Value::Float({x})"),
            Value::Char(c) => write!(f, "Value::Char({c:?})"),
            Value::Symbol(id) => write!(f, "Value::Symbol({id})"),
            Value::NativeFn(p) => write!(f, "Value::NativeFn({p:?})"),
            Value::Bool(true) => write!(f, "Value::True"),
            Value::Bool(false) => write!(f, "Value::False"),
            Value::Null => write!(f, "Value::Null"),
            Value::ObjectPtr(p) => write!(f, "Value::ObjectPtr({p:?})"),
            Value::Inline(p) => write!(f, "Value::InlinePayload({p:?})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_size() {
        assert_eq!(std::mem::size_of::<Value>(), 16);
    }

    #[test]
    fn a_plain_type_has_one_slot_per_field() {
        let td = TypeDescriptor::new(1, vec![Some("a".into()), None, Some("c".into())]);
        assert_eq!(td.slots, 3);
        assert_eq!(td.fields.iter().map(|f| f.slot).collect::<Vec<_>>(), [0, 1, 2]);
        assert_eq!(TypeDescriptor::function_type(2).slots, 3);
        assert_eq!(TypeDescriptor::intrinsic_header(LIST_TYPE_ID, 2, Some(1)).slots, 2);
    }

    #[test]
    fn an_empty_type_still_takes_one_slot() {
        let td = TypeDescriptor::new(1, vec![]);
        assert_eq!((td.slots, td.slot_count()), (0, 1));
    }

    #[test]
    fn zeroed_memory_is_null() {
        // SAFETY: an all-zero `Value` is `Null` (discriminant 0).
        let v: Value = unsafe { std::mem::zeroed() };
        assert!(v.is_null());
    }

    #[test]
    fn constants_round_trip_through_the_wire_form() {
        let values = [
            Value::Null,
            Value::Bool(true),
            Value::Bool(false),
            Value::Int(-42),
            Value::UInt(u64::MAX),
            Value::Float(-1.5),
            Value::Char('🚀'),
            Value::Symbol(7),
            Value::short_str("hi").unwrap(),
            Value::inline_pair_i32(26, -32),
        ];
        for v in values {
            let (tag, raw) = v.to_wire();
            assert_eq!(Value::from_wire(tag, raw), Ok(v), "{v:?}");
        }
    }

    #[test]
    fn wire_tags_keep_their_file_numbers() {
        assert_eq!(Value::Null.to_wire().0, 0);
        assert_eq!(Value::Bool(false).to_wire().0, 1);
        assert_eq!(Value::Bool(true).to_wire().0, 2);
        assert_eq!(Value::Int(0).to_wire().0, 3);
        assert_eq!(Value::Inline(InlinePayload::new(0, [0; 7])).to_wire().0, 11);
    }

    #[test]
    fn a_bad_wire_constant_is_an_error() {
        assert!(Value::from_wire(9, [0; 8]).is_err());
        assert!(Value::from_wire(6, 0xD800u64.to_le_bytes()).is_err(), "a surrogate is not a Char");
        assert!(Value::from_wire(10, [0; 8]).is_err(), "a null pointer is not a constant");
    }

    #[test]
    fn test_inline_value_pairs() {
        let v = Value::inline_pair_i32(26, 32);
        assert_eq!(v.wire_tag(), ValueTag::InlinePayload);
        assert_eq!(v.as_inline_pair_i32(), Some((26, 32)));
        assert_eq!(v.extract_inline_i32(0), Some(26));
        assert_eq!(v.extract_inline_i32(4), Some(32));

        let vf = Value::inline_pair_f32(1.5, -2.5);
        assert_eq!(vf.as_inline_pair_f32(), Some((1.5, -2.5)));
    }

    #[test]
    fn test_inline_raw_and_bytes() {
        let v = Value::inline_bytes(42, b"hello");
        assert_eq!(v.inline_discriminator(), Some(42));
        let payload = v.as_inline().unwrap();
        assert_eq!(&payload.data[..5], b"hello");
    }

    #[test]
    fn test_type_descriptor_pointer_bitmap() {
        let td = TypeDescriptor::with_pointer_mask(
            100,
            vec![Some("child1".into()), Some("val".into()), Some("child2".into())],
            0b101,
        );

        assert!(td.is_field_pointer(0));
        assert!(!td.is_field_pointer(1));
        assert!(td.is_field_pointer(2));
        assert!(!td.is_field_pointer(3));
    }

    #[test]
    fn test_type_registry() {
        let mut reg = TypeRegistry::new();
        let td = TypeDescriptor::new(1, vec![Some("x".into())]);
        let idx = reg.register(td.clone(), Some("Point".into())).unwrap();

        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get_by_index(idx).unwrap().id, 1);
        assert_eq!(reg.get_by_name("Point").unwrap().id, 1);
    }

    #[test]
    fn test_structural_stickiness_validation() {
        let mut reg = TypeRegistry::new();

        let valid_vt = TypeDescriptor::new_value_type(
            200,
            vec![Some("x".into()), Some("y".into()), Some("z".into())],
            true,
        );
        assert!(reg.register(valid_vt, Some("Vec3".into())).is_ok());

        let mut invalid_vt = TypeDescriptor::with_pointer_mask(201, vec![Some("data".into())], 0b1);
        invalid_vt.is_value_type = true;
        assert!(reg.register(invalid_vt, Some("BadValueType".into())).is_err());
    }

    fn header_with_state(state: usize) -> ObjectHeader {
        ObjectHeader { type_ptr: NonNull::dangling(), gc_state: AtomicUsize::new(state) }
    }

    #[test]
    fn test_region_depth_lives_in_the_header() {
        assert_eq!(header_with_state(0).region_depth(), 0, "a heap object is depth 0");
        assert_eq!(header_with_state(region_state(3)).region_depth(), 3);
        assert_eq!(header_with_state(region_state(MAX_REGION_DEPTH)).region_depth(), MAX_REGION_DEPTH);
        assert_eq!(header_with_state(region_state(5) | 0b11111).region_depth(), 5);
    }

    #[test]
    fn test_a_store_may_point_outward_or_sideways_never_inward() {
        let heap = header_with_state(0);
        let mut outer = header_with_state(region_state(1));
        let mut inner = header_with_state(region_state(2));
        let (heap_v, outer_v, inner_v) = (
            Value::boxed(NonNull::from(&heap)),
            Value::boxed(NonNull::from(&mut outer)),
            Value::boxed(NonNull::from(&mut inner)),
        );
        assert!(check_store(&heap, heap_v).is_ok());
        assert!(check_store(&outer, heap_v).is_ok(), "a region object may hold a heap pointer");
        assert!(check_store(&inner, outer_v).is_ok(), "inner may point outward");
        assert!(check_store(&inner, inner_v).is_ok(), "same depth");
        assert!(check_store(&outer, inner_v).is_err(), "outer may not point inward");
        assert!(check_store(&heap, outer_v).is_err(), "the heap may not point into a region");
        assert!(check_store(&heap, Value::small_int(1)).is_ok(), "scalars are never checked");
    }

    #[test]
    fn test_native_expanded_value_types() {
        let vi = Value::int(-42);
        assert_eq!(vi.wire_tag(), ValueTag::Int);
        assert_eq!(vi.as_int(), Some(-42));
        assert_eq!(vi.as_uint(), None);

        let vu = Value::uint(18446744073709551615);
        assert_eq!(vu.wire_tag(), ValueTag::UInt);
        assert_eq!(vu.as_uint(), Some(18446744073709551615));

        let vc = Value::char('🚀');
        assert_eq!(vc.wire_tag(), ValueTag::Char);
        assert_eq!(vc.as_char(), Some('🚀'));

        let vs = Value::symbol(123456789);
        assert_eq!(vs.wire_tag(), ValueTag::Symbol);
        assert_eq!(vs.as_symbol(), Some(123456789));

        let vstr = Value::short_str("moteVM").unwrap();
        assert_eq!(vstr.as_short_str(), Some("moteVM"));

        assert!(Value::short_str("too_long_string").is_none());
    }
}