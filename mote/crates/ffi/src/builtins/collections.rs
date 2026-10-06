//! List, Map, Set and Bytes natives.

use super::*;

const GROWTH_KNEE_BYTES: usize = 1 << 20;

const SHRINK_MIN_CAP: usize = 64;

const LIST_INITIAL_CAP: usize = 4;

fn grown_capacity(cap: usize, elem_bytes: usize) -> usize {
    if cap.saturating_mul(elem_bytes) < GROWTH_KNEE_BYTES {
        cap.saturating_mul(2)
    } else {
        cap.saturating_add(cap / 2)
    }
}

fn shrunk_capacity(len: usize, cap: usize) -> Option<usize> {
    (cap >= SHRINK_MIN_CAP && len <= cap / 4).then_some(cap / 2)
}

pub(super) fn slot_count(heap: &dyn NativeCtx, obj: Value, i: usize) -> Result<usize, String> {
    let v = heap.get_slot(obj, i)?;
    v.as_uint()
        .map(|u| u as usize)
        .or_else(|| v.as_int().filter(|n| *n >= 0).map(|n| n as usize))
        .ok_or_else(|| "list: corrupt header/backing count slot".to_string())
}

pub(super) fn list_receiver(ctx: &NativeCallContext<'_>) -> Result<Value, String> {
    let v = ctx.arg(0).unwrap_or(Value::null());
    if is_list(v) {
        Ok(v)
    } else {
        Err("list method called on a non-list value".to_string())
    }
}

pub(super) fn native_len(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let v = ctx.arg(0).unwrap_or(Value::null());
    if let Some(s) = v.heap_bytes() {
        return Ok(Value::int(s.len() as i64));
    }
    if is_list(v) || is_map(v) || is_set(v) || is_bytes(v) {
        return Ok(Value::int(slot_count(ctx.heap, v, HEADER_LEN_SLOT)? as i64));
    }
    Ok(Value::int(0))
}

pub(super) fn list_new(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let hint = ctx.arg(0).and_then(|v| v.as_int()).unwrap_or(0).max(0) as usize;
    let list = ctx.heap.alloc_header(LIST_TYPE_ID, 2)?;
    let backing = ctx.heap.alloc_backing(hint)?;
    ctx.heap.set_slot(list, HEADER_LEN_SLOT, Value::uint(0))?;
    ctx.heap.set_slot(list, HEADER_BACKING_SLOT, backing)?;
    Ok(list)
}

pub(super) fn list_elem_slot(ctx: &NativeCallContext<'_>, list: Value, idx: Value) -> Result<usize, String> {
    let idx = idx
        .as_int()
        .ok_or_else(|| "list index must be an Int".to_string())?;
    let len = slot_count(ctx.heap, list, HEADER_LEN_SLOT)?;
    if idx < 0 || idx as usize >= len {
        return Err(format!("list index {idx} out of bounds (len {len})"));
    }
    Ok(BACKING_DATA_BASE + idx as usize)
}

pub(super) fn coll_get(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let recv = ctx.arg(0).unwrap_or(Value::null());
    let key = ctx.arg(1).unwrap_or(Value::null());
    if is_list(recv) {
        let slot = list_elem_slot(ctx, recv, key)?;
        let backing = ctx.heap.get_slot(recv, HEADER_BACKING_SLOT)?;
        return ctx.heap.get_slot(backing, slot);
    }
    if is_bytes(recv) {
        let (backing, i) = bytes_index(ctx, recv, key)?;
        return Ok(Value::int(ctx.heap.read_bytes(backing, i, 1)?[0] as i64));
    }
    if is_map(recv) {
        return match map_lookup(ctx, recv, key)? {
            Some(v) => Ok(v),
            None => Err("map.get: key not present".to_string()),
        };
    }
    Err("`.get()` is only defined on List, Bytes, and Map".to_string())
}

pub(super) fn coll_set(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let recv = ctx.arg(0).unwrap_or(Value::null());
    let key = ctx.arg(1).unwrap_or(Value::null());
    let val = ctx.arg(2).unwrap_or(Value::null());
    if is_list(recv) {
        let slot = list_elem_slot(ctx, recv, key)?;
        let backing = ctx.heap.get_slot(recv, HEADER_BACKING_SLOT)?;
        ctx.heap.set_slot(backing, slot, val)?;
        return Ok(Value::null());
    }
    if is_bytes(recv) {
        let byte = as_byte(val)?;
        let (backing, i) = bytes_index(ctx, recv, key)?;
        ctx.heap.write_bytes(backing, i, &[byte])?;
        return Ok(Value::null());
    }
    if is_map(recv) {
        map_insert(ctx, recv, key, val)?;
        return Ok(Value::null());
    }
    Err("`.set()` is only defined on List, Bytes, and Map".to_string())
}

