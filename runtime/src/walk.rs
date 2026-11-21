//! Equality, hashing and deep copy of values in memory, read by the type table.

use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::OnceLock;

use mote_abi::{Header, Kind, PairPayload, Scalar, Shown, TaskContext, TypeId, TypeTable, BOX_VALUE_OFFSET, ELEMENTS_OFFSET, LENGTH_OFFSET, LIST_BUFFER_OFFSET, LIST_CAPACITY_OFFSET};

use crate::helpers::{new_object, set_word};

/// Where a `Map` keeps its parts, and the types of its keys and values.
struct MapParts {
    key: TypeId,
    value: TypeId,
    keys: usize,
    values: usize,
    hashes: usize,
    slots: usize,
    count: usize,
}

/// The parts of a `Map` class: the fields `held_keys`, `held_values`, `hashes`, `slots` and `count` in order.
fn map_parts(table: &TypeTable, map: TypeId) -> Option<MapParts> {
    let fields = &table.get(map).object.as_ref()?.fields;
    let element = |field: usize| match table.get(fields.get(field)?.ty).kind {
        Kind::List(element) => Some(element),
        _ => None,
    };
    let at = |field: usize| fields.get(field).map(|field| field.offset as usize);
    Some(MapParts { key: element(0)?, value: element(1)?, keys: at(0)?, values: at(1)?, hashes: at(2)?, slots: at(3)?, count: at(4)? })
}

/// The elements of a list object: the address of the first and the length.
///
/// # Safety
/// `list` addresses a live list.
unsafe fn list_elements(list: usize) -> (usize, u64) {
    // SAFETY: the caller's promise: a list has a buffer and a length.
    unsafe {
        let buffer = *((list + LIST_BUFFER_OFFSET as usize) as *const u64) as usize;
        (buffer + ELEMENTS_OFFSET as usize, *((list + LENGTH_OFFSET as usize) as *const u64))
    }
}

/// Reads a word of memory of the given width as 64 bits.
///
/// # Safety
/// `at` addresses `bytes` readable bytes.
unsafe fn read(at: *const u8, bytes: u32) -> u64 {
    // SAFETY: the caller's promise.
    unsafe {
        match bytes {
            1 => u64::from(*at),
            2 => u64::from((at as *const u16).read_unaligned()),
            4 => u64::from((at as *const u32).read_unaligned()),
            _ => (at as *const u64).read_unaligned(),
        }
    }
}

/// The type of the object at the address, from its header.
///
/// # Safety
/// `object` addresses a live object.
unsafe fn type_of(object: usize) -> TypeId {
    // SAFETY: the caller's promise.
    Header::from_bits(unsafe { *(object as *const u64) }).type_id()
}

/// The bytes of a string object or the elements of an array.
///
/// # Safety
/// `object` addresses a live string or array object.
unsafe fn elements(object: usize) -> (*const u8, u64) {
    // SAFETY: the caller's promise.
    unsafe { ((object + ELEMENTS_OFFSET as usize) as *const u8, *((object + LENGTH_OFFSET as usize) as *const u64)) }
}

/// A walk over values by their type: whether two are equal as `==` or as map keys, and a hash of one.
#[derive(Clone, Copy)]
pub(crate) struct Walk<'a> {
    table: &'a TypeTable,
    /// How each type is shown, which tells a `Map` and a `Set` from other classes.
    shown: &'a [Shown],
    /// Compare as keys: `-0.0` is `0.0` and `NaN` is `NaN`, and a class is its address.
    key: bool,
}

impl<'a> Walk<'a> {
    pub(crate) fn new(table: &'a TypeTable, shown: &'a [Shown], key: bool) -> Self {
        Self { table, shown, key }
    }

    fn shown_as(&self, ty: TypeId) -> &Shown {
        self.shown.get(ty.index()).unwrap_or(&Shown::Opaque)
    }

