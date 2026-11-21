//! The Rust backend of `regex`: compiled expressions by handle, matches as byte offsets.
#![allow(clippy::missing_safety_doc)]
use regex::Regex;

const FAILED: i64 = -1;

unsafe fn bytes<'a>(pointer: *const u8, length: i64) -> &'a [u8] {
    if length <= 0 { &[] } else { unsafe { std::slice::from_raw_parts(pointer, length as usize) } }
}

unsafe fn give(value: &[u8], out: *mut u8, capacity: i64) {
    if capacity > 0 {
        unsafe { std::ptr::copy_nonoverlapping(value.as_ptr(), out, value.len().min(capacity as usize)) };
    }
}

unsafe fn regex<'a>(handle: i64) -> &'a Regex {
    unsafe { &*(handle as *const Regex) }
}

/// Compiles a pattern. Returns a handle, or `-length - 1` with the message in `out`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn regex_new(pattern: *const u8, length: i64, out: *mut u8, capacity: i64) -> i64 {
    let Ok(pattern) = std::str::from_utf8(unsafe { bytes(pattern, length) }) else {
        unsafe { give(b"not UTF-8", out, capacity) };
        return -9 - 1;
    };
    match Regex::new(pattern) {
        Ok(compiled) => Box::into_raw(Box::new(compiled)) as i64,
        Err(error) => {
            let message = error.to_string();
            unsafe { give(message.as_bytes(), out, capacity) };
            -(message.len() as i64) - 1
        }
    }
}

/// Frees a compiled pattern.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn regex_free(handle: i64) -> i64 {
    if handle != 0 {
        drop(unsafe { Box::from_raw(handle as *mut Regex) });
    }
    0
}

/// How many groups the pattern has, the whole match included.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn regex_groups(handle: i64) -> i64 {
    unsafe { regex(handle) }.captures_len() as i64
}

/// The name of a group to `out`; returns its length, or -1 for a group with no name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn regex_group_name(handle: i64, index: i64, out: *mut u8, capacity: i64) -> i64 {
    let compiled = unsafe { regex(handle) };
    match compiled.capture_names().nth(index.max(0) as usize).flatten() {
        Some(name) => {
            unsafe { give(name.as_bytes(), out, capacity) };
            name.len() as i64
        }
        None => FAILED,
    }
}

/// The first match at or after byte `start`. Writes the start and end of each group as two little-endian 32-bit numbers (-1 for a group that did not take part) and returns the number of groups, or -1 for no match.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn regex_captures(handle: i64, text: *const u8, length: i64, start: i64, out: *mut u8, capacity: i64) -> i64 {
    let compiled = unsafe { regex(handle) };
    let Ok(text) = std::str::from_utf8(unsafe { bytes(text, length) }) else { return FAILED };
    let start = start.max(0) as usize;
    if start > text.len() || !text.is_char_boundary(start) {
        return FAILED;
    }
    let Some(found) = compiled.captures_at(text, start) else { return FAILED };
    let mut spans = Vec::with_capacity(found.len() * 8);
    for group in 0..found.len() {
        let (from, to) = found.get(group).map_or((-1i32, -1i32), |span| (span.start() as i32, span.end() as i32));
        spans.extend_from_slice(&from.to_le_bytes());
        spans.extend_from_slice(&to.to_le_bytes());
    }
    unsafe { give(&spans, out, capacity) };
    found.len() as i64
}

/// Whether the pattern matches anywhere in the text.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn regex_is_match(handle: i64, text: *const u8, length: i64) -> i64 {
    let compiled = unsafe { regex(handle) };
    match std::str::from_utf8(unsafe { bytes(text, length) }) {
        Ok(text) => i64::from(compiled.is_match(text)),
        Err(_) => FAILED,
    }
}
