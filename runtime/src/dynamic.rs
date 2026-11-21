//! The `Dyn` operations of compiled code: a value of a described type, read and written by position.

use mote_abi::{Described, Kind, Meta, Reflected, TaskContext, TypeId, TypeTable, BOX_VALUE_OFFSET, ELEMENTS_OFFSET, LENGTH_OFFSET, LIST_BUFFER_OFFSET, LIST_CAPACITY_OFFSET};

use crate::helpers::{new_object, new_string, set_word};
use crate::show::Show;
use crate::walk::Walk;
use crate::Runtime;

/// The codes `mote_dyn` is given.
pub mod op {
    /// The number of parts of the `Dyn` in `a`.
    pub const LEN: u32 = 0;
    /// The position of the variant the `Dyn` in `a` holds.
    pub const TAG: u32 = 1;
    /// Part `b` of the `Dyn` in `a`.
    pub const GET: u32 = 2;
    /// Replaces part `b` of the `Dyn` in `a` with the part in `c`.
    pub const SET: u32 = 3;
    /// The text of the `Dyn` in `a`, bare when `b` is not 0.
    pub const SHOW: u32 = 4;
    /// Builds a value of type number `a` and variant `b` from the list of parts in `c` at the address `d`.
    pub const BUILD: u32 = 5;
    /// A new `Dyn` of type number `a` and variant `b` from the list of parts in `c`.
    pub const MAKE: u32 = 6;
}

/// A position or variant outside what the value has.
pub const RANGE: u64 = 1;

/// The heap is full.
pub const FULL: u64 = 2;

/// The symbol of the `Dyn` operations.
pub const DYN: &str = "mote_dyn";

/// Reads a tag of the given width.
///
/// # Safety
/// `at` addresses `bytes` readable bytes.
unsafe fn read(at: *const u8, bytes: u32) -> u64 {
    // SAFETY: the caller's promise.
    unsafe {
        match bytes {
            0 => 0,
            1 => u64::from(*at),
            2 => u64::from((at as *const u16).read_unaligned()),
            4 => u64::from((at as *const u32).read_unaligned()),
            _ => (at as *const u64).read_unaligned(),
        }
    }
}

/// Reads a word.
///
/// # Safety
/// `at` addresses a readable word.
unsafe fn word(at: usize) -> u64 {
    // SAFETY: the caller's promise.
    unsafe { (at as *const u64).read_unaligned() }
}

struct Dynamic<'a> {
    table: &'a TypeTable,
    meta: &'a Meta,
    dyn_type: TypeId,
    context: *mut TaskContext,
}

/// A part of a value: the number `std.meta` knows its type by and where it is.
type Part = (u32, *mut u8);