pub(super) fn coll_get_or(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let recv = ctx.arg(0).unwrap_or(Value::null());
    let key = ctx.arg(1).unwrap_or(Value::null());
    let default = ctx.arg(2).unwrap_or(Value::null());
    if is_list(recv) {
        match list_elem_slot(ctx, recv, key) {
            Ok(slot) => {
                let backing = ctx.heap.get_slot(recv, HEADER_BACKING_SLOT)?;
                ctx.heap.get_slot(backing, slot)
            }
            Err(_) => Ok(default),
        }
    } else if is_map(recv) {
        Ok(map_lookup(ctx, recv, key)?.unwrap_or(default))
    } else {
        Err("`.get_or()` is only defined on List and Map".to_string())
    }
}

pub(super) fn coll_push(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let recv = ctx.arg(0).unwrap_or(Value::null());
    if is_bytes(recv) {
        let byte = as_byte(ctx.arg(1).unwrap_or(Value::null()))?;
        return bytes_push(ctx, recv, byte);
    }
    let list = list_receiver(ctx)?;
    let val = ctx.arg(1).unwrap_or(Value::null());
    let len = slot_count(ctx.heap, list, HEADER_LEN_SLOT)?;
    let mut backing = ctx.heap.get_slot(list, HEADER_BACKING_SLOT)?;
    let cap = slot_count(ctx.heap, backing, BACKING_CAP_SLOT)?;

    if len == cap {
        let new_cap = if cap == 0 { LIST_INITIAL_CAP } else { grown_capacity(cap, size_of::<Value>()) };
        backing = resize_list_backing(ctx, list, backing, len, new_cap)?;
    }

    ctx.heap.set_slot(backing, BACKING_DATA_BASE + len, val)?;
    ctx.heap
        .set_slot(list, HEADER_LEN_SLOT, Value::uint((len + 1) as u64))?;
    Ok(Value::null())
}

fn resize_list_backing(ctx: &mut NativeCallContext<'_>, list: Value, backing: Value, len: usize, new_cap: usize) -> Result<Value, String> {
    let resized = ctx.heap.alloc_backing(new_cap)?;
    for i in 0..len {
        let e = ctx.heap.get_slot(backing, BACKING_DATA_BASE + i)?;
        ctx.heap.set_slot(resized, BACKING_DATA_BASE + i, e)?;
    }
    ctx.heap.set_slot(list, HEADER_BACKING_SLOT, resized)?;
    ctx.heap.release_backing(backing);
    Ok(resized)
}

pub(super) fn list_pop(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let list = list_receiver(ctx)?;
    let len = slot_count(ctx.heap, list, HEADER_LEN_SLOT)?;
    if len == 0 {
        return Err("list.pop: the list is empty".to_string());
    }
    let backing = ctx.heap.get_slot(list, HEADER_BACKING_SLOT)?;
    let last = BACKING_DATA_BASE + len - 1;
    let val = ctx.heap.get_slot(backing, last)?;
    ctx.heap.set_slot(backing, last, Value::null())?;
    ctx.heap
        .set_slot(list, HEADER_LEN_SLOT, Value::uint((len - 1) as u64))?;
    let cap = slot_count(ctx.heap, backing, BACKING_CAP_SLOT)?;
    if let Some(new_cap) = shrunk_capacity(len - 1, cap) {
        resize_list_backing(ctx, list, backing, len - 1, new_cap)?;
    }
    Ok(val)
}

