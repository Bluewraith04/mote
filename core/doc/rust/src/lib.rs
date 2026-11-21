//! The native of `doc`: documents by handle, nodes by id, strings as bytes the caller owns.
#![allow(clippy::missing_safety_doc)]
pub mod model;

use model::{Doc, NONE};

const FAILED: i64 = -1;

unsafe fn bytes<'a>(pointer: *const u8, length: i64) -> &'a [u8] {
    if length <= 0 { &[] } else { unsafe { std::slice::from_raw_parts(pointer, length as usize) } }
}

unsafe fn doc<'a>(handle: i64) -> &'a mut Doc {
    unsafe { &mut *(handle as *mut Doc) }
}

/// The id as a node of the document, or `None` for a number that is not one.
fn node(doc: &Doc, id: i64) -> Option<u32> {
    (0..doc.len() as i64).contains(&id).then_some(id as u32)
}

fn id_or_none(id: u32) -> i64 {
    if id == NONE { FAILED } else { i64::from(id) }
}

/// Copies up to `capacity` bytes to `out` and gives the full length.
unsafe fn give(value: &[u8], out: *mut u8, capacity: i64) -> i64 {
    if capacity > 0 {
        let count = value.len().min(capacity as usize);
        unsafe { std::ptr::copy_nonoverlapping(value.as_ptr(), out, count) };
    }
    value.len() as i64
}

/// A new document holding only its document node (id 0). Returns a handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_new() -> i64 {
    Box::into_raw(Box::new(Doc::new())) as i64
}

/// Frees a document.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_free(handle: i64) -> i64 {
    if handle != 0 {
        drop(unsafe { Box::from_raw(handle as *mut Doc) });
    }
    0
}

/// How many nodes the document has, detached ones too.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_len(handle: i64) -> i64 {
    unsafe { doc(handle) }.len() as i64
}

/// A new element, not yet in the tree. Returns its id.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_element(handle: i64, name: *const u8, name_length: i64) -> i64 {
    i64::from(unsafe { doc(handle) }.element(unsafe { bytes(name, name_length) }))
}

/// A new text node. Returns its id.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_text_node(handle: i64, text: *const u8, length: i64) -> i64 {
    i64::from(unsafe { doc(handle) }.text(unsafe { bytes(text, length) }))
}

/// A new comment node. Returns its id.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_comment(handle: i64, text: *const u8, length: i64) -> i64 {
    i64::from(unsafe { doc(handle) }.comment(unsafe { bytes(text, length) }))
}

/// A new node of HTML text written as it is. Returns its id.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_raw(handle: i64, text: *const u8, length: i64) -> i64 {
    i64::from(unsafe { doc(handle) }.raw(unsafe { bytes(text, length) }))
}

/// Puts `child` in `parent` before `before` (-1 for the end); 0, or -1 when that would break the tree.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_insert(handle: i64, parent: i64, child: i64, before: i64) -> i64 {
    let doc = unsafe { doc(handle) };
    let (Some(parent), Some(child)) = (node(doc, parent), node(doc, child)) else { return FAILED };
    let before = if before < 0 { NONE } else { match node(doc, before) { Some(found) => found, None => return FAILED } };
    if doc.insert(parent, child, before) { 0 } else { FAILED }
}

/// Takes a node off its parent.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_detach(handle: i64, id: i64) -> i64 {
    let doc = unsafe { doc(handle) };
    match node(doc, id) {
        Some(id) if id != 0 => {
            doc.detach(id);
            0
        }
        _ => FAILED,
    }
}

/// Sets the text of a text or comment node.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_set_text(handle: i64, id: i64, text: *const u8, length: i64) -> i64 {
    let doc = unsafe { doc(handle) };
    match node(doc, id) {
        Some(id) if doc.set_text(id, unsafe { bytes(text, length) }) => 0,
        _ => FAILED,
    }
}

/// Renames an element.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_set_name(handle: i64, id: i64, name: *const u8, length: i64) -> i64 {
    let doc = unsafe { doc(handle) };
    match node(doc, id) {
        Some(id) if doc.set_name(id, unsafe { bytes(name, length) }) => 0,
        _ => FAILED,
    }
}

/// Sets an attribute of an element, adding it after the others when it is new.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_set_attr(handle: i64, id: i64, name: *const u8, name_length: i64, value: *const u8, value_length: i64) -> i64 {
    let doc = unsafe { doc(handle) };
    match node(doc, id) {
        Some(id) if doc.set_attr(id, unsafe { bytes(name, name_length) }, unsafe { bytes(value, value_length) }) => 0,
        _ => FAILED,
    }
}