    /// Whether two `Map`s of the type hold the same keys with equal values, in any order.
    ///
    /// # Safety
    /// Both address live maps of the type.
    unsafe fn same_maps(&self, ty: TypeId, a: usize, b: usize) -> bool {
        let Some(map) = map_parts(self.table, ty) else { return false };
        let keys = Walk { key: true, ..*self };
        let (key_stride, value_stride) = (self.stride(map.key) as usize, self.stride(map.value) as usize);
        // SAFETY: the caller's promise: both are maps, whose fields are the lists and the array of the layout.
        unsafe {
            if read((a + map.count) as *const u8, 8) != read((b + map.count) as *const u8, 8) {
                return false;
            }
            let (a_keys, entries) = list_elements(read((a + map.keys) as *const u8, 8) as usize);
            let (a_values, _) = list_elements(read((a + map.values) as *const u8, 8) as usize);
            let (a_hashes, _) = list_elements(read((a + map.hashes) as *const u8, 8) as usize);
            let (b_keys, _) = list_elements(read((b + map.keys) as *const u8, 8) as usize);
            let (b_values, _) = list_elements(read((b + map.values) as *const u8, 8) as usize);
            let (b_hashes, _) = list_elements(read((b + map.hashes) as *const u8, 8) as usize);
            let slots = read((b + map.slots) as *const u8, 8) as usize;
            let (words, places) = (slots + ELEMENTS_OFFSET as usize, *((slots + LENGTH_OFFSET as usize) as *const u64));
            (0..entries as usize).all(|entry| {
                let hash = *((a_hashes + entry * 8) as *const i64);
                if hash == -1 {
                    return true;
                }
                if places == 0 {
                    return false;
                }
                let key = (a_keys + entry * key_stride) as *const u8;
                let mut at = hash as u64 & (places - 1);
                loop {
                    let other = *((words + at as usize * 8) as *const i64);
                    if other == -1 {
                        return false;
                    }
                    let other = other as usize;
                    if *((b_hashes + other * 8) as *const i64) == hash && keys.same(map.key, key, (b_keys + other * key_stride) as *const u8) {
                        return self.same(map.value, (a_values + entry * value_stride) as *const u8, (b_values + other * value_stride) as *const u8);
                    }
                    at = (at + 1) & (places - 1);
                }
            })
        }
    }

    fn stride(&self, ty: TypeId) -> u32 {
        let layout = self.table.layout(ty);
        mote_abi::align_up(layout.size, layout.align)
    }

    /// Whether the values of the type at the two addresses are equal.
    ///
    /// # Safety
    /// Both addresses hold a value of the type, with the objects it refers to live.
    pub(crate) unsafe fn same(&self, ty: TypeId, a: *const u8, b: *const u8) -> bool {
        let info = self.table.get(ty);
        // SAFETY: the caller's promise, and each branch reads only what the layout of the kind holds.
        unsafe {
            match &info.kind {
                Kind::Unit | Kind::Reserved => true,
                Kind::Scalar(scalar) => self.same_scalar(*scalar, a, b),
                Kind::Struct(_) => info.fields.iter().all(|field| self.same(field.ty, a.add(field.offset as usize), b.add(field.offset as usize))),
                Kind::Enum(_) => {
                    if info.variants.is_empty() {
                        return true;
                    }
                    let (left, right) = (read(a, info.tag_size), read(b, info.tag_size));
                    left == right && info.variants[left as usize].fields.iter().all(|field| self.same(field.ty, a.add(field.offset as usize), b.add(field.offset as usize)))
                }
                Kind::Optional(inner) if info.is_reference() => self.same_object(*inner, read(a, 8) as usize, read(b, 8) as usize),
                Kind::Optional(_) => {
                    let (left, right) = (read(a, 8), read(b, 8));
                    left == right && (left == 0 || info.variants[1].fields.iter().all(|field| self.same(field.ty, a.add(field.offset as usize), b.add(field.offset as usize))))
                }
                Kind::Pair => {
                    let (left, right) = (read(a, 8), read(b, 8));
                    if left != right {
                        return false;
                    }
                    if left == 0 {
                        return true;
                    }
                    let held = TypeId::from_raw(left as u32);
                    match self.table.pair_payload(held) {
                        PairPayload::Boxed => {
                            let (x, y) = (read(a.add(8), 8) as usize, read(b.add(8), 8) as usize);
                            self.same(held, (x + BOX_VALUE_OFFSET as usize) as *const u8, (y + BOX_VALUE_OFFSET as usize) as *const u8)
                        }
                        _ => self.same(held, a.add(8), b.add(8)),
                    }
                }
                Kind::FixedArray(element, count) => {
                    let stride = self.stride(*element) as usize;
                    (0..*count as usize).all(|at| self.same(*element, a.add(at * stride), b.add(at * stride)))
                }
                _ => self.same_object(ty, read(a, 8) as usize, read(b, 8) as usize),
            }
        }
    }