pub(super) fn coll_clear(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let recv = ctx.arg(0).unwrap_or(Value::null());
    if is_map(recv) || is_set(recv) {
        table_clear(ctx, recv, if is_map(recv) { &MAP_LAYOUT } else { &SET_LAYOUT })?;
        return Ok(Value::null());
    }
    let backing = ctx.heap.get_slot(recv, HEADER_BACKING_SLOT)?;
    let cap = slot_count(ctx.heap, backing, BACKING_CAP_SLOT)?;
    if cap >= SHRINK_MIN_CAP {
        let fresh = if is_bytes(recv) {
            ctx.heap.alloc_bytes_backing(BYTES_MIN_CAP)?
        } else {
            ctx.heap.alloc_backing(LIST_INITIAL_CAP)?
        };
        ctx.heap.set_slot(recv, HEADER_BACKING_SLOT, fresh)?;
        ctx.heap.release_backing(backing);
    } else if !is_bytes(recv) {
        for i in 1..=cap {
            ctx.heap.set_slot(backing, i, Value::null())?;
        }
    }
    ctx.heap.set_slot(recv, HEADER_LEN_SLOT, Value::uint(0))?;
    Ok(Value::null())
}

pub(super) fn list_iter(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let recv = ctx.arg(0).unwrap_or(Value::null());
    if matches!(type_id_of(recv), Some(id) if id == CHANNEL_TYPE_ID || id == isa::value::GENERATOR_TYPE_ID) {
        return Ok(recv);
    }
    let list = list_receiver(ctx)?;
    let cursor = ctx.heap.alloc_header(LIST_ITER_TYPE_ID, 2)?;
    ctx.heap.set_slot(cursor, 0, list)?;
    ctx.heap.set_slot(cursor, 1, Value::int(0))?;
    Ok(cursor)
}

pub(super) const TABLE_INITIAL_ENTRIES: usize = 8;

const TABLE_MAX_ENTRIES: usize = 1 << 30;

pub(super) fn vbool(b: bool) -> Value {
    if b {
        Value::true_()
    } else {
        Value::false_()
    }
}

use isa::equality::{index_get, key_hash, table_find, table_find_hashed, table_index_len, TABLE_DUMMY, TABLE_EMPTY};

pub(super) struct TableLayout {
    key_stride: usize,
}
pub(super) const MAP_LAYOUT: TableLayout = TableLayout { key_stride: MAP_ENTRY_STRIDE };
pub(super) const SET_LAYOUT: TableLayout = TableLayout { key_stride: 1 };
impl TableLayout {
    #[inline]
    fn key_slot(&self, entry: usize) -> usize {
        1 + self.key_stride * entry
    }
    #[inline]
    fn val_slot(&self, entry: usize) -> usize {
        2 + self.key_stride * entry
    }
    fn is_map(&self) -> bool {
        self.key_stride > 1
    }
}

fn header_ref<'a>(coll: Value) -> Result<&'a ObjectHeader, String> {
    let ptr = coll.as_object_ptr().ok_or_else(|| "table: receiver is not a heap object".to_string())?;
    // SAFETY: `coll` is a live `Map` or `Set` header the caller checked; it outlives the native call.
    Ok(unsafe { &*ptr.as_ptr() })
}

fn table_parts(ctx: &NativeCallContext<'_>, coll: Value) -> Result<(Value, Value, usize), String> {
    let entries = ctx.heap.get_slot(coll, HEADER_BACKING_SLOT)?;
    let index = ctx.heap.get_slot(coll, TABLE_INDEX_SLOT)?;
    let cap = slot_count(ctx.heap, entries, BACKING_CAP_SLOT)?;
    Ok((entries, index, cap))
}

fn find(coll: Value, lay: &TableLayout, key: Value) -> Result<Option<(usize, usize)>, String> {
    // SAFETY: `header_ref` gives a live `Map` or `Set` header.
    unsafe { table_find(header_ref(coll)?, lay.key_stride, key) }
}

fn index_put(ctx: &NativeCallContext<'_>, index: Value, slot: usize, stored: u32) -> Result<(), String> {
    let base = ctx.heap.bytes_ptr(index)? as *mut u32;
    // SAFETY: `slot` is below the index's slot count; a caller has already written an entry slot, so a sealed table fails first.
    unsafe { std::ptr::write_unaligned(base.add(slot), stored) };
    Ok(())
}

fn index_place(ctx: &NativeCallContext<'_>, index: Value, slots: usize, hash: u64, stored: u32) -> Result<(), String> {
    let ptr = index.as_object_ptr().ok_or_else(|| "table: corrupt index".to_string())?;
    let mut slot = hash as usize & (slots - 1);
    // SAFETY: `index` is a live table index of `slots` slots, at most half full.
    while !matches!(unsafe { index_get(ptr.as_ref(), slot) }, TABLE_EMPTY | TABLE_DUMMY) {
        slot = (slot + 1) & (slots - 1);
    }
    index_put(ctx, index, slot, stored)
}

