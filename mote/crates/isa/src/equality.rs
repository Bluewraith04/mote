//! `==` for every value, and the key hash and key equality `Map` / `Set` use.

use crate::value::{
    ObjectHeader, Value, BACKING_DATA_BASE, HEADER_BACKING_SLOT,
    HEADER_LEN_SLOT, LIST_TYPE_ID, MAP_ENTRY_STRIDE, MAP_TYPE_ID, SET_TYPE_ID, TABLE_INDEX_SLOT, TABLE_USED_SLOT,
};
use std::ptr::NonNull;

const MAX_DEPTH: usize = 256;

/// Scalars by value; strings, lists, tuples, value types, enum variants, maps and sets by content; other heap objects by identity.
pub fn values_equal(lhs: Value, rhs: Value) -> bool {
    equal_at(lhs, rhs, 0)
}

fn equal_at(lhs: Value, rhs: Value, depth: usize) -> bool {
    match (lhs, rhs) {
        (Value::ObjectPtr(_), Value::ObjectPtr(_)) => heap_equal(lhs, rhs, depth),
        _ => lhs == rhs,
    }
}

fn heap_equal(lhs: Value, rhs: Value, depth: usize) -> bool {
    let (Some(a), Some(b)) = (lhs.as_object_ptr(), rhs.as_object_ptr()) else { return false };
    match (lhs.heap_bytes(), rhs.heap_bytes()) {
        (Some(x), Some(y)) => x == y,
        (None, None) => {
            let same_object = a == b;
            if same_object || depth >= MAX_DEPTH {
                return same_object;
            }
            // SAFETY: a boxed pointer points at a live object header.
            unsafe { objects_equal(a, b, depth + 1) }
        }
        _ => false,
    }
}

/// # Safety
/// `a` and `b` are live objects whose slots match their descriptors.
unsafe fn objects_equal(a: NonNull<ObjectHeader>, b: NonNull<ObjectHeader>, depth: usize) -> bool {
    unsafe {
        let (ha, hb) = (a.as_ref(), b.as_ref());
        let desc = ha.type_ptr.as_ref();
        if desc.id != hb.type_ptr.as_ref().id {
            return false;
        }
        match desc.id {
            LIST_TYPE_ID => return lists_equal(ha, hb, depth),
            MAP_TYPE_ID => return tables_equal(ha, hb, MAP_ENTRY_STRIDE, depth),
            SET_TYPE_ID => return tables_equal(ha, hb, 1, depth),
            _ => {}
        }
        if !desc.is_value_type && !desc.by_content {
            return false;
        }
        (0..desc.slots as usize).all(|i| equal_at(ha.get_field(i), hb.get_field(i), depth))
    }
}

fn count(v: Value) -> usize {
    v.as_uint().map(|u| u as usize).or_else(|| v.as_int().filter(|n| *n >= 0).map(|n| n as usize)).unwrap_or(0)
}

/// # Safety
/// `a` and `b` are live `List` headers with live backings.
unsafe fn lists_equal(a: &ObjectHeader, b: &ObjectHeader, depth: usize) -> bool {
    unsafe {
        let len = count(a.get_field(HEADER_LEN_SLOT));
        if len != count(b.get_field(HEADER_LEN_SLOT)) {
            return false;
        }
        let (Some(ba), Some(bb)) = (a.get_field(HEADER_BACKING_SLOT).as_object_ptr(), b.get_field(HEADER_BACKING_SLOT).as_object_ptr())
        else {
            return len == 0;
        };
        let (ba, bb) = (ba.as_ref(), bb.as_ref());
        (0..len).all(|i| equal_at(ba.get_field(BACKING_DATA_BASE + i), bb.get_field(BACKING_DATA_BASE + i), depth))
    }
}

/// # Safety
/// `a` and `b` are live `Map` or `Set` headers with live entries and index.
unsafe fn tables_equal(a: &ObjectHeader, b: &ObjectHeader, stride: usize, depth: usize) -> bool {
    unsafe {
        let len = count(a.get_field(HEADER_LEN_SLOT));
        if len != count(b.get_field(HEADER_LEN_SLOT)) {
            return false;
        }
        let Some(ea) = a.get_field(HEADER_BACKING_SLOT).as_object_ptr() else { return len == 0 };
        let ea = ea.as_ref();
        (0..count(a.get_field(TABLE_USED_SLOT))).all(|i| {
            let k = ea.get_field(1 + stride * i);
            if k.is_null() {
                return true;
            }
            let Ok(Some((_, j))) = table_find(b, stride, k) else { return false };
            let eb = b.get_field(HEADER_BACKING_SLOT).as_object_ptr().map(|p| p.as_ref());
            stride == 1 || eb.is_some_and(|eb| equal_at(ea.get_field(2 + stride * i), eb.get_field(2 + stride * j), depth))
        })
    }
}

/// Index slot value for "no entry here"; a stored entry is `index + 1`.
pub const TABLE_EMPTY: u32 = 0;
/// Index slot value for an entry that was removed; probing continues past it.
pub const TABLE_DUMMY: u32 = u32::MAX;

/// Index slots for a table of `cap` entries: a power of two, at least twice `cap`.
pub fn table_index_len(cap: usize) -> usize {
    (cap * 2).next_power_of_two().max(16)
}

/// Reads index slot `i` of a table index object.
///
/// # Safety
/// `index` is a live table index and `i` is below its slot count.
#[inline(always)]
pub unsafe fn index_get(index: &ObjectHeader, i: usize) -> u32 {
    unsafe { std::ptr::read_unaligned((index.field_ptr(1) as *const u32).add(i)) }
}