    unsafe fn same_scalar(&self, scalar: Scalar, a: *const u8, b: *const u8) -> bool {
        // SAFETY: the caller's promise.
        unsafe {
            match scalar {
                Scalar::F64 => {
                    let (x, y) = (f64::from_bits(read(a, 8)), f64::from_bits(read(b, 8)));
                    if self.key { x == y || (x.to_bits() == y.to_bits()) } else { x == y }
                }
                Scalar::F32 => {
                    let (x, y) = (f32::from_bits(read(a, 4) as u32), f32::from_bits(read(b, 4) as u32));
                    if self.key { x == y || (x.to_bits() == y.to_bits()) } else { x == y }
                }
                other => read(a, other.size()) == read(b, other.size()),
            }
        }
    }

    /// Whether two references of the type are the same object or objects of equal contents.
    unsafe fn same_object(&self, ty: TypeId, a: usize, b: usize) -> bool {
        if a == b {
            return true;
        }
        if a == 0 || b == 0 {
            return false;
        }
        // SAFETY: the caller's promise: both address live objects of the type.
        unsafe {
            match self.table.get(ty).kind {
                Kind::String => {
                    let (x, length) = elements(a);
                    let (y, other) = elements(b);
                    length == other && std::slice::from_raw_parts(x, length as usize) == std::slice::from_raw_parts(y, length as usize)
                }
                Kind::Array(element) => self.same_elements(element, elements(a).0, elements(b).0, *((a + LENGTH_OFFSET as usize) as *const u64), *((b + LENGTH_OFFSET as usize) as *const u64)),
                Kind::List(element) => {
                    let (x, y) = (*((a + LIST_BUFFER_OFFSET as usize) as *const u64) as usize, *((b + LIST_BUFFER_OFFSET as usize) as *const u64) as usize);
                    self.same_elements(element, elements(x).0, elements(y).0, *((a + LENGTH_OFFSET as usize) as *const u64), *((b + LENGTH_OFFSET as usize) as *const u64))
                }
                Kind::Box(inner) => self.same(inner, (a + BOX_VALUE_OFFSET as usize) as *const u8, (b + BOX_VALUE_OFFSET as usize) as *const u8),
                Kind::Class(_) if !self.key => match self.shown_as(ty) {
                    Shown::Map => self.same_maps(ty, a, b),
                    Shown::Set => {
                        let members = self.table.get(ty).object.as_ref().and_then(|object| object.fields.first());
                        members.is_some_and(|field| self.same_object(field.ty, read((a + field.offset as usize) as *const u8, 8) as usize, read((b + field.offset as usize) as *const u8, 8) as usize))
                    }
                    _ => false,
                },
                _ => false,
            }
        }
    }

    unsafe fn same_elements(&self, element: TypeId, a: *const u8, b: *const u8, left: u64, right: u64) -> bool {
        if left != right {
            return false;
        }
        let stride = self.stride(element) as usize;
        // SAFETY: the caller's promise: both hold `left` elements.
        (0..left as usize).all(|at| unsafe { self.same(element, a.add(at * stride), b.add(at * stride)) })
    }