fn initial_capacity(hint: usize) -> usize {
    hint.max(TABLE_INITIAL_ENTRIES).next_power_of_two()
}

fn alloc_table_parts(ctx: &mut NativeCallContext<'_>, lay: &TableLayout, cap: usize) -> Result<(Value, Value), String> {
    if cap > TABLE_MAX_ENTRIES {
        return Err("a Map or Set cannot hold that many entries".to_string());
    }
    let entries = if lay.is_map() { ctx.heap.alloc_map_backing(cap)? } else { ctx.heap.alloc_backing(cap)? };
    let bytes = table_index_len(cap) * 4;
    let index = ctx.heap.alloc_bytes_backing(bytes)?;
    // SAFETY: the index payload holds `bytes` bytes.
    unsafe { std::ptr::write_bytes(ctx.heap.bytes_ptr(index)?, 0, bytes) };
    Ok((entries, index))
}

fn rebuild_table(ctx: &mut NativeCallContext<'_>, coll: Value, lay: &TableLayout, new_cap: usize) -> Result<(Value, Value, usize), String> {
    let (old_entries, old_index, _) = table_parts(ctx, coll)?;
    let used = slot_count(ctx.heap, coll, TABLE_USED_SLOT)?;
    let (entries, index) = alloc_table_parts(ctx, lay, new_cap)?;
    let slots = table_index_len(new_cap);
    let mut kept = 0;
    for i in 0..used {
        let key = ctx.heap.get_slot(old_entries, lay.key_slot(i))?;
        if key.is_null() {
            continue;
        }
        ctx.heap.set_slot(entries, lay.key_slot(kept), key)?;
        if lay.is_map() {
            let val = ctx.heap.get_slot(old_entries, lay.val_slot(i))?;
            ctx.heap.set_slot(entries, lay.val_slot(kept), val)?;
        }
        index_place(ctx, index, slots, key_hash(key)?, (kept + 1) as u32)?;
        kept += 1;
    }
    ctx.heap.set_slot(coll, HEADER_BACKING_SLOT, entries)?;
    ctx.heap.set_slot(coll, TABLE_INDEX_SLOT, index)?;
    ctx.heap.set_slot(coll, TABLE_USED_SLOT, Value::uint(kept as u64))?;
    ctx.heap.release_backing(old_entries);
    ctx.heap.release_backing(old_index);
    Ok((entries, index, new_cap))
}

pub(super) fn map_lookup(ctx: &mut NativeCallContext<'_>, map: Value, key: Value) -> Result<Option<Value>, String> {
    match find(map, &MAP_LAYOUT, key)? {
        Some((_, entry)) => {
            let entries = ctx.heap.get_slot(map, HEADER_BACKING_SLOT)?;
            Ok(Some(ctx.heap.get_slot(entries, MAP_LAYOUT.val_slot(entry))?))
        }
        None => Ok(None),
    }
}

fn table_insert(ctx: &mut NativeCallContext<'_>, coll: Value, lay: &TableLayout, key: Value, val: Value) -> Result<bool, String> {
    let hash = key_hash(key)?;
    // SAFETY: `header_ref` gives a live `Map` or `Set` header.
    if let Some((_, entry)) = unsafe { table_find_hashed(header_ref(coll)?, lay.key_stride, hash, key) } {
        if lay.is_map() {
            let entries = ctx.heap.get_slot(coll, HEADER_BACKING_SLOT)?;
            ctx.heap.set_slot(entries, lay.val_slot(entry), val)?;
        }
        return Ok(false);
    }
    let (mut entries, mut index, mut cap) = table_parts(ctx, coll)?;
    let len = slot_count(ctx.heap, coll, HEADER_LEN_SLOT)?;
    let mut used = slot_count(ctx.heap, coll, TABLE_USED_SLOT)?;
    if used == cap {
        let new_cap = if len * 2 > cap { cap * 2 } else { cap };
        (entries, index, cap) = rebuild_table(ctx, coll, lay, new_cap)?;
        used = len;
    }
    ctx.heap.set_slot(entries, lay.key_slot(used), key)?;
    if lay.is_map() {
        ctx.heap.set_slot(entries, lay.val_slot(used), val)?;
    }
    index_place(ctx, index, table_index_len(cap), hash, (used + 1) as u32)?;
    ctx.heap.set_slot(coll, TABLE_USED_SLOT, Value::uint(used as u64 + 1))?;
    ctx.heap.set_slot(coll, HEADER_LEN_SLOT, Value::uint(len as u64 + 1))?;
    Ok(true)
}