/// Where `key` lives in a `Map` (`stride` 2) or `Set` (`stride` 1): its index slot and entry number.
///
/// # Safety
/// `h` is a live `Map` or `Set` header whose entries and index are live.
pub unsafe fn table_find(h: &ObjectHeader, stride: usize, key: Value) -> Result<Option<(usize, usize)>, String> {
    unsafe { Ok(table_find_hashed(h, stride, key_hash(key)?, key)) }
}

/// [`table_find`] for a key whose [`key_hash`] the caller already has.
///
/// # Safety
/// As [`table_find`].
pub unsafe fn table_find_hashed(h: &ObjectHeader, stride: usize, hash: u64, key: Value) -> Option<(usize, usize)> {
    unsafe {
        let (Some(entries), Some(index)) = (h.get_field(HEADER_BACKING_SLOT).as_object_ptr(), h.get_field(TABLE_INDEX_SLOT).as_object_ptr())
        else {
            return None;
        };
        let (entries, index) = (entries.as_ref(), index.as_ref());
        let n = count(index.get_field(0)) / 4;
        let mut slot = hash as usize & (n - 1);
        for _ in 0..n {
            match index_get(index, slot) {
                TABLE_EMPTY => return None,
                TABLE_DUMMY => {}
                stored => {
                    let entry = stored as usize - 1;
                    if keys_equal(entries.get_field(1 + stride * entry), key) {
                        return Some((slot, entry));
                    }
                }
            }
            slot = (slot + 1) & (n - 1);
        }
        None
    }
}

pub fn fnv1a(seed: u8, bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ (seed as u64);
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Hash of a `Map` / `Set` key. Tag-seeded so `1` (Int) and `1` (UInt) land in different buckets.
pub fn key_hash(v: Value) -> Result<u64, String> {
    match v {
        Value::Int(n) => Ok(fnv1a(1, &n.to_le_bytes())),
        Value::UInt(n) => Ok(fnv1a(2, &n.to_le_bytes())),
        Value::Float(f) => Ok(fnv1a(3, &f.to_bits().to_le_bytes())),
        Value::Char(c) => Ok(fnv1a(4, &(c as u32).to_le_bytes())),
        Value::Bool(true) => Ok(fnv1a(5, &[])),
        Value::Bool(false) => Ok(fnv1a(6, &[])),
        Value::Symbol(id) => Ok(fnv1a(9, &id.to_le_bytes())),
        Value::ObjectPtr(_) => Ok(heap_hash(v, 0)),
        Value::Null => Err("null is not a valid Map/Set key".to_string()),
        _ => Err("this value type cannot be a Map/Set key".to_string()),
    }
}

const MAX_HASH_DEPTH: usize = 8;

fn heap_hash(v: Value, depth: usize) -> u64 {
    if let Some(s) = v.heap_bytes() {
        return fnv1a(7, s);
    }
    let Some(p) = v.as_object_ptr() else { return fnv1a(8, &[]) };
    // SAFETY: a boxed pointer points at a live object header whose descriptor outlives it.
    let (h, desc) = unsafe { (p.as_ref(), p.as_ref().type_ptr.as_ref()) };
    let id = desc.id.to_le_bytes();
    if depth >= MAX_HASH_DEPTH {
        return fnv1a(10, &id);
    }
    let mix = |acc: u64, x: u64| (acc ^ x).wrapping_mul(0x0000_0100_0000_01b3);
    // SAFETY: `h` is the live header read above, and its slots match its descriptor.
    unsafe {
        match desc.id {
            LIST_TYPE_ID => {
                let len = count(h.get_field(HEADER_LEN_SLOT));
                let Some(b) = h.get_field(HEADER_BACKING_SLOT).as_object_ptr() else { return fnv1a(11, &id) };
                (0..len).fold(fnv1a(11, &id), |acc, i| mix(acc, elem_hash(b.as_ref().get_field(BACKING_DATA_BASE + i), depth + 1)))
            }
            MAP_TYPE_ID | SET_TYPE_ID => fnv1a(12, &[id, (count(h.get_field(HEADER_LEN_SLOT)) as u64).to_le_bytes()].concat()),
            _ if desc.is_value_type || desc.by_content => {
                (0..desc.slots as usize).fold(fnv1a(13, &id), |acc, i| mix(acc, elem_hash(h.get_field(i), depth + 1)))
            }
            _ => fnv1a(8, &(p.as_ptr() as usize as u64).to_le_bytes()),
        }
    }
}

fn elem_hash(v: Value, depth: usize) -> u64 {
    match v {
        Value::ObjectPtr(_) => heap_hash(v, depth),
        Value::Float(f) => fnv1a(3, &(f + 0.0).to_bits().to_le_bytes()),
        _ => key_hash(v).unwrap_or_else(|_| fnv1a(0, &[v.wire_tag() as u8])),
    }
}

/// Key equality for `Map` / `Set`; mirrors `key_hash`.
pub(crate) fn keys_equal(a: Value, b: Value) -> bool {
    match (a, b) {
        (Value::Float(x), Value::Float(y)) => x.to_bits() == y.to_bits(),
        (Value::ObjectPtr(_), Value::ObjectPtr(_)) => values_equal(a, b),
        (Value::NativeFn(_), Value::NativeFn(_)) | (Value::Inline(_), Value::Inline(_)) => false,
        _ => a == b,
    }
}