    /// Feeds the value at the address to the hasher, so that values that are equal as keys feed the same.
    ///
    /// # Safety
    /// The address holds a value of the type, with the objects it refers to live.
    pub(crate) unsafe fn hash(&self, ty: TypeId, at: *const u8, state: &mut impl Hasher) {
        let info = self.table.get(ty);
        // SAFETY: the caller's promise.
        unsafe {
            match &info.kind {
                Kind::Unit | Kind::Reserved => {}
                Kind::Scalar(Scalar::F64) => hash_float(f64::from_bits(read(at, 8)), state),
                Kind::Scalar(Scalar::F32) => hash_float(f64::from(f32::from_bits(read(at, 4) as u32)), state),
                Kind::Scalar(scalar) => read(at, scalar.size()).hash(state),
                Kind::Struct(_) => info.fields.iter().for_each(|field| self.hash(field.ty, at.add(field.offset as usize), state)),
                Kind::Enum(_) => {
                    if info.variants.is_empty() {
                        return;
                    }
                    let tag = read(at, info.tag_size);
                    tag.hash(state);
                    info.variants[tag as usize].fields.iter().for_each(|field| self.hash(field.ty, at.add(field.offset as usize), state));
                }
                Kind::Optional(inner) if info.is_reference() => self.hash_object(*inner, read(at, 8) as usize, state),
                Kind::Optional(_) => {
                    let tag = read(at, 8);
                    tag.hash(state);
                    if tag != 0 {
                        info.variants[1].fields.iter().for_each(|field| self.hash(field.ty, at.add(field.offset as usize), state));
                    }
                }
                Kind::Pair => {
                    let word = read(at, 8);
                    word.hash(state);
                    if word != 0 {
                        let held = TypeId::from_raw(word as u32);
                        match self.table.pair_payload(held) {
                            PairPayload::Boxed => self.hash(held, (read(at.add(8), 8) as usize + BOX_VALUE_OFFSET as usize) as *const u8, state),
                            _ => self.hash(held, at.add(8), state),
                        }
                    }
                }
                Kind::FixedArray(element, count) => {
                    let stride = self.stride(*element) as usize;
                    (0..*count as usize).for_each(|index| self.hash(*element, at.add(index * stride), state));
                }
                _ => self.hash_object(ty, read(at, 8) as usize, state),
            }
        }
    }

    unsafe fn hash_object(&self, ty: TypeId, object: usize, state: &mut impl Hasher) {
        if object == 0 {
            0u64.hash(state);
            return;
        }
        // SAFETY: the caller's promise: the object is live and of the type.
        unsafe {
            match self.table.get(ty).kind {
                Kind::String => {
                    let (bytes, length) = elements(object);
                    std::slice::from_raw_parts(bytes, length as usize).hash(state);
                }
                Kind::Array(element) => {
                    let length = *((object + LENGTH_OFFSET as usize) as *const u64);
                    self.hash_elements(element, elements(object).0, length, state);
                }
                Kind::List(element) => {
                    let buffer = *((object + LIST_BUFFER_OFFSET as usize) as *const u64) as usize;
                    let length = *((object + LENGTH_OFFSET as usize) as *const u64);
                    self.hash_elements(element, elements(buffer).0, length, state);
                }
                Kind::Box(inner) => self.hash(inner, (object + BOX_VALUE_OFFSET as usize) as *const u8, state),
                _ => object.hash(state),
            }
        }
    }

    unsafe fn hash_elements(&self, element: TypeId, first: *const u8, count: u64, state: &mut impl Hasher) {
        let stride = self.stride(element) as usize;
        count.hash(state);
        // SAFETY: the caller's promise: `count` elements follow.
        (0..count as usize).for_each(|at| unsafe { self.hash(element, first.add(at * stride), state) });
    }

    /// The hash of a value as a non-negative `Int`, which differs from process to process.
    ///
    /// # Safety
    /// The address holds a value of the type.
    pub(crate) unsafe fn hash_of(&self, ty: TypeId, at: *const u8) -> u64 {
        static SEED: OnceLock<std::collections::hash_map::RandomState> = OnceLock::new();
        let mut state = SEED.get_or_init(Default::default).build_hasher();
        // SAFETY: the caller's promise.
        unsafe { self.hash(ty, at, &mut state) };
        state.finish() >> 1
    }
}

fn hash_float(x: f64, state: &mut impl Hasher) {
    (if x == 0.0 { 0 } else { x.to_bits() }).hash(state);
}

/// A copy of everything reachable from a value; an object reached twice is copied once.
pub(crate) struct Copier<'a> {
    table: &'a TypeTable,
    shown: &'a [Shown],
    context: *mut TaskContext,
    seen: HashMap<usize, usize>,
}

impl<'a> Copier<'a> {
    pub(crate) fn new(table: &'a TypeTable, shown: &'a [Shown], context: *mut TaskContext) -> Self {
        Self { table, shown, context, seen: HashMap::new() }
    }