pub(super) fn map_insert(ctx: &mut NativeCallContext<'_>, map: Value, key: Value, val: Value) -> Result<(), String> {
    table_insert(ctx, map, &MAP_LAYOUT, key, val).map(|_| ())
}

pub(super) fn set_add_impl(ctx: &mut NativeCallContext<'_>, set: Value, key: Value) -> Result<bool, String> {
    table_insert(ctx, set, &SET_LAYOUT, key, Value::null())
}

pub(super) fn table_remove(ctx: &mut NativeCallContext<'_>, coll: Value, lay: &TableLayout, key: Value) -> Result<bool, String> {
    let Some((slot, entry)) = find(coll, lay, key)? else { return Ok(false) };
    let (entries, index, cap) = table_parts(ctx, coll)?;
    ctx.heap.set_slot(entries, lay.key_slot(entry), Value::null())?;
    if lay.is_map() {
        ctx.heap.set_slot(entries, lay.val_slot(entry), Value::null())?;
    }
    index_put(ctx, index, slot, TABLE_DUMMY)?;
    let len = slot_count(ctx.heap, coll, HEADER_LEN_SLOT)?.saturating_sub(1);
    ctx.heap.set_slot(coll, HEADER_LEN_SLOT, Value::uint(len as u64))?;
    if let Some(new_cap) = shrunk_capacity(len, cap) {
        rebuild_table(ctx, coll, lay, new_cap)?;
    }
    Ok(true)
}

fn table_clear(ctx: &mut NativeCallContext<'_>, coll: Value, lay: &TableLayout) -> Result<(), String> {
    ctx.heap.set_slot(coll, HEADER_LEN_SLOT, Value::uint(0))?;
    let (entries, index, cap) = table_parts(ctx, coll)?;
    if cap >= SHRINK_MIN_CAP {
        let (fresh, fresh_index) = alloc_table_parts(ctx, lay, TABLE_INITIAL_ENTRIES)?;
        ctx.heap.set_slot(coll, HEADER_BACKING_SLOT, fresh)?;
        ctx.heap.set_slot(coll, TABLE_INDEX_SLOT, fresh_index)?;
        ctx.heap.release_backing(entries);
        ctx.heap.release_backing(index);
    } else {
        let used = slot_count(ctx.heap, coll, TABLE_USED_SLOT)?;
        for slot in 1..=lay.key_stride * used {
            ctx.heap.set_slot(entries, slot, Value::null())?;
        }
        let bytes = table_index_len(cap) * 4;
        // SAFETY: the index payload holds `bytes` bytes.
        unsafe { std::ptr::write_bytes(ctx.heap.bytes_ptr(index)?, 0, bytes) };
    }
    ctx.heap.set_slot(coll, TABLE_USED_SLOT, Value::uint(0))
}

fn new_table(ctx: &mut NativeCallContext<'_>, id: u64, lay: &TableLayout, hint: usize) -> Result<Value, String> {
    let coll = ctx.heap.alloc_header(id, TABLE_HEADER_SLOTS)?;
    let (entries, index) = alloc_table_parts(ctx, lay, initial_capacity(hint))?;
    ctx.heap.set_slot(coll, HEADER_LEN_SLOT, Value::uint(0))?;
    ctx.heap.set_slot(coll, HEADER_BACKING_SLOT, entries)?;
    ctx.heap.set_slot(coll, TABLE_INDEX_SLOT, index)?;
    ctx.heap.set_slot(coll, TABLE_USED_SLOT, Value::uint(0))?;
    Ok(coll)
}

pub(super) fn map_new(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let hint = ctx.arg(0).and_then(|v| v.as_int()).unwrap_or(0).max(0) as usize;
    new_map(ctx, hint)
}

pub(super) fn new_map(ctx: &mut NativeCallContext<'_>, hint: usize) -> Result<Value, String> {
    new_table(ctx, MAP_TYPE_ID, &MAP_LAYOUT, hint)
}

