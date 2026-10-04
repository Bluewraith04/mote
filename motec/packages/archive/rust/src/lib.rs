//! tar and zip, behind the bytes-in, bytes-out C convention of `std.dev.libtools`.
//!
//! Entries cross as frames, one after another: `<name length> <data length>\n`, the name, then the data (lengths in decimal bytes).

use std::io::{Cursor, Read, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::slice;

const MAX_OUTPUT: u64 = 1 << 30;

type Entries = Vec<(String, Vec<u8>)>;

/// Runs `work` on the input and writes its answer to `out`: the answer's length (nothing is written when it exceeds `cap`), or the negated length of an error message written to `out`.
fn call(input: *const u8, len: i64, out: *mut u8, cap: i64, work: impl FnOnce(&[u8]) -> Result<Vec<u8>, String>) -> i64 {
    let bytes: &[u8] = if len <= 0 || input.is_null() { &[] } else { unsafe { slice::from_raw_parts(input, len as usize) } };
    let answer = catch_unwind(AssertUnwindSafe(|| work(bytes))).unwrap_or_else(|_| Err("the library failed".to_string()));
    let (data, sign) = match answer {
        Ok(data) => (data, 1),
        Err(message) => (if message.is_empty() { b"failed".to_vec() } else { message.into_bytes() }, -1),
    };
    let fits = data.len() as i64 <= cap;
    if (fits || sign < 0) && cap > 0 {
        let n = data.len().min(cap as usize);
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), out, n) };
    }
    sign * data.len() as i64
}

fn read_capped(r: impl Read, budget: &mut u64) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let limit = *budget;
    r.take(limit + 1).read_to_end(&mut out).map_err(|e| e.to_string())?;
    if out.len() as u64 > limit {
        return Err("decompressed data is larger than 1 GiB".to_string());
    }
    *budget -= out.len() as u64;
    Ok(out)
}

fn safe_name(name: &str) -> bool {
    let trimmed = name.trim_end_matches('/');
    let drive = trimmed.len() >= 2 && trimmed.as_bytes()[1] == b':' && trimmed.as_bytes()[0].is_ascii_alphabetic();
    !trimmed.is_empty()
        && !name.starts_with('/')
        && !name.contains('\\')
        && !name.contains('\0')
        && !drive
        && trimmed.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

fn number(bytes: &[u8], at: &mut usize, stop: u8) -> Result<usize, String> {
    let mut n = 0usize;
    loop {
        let b = *bytes.get(*at).ok_or("a frame is cut short")?;
        *at += 1;
        if b == stop {
            return Ok(n);
        }
        if !b.is_ascii_digit() {
            return Err("a frame header is not a number".to_string());
        }
        n = n.checked_mul(10).and_then(|n| n.checked_add((b - b'0') as usize)).ok_or("a frame is too long")?;
    }
}

fn read_frames(bytes: &[u8]) -> Result<Entries, String> {
    let mut at = 0;
    let mut out = Vec::new();
    while at < bytes.len() {
        let name_len = number(bytes, &mut at, b' ')?;
        let data_len = number(bytes, &mut at, b'\n')?;
        let name_end = at.checked_add(name_len).filter(|e| *e <= bytes.len()).ok_or("a frame is cut short")?;
        let data_end = name_end.checked_add(data_len).filter(|e| *e <= bytes.len()).ok_or("a frame is cut short")?;
        let name = String::from_utf8(bytes[at..name_end].to_vec()).map_err(|_| "an entry name is not text".to_string())?;
        if !safe_name(&name) {
            return Err(format!("unsafe entry name {name}"));
        }
        out.push((name, bytes[name_end..data_end].to_vec()));
        at = data_end;
    }
    Ok(out)
}

fn write_frames(entries: Entries) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, data) in entries {
        out.extend_from_slice(format!("{} {}\n", name.len(), data.len()).as_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&data);
    }
    out
}

fn tar_pack(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, data) in read_frames(input)? {
        let mut header = tar::Header::new_gnu();
        let dir = name.ends_with('/');
        header.set_entry_type(if dir { tar::EntryType::Directory } else { tar::EntryType::Regular });
        header.set_mode(if dir { 0o755 } else { 0o644 });
        header.set_mtime(0);
        header.set_size(if dir { 0 } else { data.len() as u64 });
        let body: &[u8] = if dir { &[] } else { &data };
        builder.append_data(&mut header, &name, body).map_err(|e| e.to_string())?;
    }
    builder.into_inner().map_err(|e| e.to_string())
}

fn tar_unpack(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut archive = tar::Archive::new(Cursor::new(input));
    let mut budget = MAX_OUTPUT;
    let mut out = Vec::new();
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.header().entry_type();
        let mut name = entry.path().map_err(|e| e.to_string())?.to_string_lossy().into_owned();
        if kind.is_dir() {
            if !name.ends_with('/') {
                name.push('/');
            }
        } else if !kind.is_file() {
            continue;
        }
        if !safe_name(&name) {
            return Err(format!("unsafe entry name {name}"));
        }
        let body = if kind.is_dir() { Vec::new() } else { read_capped(&mut entry, &mut budget)? };
        out.push((name, body));
    }
    Ok(write_frames(out))
}

fn zip_pack(input: &[u8], level: i64) -> Result<Vec<u8>, String> {
    let level = level.clamp(0, 9);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(if level == 0 { zip::CompressionMethod::Stored } else { zip::CompressionMethod::Deflated })
        .compression_level(Some(level))
        .unix_permissions(0o644);
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in read_frames(input)? {
        if name.ends_with('/') {
            writer.add_directory(&name, options.unix_permissions(0o755)).map_err(|e| e.to_string())?;
        } else {
            writer.start_file(&name, options).map_err(|e| e.to_string())?;
            writer.write_all(&data).map_err(|e| e.to_string())?;
        }
    }
    Ok(writer.finish().map_err(|e| e.to_string())?.into_inner())
}

fn zip_unpack(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(input)).map_err(|e| e.to_string())?;
    let mut budget = MAX_OUTPUT;
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let mut name = file.name().to_string();
        let dir = file.is_dir();
        if dir && !name.ends_with('/') {
            name.push('/');
        }
        if !safe_name(&name) {
            return Err(format!("unsafe entry name {name}"));
        }
        let body = if dir { Vec::new() } else { read_capped(&mut file, &mut budget)? };
        out.push((name, body));
    }
    Ok(write_frames(out))
}

/// Frames in, a tar archive out.
///
/// # Safety
/// `input` must be readable for `len` bytes and `out` writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_tar_pack(input: *const u8, len: i64, out: *mut u8, cap: i64) -> i64 {
    call(input, len, out, cap, tar_pack)
}

/// A tar archive in, frames out; links and devices are skipped.
///
/// # Safety
/// `input` must be readable for `len` bytes and `out` writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_tar_unpack(input: *const u8, len: i64, out: *mut u8, cap: i64) -> i64 {
    call(input, len, out, cap, tar_unpack)
}

/// Frames in, a zip archive out, deflated at `level` (0 stores).
///
/// # Safety
/// `input` must be readable for `len` bytes and `out` writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_zip_pack(input: *const u8, len: i64, out: *mut u8, cap: i64, level: i64) -> i64 {
    call(input, len, out, cap, |bytes| zip_pack(bytes, level))
}

/// A zip archive in, frames out.
///
/// # Safety
/// `input` must be readable for `len` bytes and `out` writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_zip_unpack(input: *const u8, len: i64, out: *mut u8, cap: i64) -> i64 {
    call(input, len, out, cap, zip_unpack)
}