    /// Recomputes the hashes and slots of a copied `Map`, whose keys held by address now hash differently.
    ///
    /// # Safety
    /// `map` addresses a live map of the type, whose lists and slots are complete.
    unsafe fn rehash(&self, ty: TypeId, map: usize) {
        let Some(parts) = map_parts(self.table, ty) else { return };
        let walk = Walk::new(self.table, self.shown, true);
        let key_stride = self.stride(parts.key) as usize;
        // SAFETY: the caller's promise: the lists and the array are the ones of the layout.
        unsafe {
            let (keys, entries) = list_elements(read((map + parts.keys) as *const u8, 8) as usize);
            let (hashes, _) = list_elements(read((map + parts.hashes) as *const u8, 8) as usize);
            let slots = read((map + parts.slots) as *const u8, 8) as usize;
            let (words, places) = (slots + ELEMENTS_OFFSET as usize, *((slots + LENGTH_OFFSET as usize) as *const u64));
            for at in 0..places as usize {
                *((words + at * 8) as *mut i64) = -1;
            }
            for entry in 0..entries as usize {
                let cell = (hashes + entry * 8) as *mut i64;
                if *cell == -1 {
                    continue;
                }
                let hash = walk.hash_of(parts.key, (keys + entry * key_stride) as *const u8) as i64;
                *cell = hash;
                let mut at = hash as u64 & (places - 1);
                while *((words + at as usize * 8) as *const i64) != -1 {
                    at = (at + 1) & (places - 1);
                }
                *((words + at as usize * 8) as *mut i64) = entry as i64;
            }
        }
    }

    fn stride(&self, ty: TypeId) -> u32 {
        let layout = self.table.layout(ty);
        mote_abi::align_up(layout.size, layout.align)
    }

    /// Writes a copy of the value at `from` to `to`; `None` when the heap is full.
    ///
    /// # Safety
    /// `from` holds a value of the type and `to` has room for one.
    pub(crate) unsafe fn copy(&mut self, ty: TypeId, from: *const u8, to: *mut u8) -> Option<()> {
        let info = self.table.get(ty);
        // SAFETY: the caller's promise.
        unsafe {
            match &info.kind {
                Kind::Unit | Kind::Reserved => {}
                Kind::Scalar(_) => std::ptr::copy_nonoverlapping(from, to, info.layout.size as usize),
                Kind::Struct(_) => {
                    for field in &info.fields {
                        self.copy(field.ty, from.add(field.offset as usize), to.add(field.offset as usize))?;
                    }
                }
                Kind::Enum(_) => {
                    std::ptr::copy_nonoverlapping(from, to, info.layout.size as usize);
                    if !info.variants.is_empty() {
                        let tag = read(from, info.tag_size) as usize;
                        for field in &info.variants[tag].fields {
                            self.copy(field.ty, from.add(field.offset as usize), to.add(field.offset as usize))?;
                        }
                    }
                }
                Kind::Optional(inner) if info.is_reference() => (to as *mut u64).write_unaligned(self.copy_object(*inner, read(from, 8) as usize)? as u64),
                Kind::Optional(_) => {
                    std::ptr::copy_nonoverlapping(from, to, info.layout.size as usize);
                    if read(from, 8) != 0 {
                        for field in &info.variants[1].fields {
                            self.copy(field.ty, from.add(field.offset as usize), to.add(field.offset as usize))?;
                        }
                    }
                }
                Kind::Pair => {
                    std::ptr::copy_nonoverlapping(from, to, info.layout.size as usize);
                    let word = read(from, 8);
                    if word != 0 {
                        let held = TypeId::from_raw(word as u32);
                        match self.table.pair_payload(held) {
                            PairPayload::Boxed => {
                                let object = read(from.add(8), 8) as usize;
                                let copy = new_object(self.context, type_of(object).raw(), 0);
                                if copy == 0 {
                                    return None;
                                }
                                self.copy(held, (object + BOX_VALUE_OFFSET as usize) as *const u8, (copy as usize + BOX_VALUE_OFFSET as usize) as *mut u8)?;
                                (to.add(8) as *mut u64).write_unaligned(copy);
                            }
                            _ => self.copy(held, from.add(8), to.add(8))?,
                        }
                    }
                }
                Kind::FixedArray(element, count) => {
                    let stride = self.stride(*element) as usize;
                    for at in 0..*count as usize {
                        self.copy(*element, from.add(at * stride), to.add(at * stride))?;
                    }
                }
                _ => (to as *mut u64).write_unaligned(self.copy_object(ty, read(from, 8) as usize)? as u64),
            }
        }
        Some(())
    }