pub(super) fn set_new(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let hint = ctx.arg(0).and_then(|v| v.as_int()).unwrap_or(0).max(0) as usize;
    new_table(ctx, SET_TYPE_ID, &SET_LAYOUT, hint)
}

fn table_pairs(ctx: &NativeCallContext<'_>, coll: Value, lay: &TableLayout) -> Result<Vec<(Value, Value)>, String> {
    let entries = ctx.heap.get_slot(coll, HEADER_BACKING_SLOT)?;
    let used = slot_count(ctx.heap, coll, TABLE_USED_SLOT)?;
    let mut out = Vec::with_capacity(slot_count(ctx.heap, coll, HEADER_LEN_SLOT)?);
    for i in 0..used {
        let key = ctx.heap.get_slot(entries, lay.key_slot(i))?;
        if !key.is_null() {
            let val = if lay.is_map() { ctx.heap.get_slot(entries, lay.val_slot(i))? } else { Value::null() };
            out.push((key, val));
        }
    }
    Ok(out)
}

pub(super) fn coll_remove(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let recv = ctx.arg(0).unwrap_or(Value::null());
    let key = ctx.arg(1).unwrap_or(Value::null());
    let present = if is_map(recv) {
        table_remove(ctx, recv, &MAP_LAYOUT, key)?
    } else if is_set(recv) {
        table_remove(ctx, recv, &SET_LAYOUT, key)?
    } else {
        return Err("`.remove()` is only defined on Map and Set".to_string());
    };
    Ok(vbool(present))
}

pub(super) fn coll_contains(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let recv = ctx.arg(0).unwrap_or(Value::null());
    let x = ctx.arg(1).unwrap_or(Value::null());
    let found = if is_list(recv) {
        let len = slot_count(ctx.heap, recv, HEADER_LEN_SLOT)?;
        let backing = ctx.heap.get_slot(recv, HEADER_BACKING_SLOT)?;
        let mut hit = false;
        for i in 0..len {
            if isa::equality::values_equal(ctx.heap.get_slot(backing, BACKING_DATA_BASE + i)?, x) {
                hit = true;
                break;
            }
        }
        hit
    } else if is_bytes(recv) {
        match x.as_int() {
            Some(n) if (0..=255).contains(&n) => {
                let (backing, len) = bytes_state(ctx, recv)?;
                ctx.heap.read_bytes(backing, 0, len)?.contains(&(n as u8))
            }
            _ => false,
        }
    } else if is_map(recv) {
        map_lookup(ctx, recv, x)?.is_some()
    } else if is_set(recv) {
        find(recv, &SET_LAYOUT, x)?.is_some()
    } else if let Some(s) = recv.as_heap_string() {
        let sub = x
            .as_heap_string()
            .ok_or_else(|| "string.contains expects a String argument".to_string())?;
        s.contains(&sub)
    } else {
        return Err("`.contains()` is only defined on String, List, Bytes, Map, and Set".to_string());
    };
    Ok(vbool(found))
}

pub(super) fn set_add(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let set = ctx.arg(0).unwrap_or(Value::null());
    if !is_set(set) {
        return Err("`.add()` is only defined on Set".to_string());
    }
    let x = ctx.arg(1).unwrap_or(Value::null());
    Ok(vbool(set_add_impl(ctx, set, x)?))
}

pub(super) fn build_list(ctx: &mut NativeCallContext<'_>, items: &[Value]) -> Result<Value, String> {
    let list = ctx.heap.alloc_header(LIST_TYPE_ID, 2)?;
    let backing = ctx.heap.alloc_backing(items.len().max(1))?;
    for (i, v) in items.iter().enumerate() {
        ctx.heap.set_slot(backing, BACKING_DATA_BASE + i, *v)?;
    }
    ctx.heap
        .set_slot(list, HEADER_LEN_SLOT, Value::uint(items.len() as u64))?;
    ctx.heap.set_slot(list, HEADER_BACKING_SLOT, backing)?;
    Ok(list)
}

pub(super) fn map_pairs(ctx: &mut NativeCallContext<'_>, map: Value) -> Result<Vec<(Value, Value)>, String> {
    table_pairs(ctx, map, &MAP_LAYOUT)
}

pub(super) fn map_keys(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let map = require_map(ctx, "keys")?;
    let ks: Vec<Value> = map_pairs(ctx, map)?.into_iter().map(|(k, _)| k).collect();
    build_list(ctx, &ks)
}