/// Removes an attribute; -1 when the element had none of the name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_remove_attr(handle: i64, id: i64, name: *const u8, name_length: i64) -> i64 {
    let doc = unsafe { doc(handle) };
    match node(doc, id) {
        Some(id) if doc.remove_attr(id, unsafe { bytes(name, name_length) }) => 0,
        _ => FAILED,
    }
}

/// The kind of a node: 0 document, 1 element, 2 text, 3 comment, 4 raw HTML; -1 for no such node.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_kind(handle: i64, id: i64) -> i64 {
    let doc = unsafe { doc(handle) };
    node(doc, id).map_or(FAILED, |id| i64::from(doc.kind(id)))
}

/// The parent of a node, or -1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_parent(handle: i64, id: i64) -> i64 {
    let doc = unsafe { doc(handle) };
    node(doc, id).map_or(FAILED, |id| id_or_none(doc.parent(id)))
}

/// How many children a node has, or -1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_child_count(handle: i64, id: i64) -> i64 {
    let doc = unsafe { doc(handle) };
    node(doc, id).map_or(FAILED, |id| doc.children(id).len() as i64)
}

/// The child of a node at an index, or -1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_child(handle: i64, id: i64, index: i64) -> i64 {
    let doc = unsafe { doc(handle) };
    let Some(id) = node(doc, id) else { return FAILED };
    usize::try_from(index).ok().and_then(|index| doc.children(id).get(index)).map_or(FAILED, |&child| i64::from(child))
}

/// How many attributes an element has, or -1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_attr_count(handle: i64, id: i64) -> i64 {
    let doc = unsafe { doc(handle) };
    node(doc, id).map_or(FAILED, |id| doc.attr_count(id) as i64)
}

/// The name of an element to `out`; returns its length, or -1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_name(handle: i64, id: i64, out: *mut u8, capacity: i64) -> i64 {
    let doc = unsafe { doc(handle) };
    node(doc, id).map_or(FAILED, |id| unsafe { give(doc.name(id), out, capacity) })
}

/// The text of a text or comment node to `out`; returns its length, or -1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_text(handle: i64, id: i64, out: *mut u8, capacity: i64) -> i64 {
    let doc = unsafe { doc(handle) };
    node(doc, id).map_or(FAILED, |id| unsafe { give(doc.text_of(id), out, capacity) })
}

/// The value of the attribute of a name to `out`; returns its length, or -1 when there is none.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_attr(handle: i64, id: i64, name: *const u8, name_length: i64, out: *mut u8, capacity: i64) -> i64 {
    let doc = unsafe { doc(handle) };
    let Some(id) = node(doc, id) else { return FAILED };
    doc.attr(id, unsafe { bytes(name, name_length) }).map_or(FAILED, |value| unsafe { give(value, out, capacity) })
}

/// The name of the attribute at an index to `out`; returns its length, or -1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_attr_name(handle: i64, id: i64, index: i64, out: *mut u8, capacity: i64) -> i64 {
    let doc = unsafe { doc(handle) };
    let Some(id) = node(doc, id) else { return FAILED };
    usize::try_from(index).ok().and_then(|index| doc.attr_at(id, index)).map_or(FAILED, |(name, _)| unsafe { give(name, out, capacity) })
}

/// The value of the attribute at an index to `out`; returns its length, or -1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_attr_value(handle: i64, id: i64, index: i64, out: *mut u8, capacity: i64) -> i64 {
    let doc = unsafe { doc(handle) };
    let Some(id) = node(doc, id) else { return FAILED };
    usize::try_from(index).ok().and_then(|index| doc.attr_at(id, index)).map_or(FAILED, |(_, value)| unsafe { give(value, out, capacity) })
}

/// Copies the subtree at `id` of another document into this one, unattached. Returns the id of the copy, or -1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_import(handle: i64, other: i64, id: i64) -> i64 {
    let into = unsafe { doc(handle) };
    if handle == other {
        return node(into, id).map_or(FAILED, |id| i64::from(into.duplicate(id)));
    }
    let from = unsafe { doc(other) };
    node(from, id).map_or(FAILED, |id| i64::from(into.import(from, id)))
}

/// The byte form to `out`; returns its length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_bytes(handle: i64, out: *mut u8, capacity: i64) -> i64 {
    let form = unsafe { doc(handle) }.to_bytes();
    unsafe { give(&form, out, capacity) }
}

/// A document read from its byte form. Returns a handle, or -1 when the bytes break its rules.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doc_from_bytes(data: *const u8, length: i64) -> i64 {
    match Doc::from_bytes(unsafe { bytes(data, length) }) {
        Ok(doc) => Box::into_raw(Box::new(doc)) as i64,
        Err(_) => FAILED,
    }
}