    /// A copy of the object, or the object itself when it is a string, closure or handle.
    unsafe fn copy_object(&mut self, ty: TypeId, object: usize) -> Option<usize> {
        if object == 0 {
            return Some(0);
        }
        if let Some(&done) = self.seen.get(&object) {
            return Some(done);
        }
        let info = self.table.get(ty);
        // SAFETY: the caller's promise: the object is live and of the type.
        unsafe {
            match &info.kind {
                Kind::Class(_) | Kind::Box(_) => {
                    let copy = new_object(self.context, ty.raw(), 0) as usize;
                    if copy == 0 {
                        return None;
                    }
                    self.seen.insert(object, copy);
                    for field in &info.object.as_ref()?.fields {
                        self.copy(field.ty, (object + field.offset as usize) as *const u8, (copy + field.offset as usize) as *mut u8)?;
                    }
                    if matches!(info.kind, Kind::Class(_)) && matches!(self.shown.get(ty.index()), Some(Shown::Map)) {
                        self.rehash(ty, copy);
                    }
                    Some(copy)
                }
                Kind::Array(element) => {
                    let length = *((object + LENGTH_OFFSET as usize) as *const u64);
                    let copy = new_object(self.context, ty.raw(), length) as usize;
                    if copy == 0 {
                        return None;
                    }
                    self.seen.insert(object, copy);
                    self.copy_elements(*element, object, copy, length)?;
                    Some(copy)
                }
                Kind::List(element) => {
                    let length = *((object + LENGTH_OFFSET as usize) as *const u64);
                    let buffer = *((object + LIST_BUFFER_OFFSET as usize) as *const u64) as usize;
                    let copy = new_object(self.context, ty.raw(), 0) as usize;
                    if copy == 0 {
                        return None;
                    }
                    self.seen.insert(object, copy);
                    let fresh = new_object(self.context, type_of(buffer).raw(), length) as usize;
                    if fresh == 0 {
                        return None;
                    }
                    self.copy_elements(*element, buffer, fresh, length)?;
                    set_word(copy as u64, LENGTH_OFFSET, length);
                    set_word(copy as u64, LIST_CAPACITY_OFFSET, length);
                    set_word(copy as u64, LIST_BUFFER_OFFSET, fresh as u64);
                    Some(copy)
                }
                _ => Some(object),
            }
        }
    }

    unsafe fn copy_elements(&mut self, element: TypeId, from: usize, to: usize, count: u64) -> Option<()> {
        let stride = self.stride(element) as usize;
        for at in 0..count as usize {
            // SAFETY: the caller's promise: both objects hold `count` elements.
            unsafe { self.copy(element, (from + ELEMENTS_OFFSET as usize + at * stride) as *const u8, (to + ELEMENTS_OFFSET as usize + at * stride) as *mut u8)? };
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use mote_abi::TypeTable;

    use super::*;

    fn table() -> (TypeTable, TypeId, TypeId) {
        let mut table = TypeTable::new();
        let float = table.add("Float", Kind::Scalar(Scalar::F64));
        let int = table.add("Int", Kind::Scalar(Scalar::I64));
        (table, float, int)
    }

    #[test]
    fn walk_keys_that_are_equal_hash_alike_and_zero_has_one_hash() {
        let (table, float, int) = table();
        let walk = Walk::new(&table, &[], true);
        let (zero, negative) = (0.0f64, -0.0f64);
        // SAFETY: both addresses hold a `Float`.
        unsafe {
            assert!(walk.same(float, &zero as *const f64 as *const u8, &negative as *const f64 as *const u8));
            assert_eq!(walk.hash_of(float, &zero as *const f64 as *const u8), walk.hash_of(float, &negative as *const f64 as *const u8));
            let (one, other) = (1i64, 2i64);
            assert!(!walk.same(int, &one as *const i64 as *const u8, &other as *const i64 as *const u8));
        }
    }

    #[test]
    fn walk_nan_is_not_equal_to_itself_as_a_value_but_is_as_a_key() {
        let (table, float, _) = table();
        let nan = f64::NAN;
        let at = &nan as *const f64 as *const u8;
        // SAFETY: the address holds a `Float`.
        unsafe {
            assert!(!Walk::new(&table, &[], false).same(float, at, at));
            assert!(Walk::new(&table, &[], true).same(float, at, at));
        }
    }
}