pub(super) fn map_values(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let map = require_map(ctx, "values")?;
    let vs: Vec<Value> = map_pairs(ctx, map)?.into_iter().map(|(_, v)| v).collect();
    build_list(ctx, &vs)
}

pub(super) fn map_entries(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let map = require_map(ctx, "entries")?;
    let pairs = map_pairs(ctx, map)?;
    let mut rows = Vec::with_capacity(pairs.len());
    for (k, v) in pairs {
        rows.push(build_list(ctx, &[k, v])?);
    }
    build_list(ctx, &rows)
}

pub(super) fn set_items(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let set = ctx.arg(0).unwrap_or(Value::null());
    if !is_set(set) {
        return Err("`.items()` is only defined on Set".to_string());
    }
    let items: Vec<Value> = table_pairs(ctx, set, &SET_LAYOUT)?.into_iter().map(|(k, _)| k).collect();
    build_list(ctx, &items)
}

pub(super) fn require_map(ctx: &NativeCallContext<'_>, method: &str) -> Result<Value, String> {
    let m = ctx.arg(0).unwrap_or(Value::null());
    if is_map(m) {
        Ok(m)
    } else {
        Err(format!("`.{method}()` is only defined on Map"))
    }
}

pub(super) const BYTES_MIN_CAP: usize = 16;

pub(super) fn require_bytes(ctx: &NativeCallContext<'_>, method: &str) -> Result<Value, String> {
    let b = ctx.arg(0).unwrap_or(Value::null());
    if is_bytes(b) {
        Ok(b)
    } else {
        Err(format!("`.{method}()` is only defined on Bytes"))
    }
}

pub(super) fn as_byte(v: Value) -> Result<u8, String> {
    match v.as_int() {
        Some(n) if (0..=255).contains(&n) => Ok(n as u8),
        _ => Err("a byte must be an Int in 0..=255".to_string()),
    }
}

pub(super) fn bytes_state(ctx: &NativeCallContext<'_>, b: Value) -> Result<(Value, usize), String> {
    let len = slot_count(ctx.heap, b, HEADER_LEN_SLOT)?;
    let backing = ctx.heap.get_slot(b, HEADER_BACKING_SLOT)?;
    Ok((backing, len))
}

pub(super) fn bytes_index(ctx: &NativeCallContext<'_>, b: Value, key: Value) -> Result<(Value, usize), String> {
    let idx = key
        .as_int()
        .ok_or_else(|| "bytes index must be an Int".to_string())?;
    let (backing, len) = bytes_state(ctx, b)?;
    if idx < 0 || idx as usize >= len {
        return Err(format!("bytes index {idx} out of bounds (len {len})"));
    }
    Ok((backing, idx as usize))
}

pub(super) fn bytes_grow_to(ctx: &mut NativeCallContext<'_>, b: Value, need: usize) -> Result<Value, String> {
    let (backing, len) = bytes_state(ctx, b)?;
    let cap = slot_count(ctx.heap, backing, BACKING_CAP_SLOT)?;
    if need <= cap {
        return Ok(backing);
    }
    let new_cap = grown_capacity(cap, 1).max(need).max(BYTES_MIN_CAP);
    let grown = ctx.heap.alloc_bytes_backing(new_cap)?;
    if len > 0 {
        let existing = ctx.heap.read_bytes(backing, 0, len)?;
        ctx.heap.write_bytes(grown, 0, &existing)?;
    }
    ctx.heap.set_slot(b, HEADER_BACKING_SLOT, grown)?;
    ctx.heap.release_backing(backing);
    Ok(grown)
}

pub(super) fn bytes_push(ctx: &mut NativeCallContext<'_>, b: Value, byte: u8) -> Result<Value, String> {
    let len = slot_count(ctx.heap, b, HEADER_LEN_SLOT)?;
    let backing = bytes_grow_to(ctx, b, len + 1)?;
    ctx.heap.write_bytes(backing, len, &[byte])?;
    ctx.heap
        .set_slot(b, HEADER_LEN_SLOT, Value::uint((len + 1) as u64))?;
    Ok(Value::null())
}