impl Dynamic<'_> {
    /// The offset of a field of the `Dyn` class: the type number, then the box.
    fn field(&self, index: usize) -> usize {
        self.table.get(self.dyn_type).object.as_ref().expect("Dyn is a class").fields[index].offset as usize
    }

    fn stride(&self, ty: TypeId) -> usize {
        let layout = self.table.layout(ty);
        mote_abi::align_up(layout.size, layout.align) as usize
    }

    /// The type a `Dyn` holds and the address of the value.
    ///
    /// # Safety
    /// `dynamic` is a live `Dyn` made by `dyn_of`, `get` or `build`.
    unsafe fn held(&self, dynamic: u64) -> (&Described, *mut u8) {
        // SAFETY: the caller's promise: a `Dyn` holds the type number and a reference to a box of that type.
        unsafe {
            let number = word(dynamic as usize + self.field(0)) as usize;
            let boxed = word(dynamic as usize + self.field(1)) as usize;
            (&self.meta.described[number], (boxed + BOX_VALUE_OFFSET as usize) as *mut u8)
        }
    }

    /// The parts of the value of the type at the address.
    ///
    /// # Safety
    /// The address holds a value of the type.
    unsafe fn parts(&self, entry: &Described, at: *mut u8) -> Vec<Part> {
        let info = self.table.get(entry.ty);
        // SAFETY: the caller's promise, and each branch reads only what the layout of the kind holds.
        unsafe {
            match (entry.kind, &info.kind) {
                (Reflected::Struct, Kind::Struct(_)) => info.fields.iter().zip(&entry.fields).map(|(field, &number)| (number, at.add(field.offset as usize))).collect(),
                (Reflected::Tuple, Kind::Struct(_)) => info.fields.iter().zip(&entry.args).map(|(field, &number)| (number, at.add(field.offset as usize))).collect(),
                (Reflected::Class, Kind::Class(_)) => {
                    let object = word(at as usize) as usize;
                    info.object.as_ref().expect("a class has fields").fields.iter().zip(&entry.fields).map(|(field, &number)| (number, (object + field.offset as usize) as *mut u8)).collect()
                }
                (Reflected::Enum, Kind::Enum(_)) => {
                    let tag = read(at, info.tag_size) as usize;
                    info.variants[tag].fields.iter().zip(&entry.variants[tag]).map(|(field, &number)| (number, at.add(field.offset as usize))).collect()
                }
                (Reflected::Array, Kind::FixedArray(element, count)) => {
                    let stride = self.stride(*element);
                    (0..*count as usize).map(|index| (entry.args[0], at.add(index * stride))).collect()
                }
                (Reflected::Array, Kind::Array(element)) => {
                    let object = word(at as usize) as usize;
                    let (stride, length) = (self.stride(*element), word(object + LENGTH_OFFSET as usize) as usize);
                    (0..length).map(|index| (entry.args[0], (object + ELEMENTS_OFFSET as usize + index * stride) as *mut u8)).collect()
                }
                (Reflected::List, Kind::List(element)) => {
                    let object = word(at as usize) as usize;
                    let buffer = word(object + LIST_BUFFER_OFFSET as usize) as usize;
                    let (stride, length) = (self.stride(*element), word(object + LENGTH_OFFSET as usize) as usize);
                    (0..length).map(|index| (entry.args[0], (buffer + ELEMENTS_OFFSET as usize + index * stride) as *mut u8)).collect()
                }
                (Reflected::Map, Kind::Class(_)) => {
                    let (key, value) = (entry.args[0], entry.args[1]);
                    self.map_entries(entry.ty, word(at as usize) as usize).into_iter().flat_map(|(from, to)| [(key, from), (value, to)]).collect()
                }
                (Reflected::Set, Kind::Class(_)) => {
                    let object = word(at as usize) as usize;
                    let members = info.object.as_ref().expect("a set has fields").fields[0];
                    let inner = word(object + members.offset as usize) as usize;
                    self.map_entries(members.ty, inner).into_iter().map(|(from, _)| (entry.args[0], from)).collect()
                }
                (Reflected::Optional, Kind::Optional(_)) if info.is_reference() => {
                    if word(at as usize) == 0 { Vec::new() } else { vec![(entry.args[0], at)] }
                }
                (Reflected::Optional, Kind::Optional(_)) => {
                    if word(at as usize) == 0 {
                        Vec::new()
                    } else {
                        vec![(entry.args[0], at.add(info.variants[1].fields[0].offset as usize))]
                    }
                }
                _ => Vec::new(),
            }
        }
    }

    /// The places of the key and the value of each live entry of the map object of the type, in the order they were set.
    ///
    /// # Safety
    /// `object` is a live map of the type.
    unsafe fn map_entries(&self, map: TypeId, object: usize) -> Vec<(*mut u8, *mut u8)> {
        let fields = &self.table.get(map).object.as_ref().expect("a map is a class").fields;
        let (Kind::List(key), Kind::List(value)) = (&self.table.get(fields[0].ty).kind, &self.table.get(fields[1].ty).kind) else { return Vec::new() };
        let (key_stride, value_stride) = (self.stride(*key), self.stride(*value));
        // SAFETY: the caller's promise: a map holds the lists `held_keys`, `held_values` and `hashes`, in that order.
        unsafe {
            let list = |index: usize| word(object + fields[index].offset as usize) as usize;
            let (keys, values, hashes) = (list(0), list(1), list(2));
            let elements = |list: usize| word(list + LIST_BUFFER_OFFSET as usize) as usize + ELEMENTS_OFFSET as usize;
            let (keys_at, values_at, hashes_at) = (elements(keys), elements(values), elements(hashes));
            let count = word(keys + LENGTH_OFFSET as usize) as usize;
            (0..count)
                .filter(|&index| word(hashes_at + index * 8) as i64 != -1)
                .map(|index| ((keys_at + index * key_stride) as *mut u8, (values_at + index * value_stride) as *mut u8))
                .collect()
        }
    }

    /// A new list object of the list type with `count` zeroed elements and its length and capacity set, or 0 when the heap is full or the type has no buffer.
    ///
    /// # Safety
    /// `list` is a list type.
    unsafe fn new_list(&self, list: TypeId, count: usize) -> u64 {
        let Kind::List(element) = self.table.get(list).kind else { return 0 };
        let Some(array) = self.table.iter().find(|(_, other)| other.kind == Kind::Array(element)).map(|(id, _)| id) else { return 0 };
        // SAFETY: the context is attached; a list holds its length, capacity and buffer in the fields set here.
        unsafe {
            let buffer = new_object(self.context, array.raw(), count as u64);
            let object = new_object(self.context, list.raw(), 0);
            if buffer == 0 || object == 0 {
                return 0;
            }
            set_word(object, LENGTH_OFFSET, count as u64);
            set_word(object, LIST_CAPACITY_OFFSET, count as u64);
            set_word(object, LIST_BUFFER_OFFSET, buffer);
            object
        }
    }

    /// A new map object of the map type holding the pairs, a later pair replacing an earlier one with an equal key; 0 when the heap is full.
    ///
    /// # Safety
    /// Each pair is a key and a value of the map's types; a value of a size of 0 is never read.
    unsafe fn new_map(&self, map: TypeId, pairs: &[(*mut u8, *mut u8)]) -> u64 {
        let fields = &self.table.get(map).object.as_ref().expect("a map is a class").fields;
        let (Kind::List(key), Kind::List(value)) = (&self.table.get(fields[0].ty).kind, &self.table.get(fields[1].ty).kind) else { return 0 };
        let walk = Walk::new(self.table, &self.meta.shown, true);
        // SAFETY: the caller's promise, and the lists and the array allocated here have the layout the map's fields name.
        unsafe {
            let mut entries: Vec<(i64, *mut u8, *mut u8)> = Vec::new();
            let mut seen: std::collections::HashMap<i64, Vec<usize>> = std::collections::HashMap::new();
            for &(from, to) in pairs {
                let hash = walk.hash_of(*key, from) as i64;
                let same = seen.entry(hash).or_default();
                match same.iter().find(|&&index| walk.same(*key, entries[index].1, from)) {
                    Some(&index) => entries[index].2 = to,
                    None => {
                        same.push(entries.len());
                        entries.push((hash, from, to));
                    }
                }
            }
            let (keys, values, hashes) = (self.new_list(fields[0].ty, entries.len()), self.new_list(fields[1].ty, entries.len()), self.new_list(fields[2].ty, entries.len()));
            let mut size = 8usize;
            while size * 3 < (entries.len() + 1) * 8 {
                size *= 2;
            }
            let slots = new_object(self.context, fields[3].ty.raw(), size as u64);
            let object = new_object(self.context, map.raw(), 0);
            if keys == 0 || values == 0 || hashes == 0 || slots == 0 || object == 0 {
                return 0;
            }
            let elements = |list: u64| word(list as usize + LIST_BUFFER_OFFSET as usize) as usize + ELEMENTS_OFFSET as usize;
            let (keys_at, values_at, hashes_at, slots_at) = (elements(keys), elements(values), elements(hashes), slots as usize + ELEMENTS_OFFSET as usize);
            let (key_stride, value_stride) = (self.stride(*key), self.stride(*value));
            for place in 0..size {
                *((slots_at + place * 8) as *mut i64) = -1;
            }
            for (index, &(hash, from, to)) in entries.iter().enumerate() {
                std::ptr::copy_nonoverlapping(from, (keys_at + index * key_stride) as *mut u8, self.table.layout(*key).size as usize);
                std::ptr::copy_nonoverlapping(to, (values_at + index * value_stride) as *mut u8, self.table.layout(*value).size as usize);
                *((hashes_at + index * 8) as *mut i64) = hash;
                let mut at = hash as u64 as usize & (size - 1);
                while *((slots_at + at * 8) as *const i64) != -1 {
                    at = (at + 1) & (size - 1);
                }
                *((slots_at + at * 8) as *mut i64) = index as i64;
            }
            for (field, list) in [(0, keys), (1, values), (2, hashes)] {
                set_word(object, fields[field].offset, list);
            }
            set_word(object, fields[3].offset, slots);
            set_word(object, fields[4].offset, entries.len() as u64);
            object
        }
    }

    /// A new `Dyn` holding a copy of the value of the type number at the address, or 0 when the heap is full.
    ///
    /// # Safety
    /// The address holds a value of that type.
    unsafe fn make(&self, number: u32, from: *const u8) -> u64 {
        let entry = &self.meta.described[number as usize];
        let size = self.table.layout(entry.ty).size as usize;
        // SAFETY: the context is attached; the box holds its value from `BOX_VALUE_OFFSET`, and the `Dyn` its two fields.
        unsafe {
            let boxed = new_object(self.context, entry.boxed.raw(), 0);
            if boxed == 0 {
                return 0;
            }
            std::ptr::copy_nonoverlapping(from, (boxed as usize + BOX_VALUE_OFFSET as usize) as *mut u8, size);
            let dynamic = new_object(self.context, self.dyn_type.raw(), 0);
            if dynamic == 0 {
                return 0;
            }
            *((dynamic as usize + self.field(0)) as *mut u64) = u64::from(number);
            *((dynamic as usize + self.field(1)) as *mut u64) = boxed;
            dynamic
        }
    }

    /// The type and place of each `Dyn` of a list.
    ///
    /// # Safety
    /// `parts` is a live list of `Dyn`.
    unsafe fn given(&self, parts: u64) -> Vec<(&Described, *mut u8)> {
        // SAFETY: the caller's promise.
        unsafe {
            let length = word(parts as usize + LENGTH_OFFSET as usize) as usize;
            let buffer = word(parts as usize + LIST_BUFFER_OFFSET as usize) as usize;
            (0..length).map(|index| self.held(word(buffer + ELEMENTS_OFFSET as usize + index * 8))).collect()
        }
    }

    /// A new `Dyn` of the type number: a struct, class, tuple or enum as `build` makes it, a list or array of the parts, or an optional of none or one part; or says why not.
    ///
    /// # Safety
    /// `parts` is a live list of `Dyn` whose types are the ones the type is made of.
    unsafe fn make_dyn(&self, number: u64, tag: u64, parts: u64) -> u64 {
        let Some(entry) = self.meta.described.get(number as usize) else { return RANGE };
        let info = self.table.get(entry.ty);
        // SAFETY: the caller's promise, and the layout of the kind says where each part goes.
        unsafe {
            let held = |value: *const u8| match self.make(number as u32, value) {
                0 => FULL,
                made => made,
            };
            let room = |size: usize| vec![0u128; size.div_ceil(16).max(1)];
            match (entry.kind, &info.kind) {
                (Reflected::Struct | Reflected::Tuple | Reflected::Class | Reflected::Enum, _) => {
                    let mut room = room(self.table.layout(entry.ty).size as usize);
                    let out = room.as_mut_ptr() as *mut u8;
                    match self.build(number, tag, parts, out) {
                        0 => held(out),
                        failed => failed,
                    }
                }
                (Reflected::List, Kind::List(element)) => {
                    let given = self.given(parts);
                    let list = self.new_list(entry.ty, given.len());
                    if list == 0 {
                        return FULL;
                    }
                    self.copy_parts(*element, &given, word(list as usize + LIST_BUFFER_OFFSET as usize));
                    held(&list as *const u64 as *const u8)
                }
                (Reflected::Map, Kind::Class(_)) => {
                    let given = self.given(parts);
                    if !given.len().is_multiple_of(2) {
                        return RANGE;
                    }
                    let pairs: Vec<_> = given.chunks(2).map(|pair| (pair[0].1, pair[1].1)).collect();
                    match self.new_map(entry.ty, &pairs) {
                        0 => FULL,
                        map => held(&map as *const u64 as *const u8),
                    }
                }
                (Reflected::Set, Kind::Class(_)) => {
                    let given = self.given(parts);
                    let members = info.object.as_ref().expect("a set has fields").fields[0];
                    let none = 0u64;
                    let pairs: Vec<_> = given.iter().map(|&(_, from)| (from, &none as *const u64 as *mut u8)).collect();
                    let inner = self.new_map(members.ty, &pairs);
                    let set = new_object(self.context, entry.ty.raw(), 0);
                    if inner == 0 || set == 0 {
                        return FULL;
                    }
                    set_word(set, members.offset, inner);
                    held(&set as *const u64 as *const u8)
                }
                (Reflected::Array, Kind::Array(element)) => {
                    let given = self.given(parts);
                    let array = new_object(self.context, entry.ty.raw(), given.len() as u64);
                    if array == 0 {
                        return FULL;
                    }
                    self.copy_parts(*element, &given, array);
                    held(&array as *const u64 as *const u8)
                }
                (Reflected::Optional, Kind::Optional(_)) => {
                    let given = self.given(parts);
                    if given.len() > 1 {
                        return RANGE;
                    }
                    let mut room = room(self.table.layout(entry.ty).size as usize);
                    let out = room.as_mut_ptr() as *mut u8;
                    if let Some(&(_, from)) = given.first() {
                        if info.is_reference() {
                            std::ptr::copy_nonoverlapping(from, out, 8);
                        } else {
                            let payload = &info.variants[1].fields[0];
                            let some = 1u64;
                            std::ptr::copy_nonoverlapping(&some as *const u64 as *const u8, out, info.tag_size as usize);
                            std::ptr::copy_nonoverlapping(from, out.add(payload.offset as usize), self.table.layout(payload.ty).size as usize);
                        }
                    }
                    held(out)
                }
                _ => RANGE,
            }
        }
    }

    /// Copies the parts into the elements of the array object.
    ///
    /// # Safety
    /// The array object holds `parts.len()` elements of the type, and each part is a value of it.
    unsafe fn copy_parts(&self, element: TypeId, parts: &[(&Described, *mut u8)], array: u64) {
        let (stride, size) = (self.stride(element), self.table.layout(element).size as usize);
        for (index, &(_, from)) in parts.iter().enumerate() {
            // SAFETY: the caller's promise.
            unsafe { std::ptr::copy_nonoverlapping(from, (array as usize + ELEMENTS_OFFSET as usize + index * stride) as *mut u8, size) };
        }
    }

    /// Builds the value of the type number and variant from the `Dyn`s of a list at the address, or says why not.
    ///
    /// # Safety
    /// `parts` is a live list of `Dyn`; `out` holds the size of the type.
    unsafe fn build(&self, number: u64, tag: u64, parts: u64, out: *mut u8) -> u64 {
        let Some(entry) = self.meta.described.get(number as usize) else { return RANGE };
        let info = self.table.get(entry.ty);
        // SAFETY: the caller's promise, and the layout of the kind says where each field goes.
        unsafe {
            let given = self.given(parts);
            let places: Vec<(TypeId, usize)> = match entry.kind {
                Reflected::Struct | Reflected::Tuple => info.fields.iter().map(|field| (field.ty, field.offset as usize)).collect(),
                Reflected::Class => info.object.as_ref().expect("a class has fields").fields.iter().map(|field| (field.ty, field.offset as usize)).collect(),
                Reflected::Enum => match info.variants.get(tag as usize) {
                    Some(variant) if (tag as i64) >= 0 => variant.fields.iter().map(|field| (field.ty, field.offset as usize)).collect(),
                    _ => return RANGE,
                },
                _ => return RANGE,
            };
            if places.len() != given.len() {
                return RANGE;
            }
            let size = self.table.layout(entry.ty).size as usize;
            std::ptr::write_bytes(out, 0, size.max(8));
            let base = if entry.kind == Reflected::Class {
                let object = new_object(self.context, entry.ty.raw(), 0);
                if object == 0 {
                    return FULL;
                }
                *(out as *mut u64) = object;
                object as usize as *mut u8
            } else {
                out
            };
            if entry.kind == Reflected::Enum {
                std::ptr::copy_nonoverlapping(&tag as *const u64 as *const u8, base, info.tag_size as usize);
            }
            for ((ty, offset), (_, from)) in places.into_iter().zip(given) {
                std::ptr::copy_nonoverlapping(from, base.add(offset), self.table.layout(ty).size as usize);
            }
        }
        0
    }
}

/// An operation on a `Dyn` by its code in `op`; gives a number, a reference, or `RANGE` or `FULL` where an operation can fail.
#[unsafe(no_mangle)]
pub extern "C" fn mote_dyn(context: *mut TaskContext, code: u32, a: u64, b: u64, c: u64, d: u64) -> u64 {
    // SAFETY: compiled code passes the context it was given, which was attached to a live runtime.
    let runtime = unsafe { Runtime::of(context) };
    let meta = runtime.data::<Meta>().expect("a compiled program keeps its meta");
    let table = runtime.heap().table();
    let Some(dyn_type) = meta.dyn_type else { return RANGE };
    let this = Dynamic { table, meta, dyn_type, context };
    // SAFETY: compiled code passes live `Dyn` values, a list of them, or the address of room for the value to build.
    unsafe {
        match code {
            op::LEN => {
                let (entry, at) = this.held(a);
                this.parts(entry, at).len() as u64
            }
            op::TAG => {
                let (entry, at) = this.held(a);
                match entry.kind {
                    Reflected::Enum => read(at, table.get(entry.ty).tag_size),
                    _ => 0,
                }
            }
            op::GET => {
                let (entry, at) = this.held(a);
                let parts = this.parts(entry, at);
                let Some(&(number, from)) = usize::try_from(b as i64).ok().and_then(|index| parts.get(index)) else { return RANGE };
                match this.make(number, from) {
                    0 => FULL,
                    made => made,
                }
            }
            op::SET => {
                let (entry, at) = this.held(a);
                let parts = this.parts(entry, at);
                let Some(&(number, to)) = usize::try_from(b as i64).ok().and_then(|index| parts.get(index)) else { return RANGE };
                let (_, from) = this.held(c);
                std::ptr::copy_nonoverlapping(from, to, table.layout(this.meta.described[number as usize].ty).size as usize);
                0
            }
            op::SHOW => {
                let (entry, at) = this.held(a);
                let text = Show { table, meta }.show(entry.ty, at, b != 0);
                match new_string(context, meta.string.raw(), text.as_bytes()) {
                    0 => FULL,
                    made => made,
                }
            }
            op::BUILD => this.build(a, b, c, d as *mut u8),
            op::MAKE => this.make_dyn(a, b, c),
            _ => RANGE,
        }
    }
}