pub(super) fn bytes_new(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let hint = ctx.arg(0).and_then(|v| v.as_int()).unwrap_or(0).max(0) as usize;
    let b = ctx.heap.alloc_header(BYTES_TYPE_ID, 2)?;
    let backing = ctx.heap.alloc_bytes_backing(hint)?;
    ctx.heap.set_slot(b, HEADER_LEN_SLOT, Value::uint(0))?;
    ctx.heap.set_slot(b, HEADER_BACKING_SLOT, backing)?;
    Ok(b)
}

pub(super) fn bytes_zeros(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let n = ctx.arg(0).and_then(|v| v.as_int()).ok_or("Bytes(n): n must be an Int")?;
    let n = usize::try_from(n).map_err(|_| format!("Bytes(n): n must not be negative, got {n}"))?;
    let b = ctx.heap.alloc_header(BYTES_TYPE_ID, 2)?;
    let backing = ctx.heap.alloc_bytes_backing(n)?;
    ctx.heap.set_slot(b, HEADER_LEN_SLOT, Value::uint(n as u64))?;
    ctx.heap.set_slot(b, HEADER_BACKING_SLOT, backing)?;
    Ok(b)
}

pub(super) fn bytes_extend(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let b = require_bytes(ctx, "extend")?;
    let other = ctx.arg(1).unwrap_or(Value::null());
    if !is_bytes(other) {
        return Err("bytes.extend expects a Bytes argument".to_string());
    }
    let (ob, olen) = bytes_state(ctx, other)?;
    let src = ctx.heap.read_bytes(ob, 0, olen)?;
    let len = slot_count(ctx.heap, b, HEADER_LEN_SLOT)?;
    let backing = bytes_grow_to(ctx, b, len + src.len())?;
    ctx.heap.write_bytes(backing, len, &src)?;
    ctx.heap
        .set_slot(b, HEADER_LEN_SLOT, Value::uint((len + src.len()) as u64))?;
    Ok(Value::null())
}

pub(super) fn coll_slice(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let recv = ctx.arg(0).unwrap_or(Value::null());
    let start = ctx.arg(1).and_then(|v| v.as_int()).unwrap_or(0);

    if let Some(s) = recv.heap_bytes() {
        let end = ctx.arg(2).and_then(|v| v.as_int()).unwrap_or(s.len() as i64);
        if start < 0 || end < start || end as usize > s.len() {
            return Err(format!("string.slice({start}, {end}) out of range (len {})", s.len()));
        }
        let (a, b) = (start as usize, end as usize);
        let on_boundary = |i: usize| i == s.len() || (s[i] & 0xC0) != 0x80;
        if !on_boundary(a) || !on_boundary(b) {
            return Err(format!("string.slice({start}, {end}) is not on a UTF-8 char boundary"));
        }
        let piece = s[a..b].to_vec();
        return ctx.heap.alloc_string(&piece);
    }

    let b = require_bytes(ctx, "slice")?;
    let (backing, len) = bytes_state(ctx, b)?;
    let end = ctx.arg(2).and_then(|v| v.as_int()).unwrap_or(len as i64);
    if start < 0 || end < start || end as usize > len {
        return Err(format!("bytes.slice({start}, {end}) out of range (len {len})"));
    }
    let slice = ctx
        .heap
        .read_bytes(backing, start as usize, (end - start) as usize)?;
    let nb = ctx.heap.alloc_header(BYTES_TYPE_ID, 2)?;
    let nbacking = ctx.heap.alloc_bytes_backing(slice.len())?;
    ctx.heap.write_bytes(nbacking, 0, &slice)?;
    ctx.heap
        .set_slot(nb, HEADER_LEN_SLOT, Value::uint(slice.len() as u64))?;
    ctx.heap.set_slot(nb, HEADER_BACKING_SLOT, nbacking)?;
    Ok(nb)
}

pub(super) fn bytes_is_utf8(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let b = require_bytes(ctx, "decode")?;
    let (backing, len) = bytes_state(ctx, b)?;
    let bytes = ctx.heap.read_bytes(backing, 0, len)?;
    Ok(vbool(std::str::from_utf8(&bytes).is_ok()))
}

pub(super) fn bytes_decode(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let b = require_bytes(ctx, "decode")?;
    let (backing, len) = bytes_state(ctx, b)?;
    let bytes = ctx.heap.read_bytes(backing, 0, len)?;
    std::str::from_utf8(&bytes).map_err(|e| format!("bytes.decode: invalid UTF-8: {e}"))?;
    ctx.heap.alloc_string(&bytes)
}

