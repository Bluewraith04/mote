//! The Rust backend of `archive`: tar and zip read into entries by handle, and written entry by entry.
#![allow(clippy::missing_safety_doc)]
use std::io::{Cursor, Read, Write};
use tar::{Archive, Builder, EntryType, Header};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

const TAR: i64 = 0;
const ZIP: i64 = 1;

const FILE: i64 = 0;
const DIRECTORY: i64 = 1;
const SYMLINK: i64 = 2;
const HARDLINK: i64 = 3;

unsafe fn bytes<'a>(pointer: *const u8, length: i64) -> &'a [u8] {
    if length <= 0 { &[] } else { unsafe { std::slice::from_raw_parts(pointer, length as usize) } }
}

unsafe fn give(value: &[u8], out: *mut u8, capacity: i64) {
    if capacity > 0 {
        unsafe { std::ptr::copy_nonoverlapping(value.as_ptr(), out, value.len().min(capacity as usize)) };
    }
}

unsafe fn fail(message: &str, error: *mut u8, capacity: i64) -> i64 {
    unsafe { give(message.as_bytes(), error, capacity) };
    -(message.len() as i64) - 1
}

struct Item {
    kind: i64,
    mode: i64,
    mtime: i64,
    path: String,
    target: String,
    data: Vec<u8>,
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146097 + day_of_era - 719468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719468;
    let era = days.div_euclid(146097);
    let day_of_era = days - era * 146097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 { shifted + 3 } else { shifted - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn zip_time(mtime: i64) -> DateTime {
    let (year, month, day) = civil_from_days(mtime.div_euclid(86400));
    let seconds = mtime.rem_euclid(86400);
    DateTime::from_date_and_time(year as u16, month as u8, day as u8, (seconds / 3600) as u8, (seconds / 60 % 60) as u8, (seconds % 60) as u8).unwrap_or_default()
}

fn unix_time(time: DateTime) -> i64 {
    days_from_civil(i64::from(time.year()), i64::from(time.month()), i64::from(time.day())) * 86400
        + i64::from(time.hour()) * 3600
        + i64::from(time.minute()) * 60
        + i64::from(time.second())
}

/// Reads up to what is left of the budget; fails when the entry is bigger.
fn read_limited(reader: &mut impl Read, budget: &mut i64) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    let allowed = if *budget < 0 { u64::MAX } else { *budget as u64 };
    reader.take(allowed.saturating_add(1)).read_to_end(&mut data).map_err(|e| e.to_string())?;
    if data.len() as u64 > allowed {
        return Err("contents are over the limit".to_string());
    }
    if *budget >= 0 {
        *budget -= data.len() as i64;
    }
    Ok(data)
}

fn read_tar(data: &[u8], mut budget: i64) -> Result<Vec<Item>, String> {
    let mut archive = Archive::new(Cursor::new(data));
    let mut items = Vec::new();
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let kind = match entry.header().entry_type() {
            EntryType::Regular | EntryType::Continuous => FILE,
            EntryType::Directory => DIRECTORY,
            EntryType::Symlink => SYMLINK,
            EntryType::Link => HARDLINK,
            _ => continue,
        };
        let path = entry.path().map_err(|e| e.to_string())?.to_string_lossy().into_owned();
        let target = match entry.link_name().map_err(|e| e.to_string())? {
            Some(target) => target.to_string_lossy().into_owned(),
            None => String::new(),
        };
        let mode = i64::from(entry.header().mode().map_err(|e| e.to_string())?);
        let mtime = entry.header().mtime().map_err(|e| e.to_string())? as i64;
        let data = if kind == FILE { read_limited(&mut entry, &mut budget)? } else { Vec::new() };
        items.push(Item { kind, mode, mtime, path, target, data });
    }
    Ok(items)
}

fn read_zip(data: &[u8], mut budget: i64) -> Result<Vec<Item>, String> {
    let mut archive = ZipArchive::new(Cursor::new(data)).map_err(|e| e.to_string())?;
    let mut items = Vec::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|e| e.to_string())?;
        let path = file.name().trim_end_matches('/').to_string();
        let mtime = file.last_modified().map(unix_time).unwrap_or(0);
        let default = if file.is_dir() { 0o755 } else { 0o644 };
        let mode = i64::from(file.unix_mode().unwrap_or(default) & 0o7777);
        if file.is_dir() {
            items.push(Item { kind: DIRECTORY, mode, mtime, path, target: String::new(), data: Vec::new() });
        } else if file.is_symlink() {
            let target = String::from_utf8_lossy(&read_limited(&mut file, &mut budget)?).into_owned();
            items.push(Item { kind: SYMLINK, mode, mtime, path, target, data: Vec::new() });
        } else {
            let data = read_limited(&mut file, &mut budget)?;
            items.push(Item { kind: FILE, mode, mtime, path, target: String::new(), data });
        }
    }
    Ok(items)
}

unsafe fn items<'a>(handle: i64) -> &'a [Item] {
    unsafe { &*(handle as *const Vec<Item>) }
}

/// Reads a whole archive of format 0 (tar) or 1 (zip) whose contents may total `limit` bytes (negative for no limit).
/// Returns a handle, or `-length - 1` with the message in `error`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn archive_read(format: i64, data: *const u8, length: i64, limit: i64, error: *mut u8, capacity: i64) -> i64 {
    let data = unsafe { bytes(data, length) };
    let read = match format {
        TAR => read_tar(data, limit),
        ZIP => read_zip(data, limit),
        _ => Err("unknown format".to_string()),
    };
    match read {
        Ok(items) => Box::into_raw(Box::new(items)) as i64,
        Err(message) => unsafe { fail(&message, error, capacity) },
    }
}

/// How many entries were read.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn archive_count(handle: i64) -> i64 {
    unsafe { items(handle) }.len() as i64
}

/// Writes kind, mode, mtime, path length, target length and data length of an entry to `out`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn archive_info(handle: i64, index: i64, out: *mut i64, length: i64) -> i64 {
    let Some(item) = unsafe { items(handle) }.get(index as usize) else { return -1 };
    let fields = [item.kind, item.mode, item.mtime, item.path.len() as i64, item.target.len() as i64, item.data.len() as i64];
    for (at, field) in fields.iter().enumerate().take(length.max(0) as usize) {
        unsafe { *out.add(at) = *field };
    }
    0
}

/// Copies the path (what 0), link target (1) or contents (2) of an entry to `out`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn archive_field(handle: i64, index: i64, what: i64, out: *mut u8, capacity: i64) -> i64 {
    let Some(item) = unsafe { items(handle) }.get(index as usize) else { return -1 };
    let value: &[u8] = match what {
        0 => item.path.as_bytes(),
        1 => item.target.as_bytes(),
        _ => &item.data,
    };
    unsafe { give(value, out, capacity) };
    value.len() as i64
}

/// Frees a read archive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn archive_free(handle: i64) -> i64 {
    if handle != 0 {
        drop(unsafe { Box::from_raw(handle as *mut Vec<Item>) });
    }
    0
}

enum Writer {
    Tar(Builder<Vec<u8>>),
    Zip(Box<ZipWriter<Cursor<Vec<u8>>>>),
    Done(Vec<u8>),
}

/// Starts writing an archive of format 0 (tar) or 1 (zip). Returns a handle, or 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn archive_writer_new(format: i64) -> i64 {
    let writer = match format {
        TAR => Writer::Tar(Builder::new(Vec::new())),
        ZIP => Writer::Zip(Box::new(ZipWriter::new(Cursor::new(Vec::new())))),
        _ => return 0,
    };
    Box::into_raw(Box::new(writer)) as i64
}

fn add_tar(builder: &mut Builder<Vec<u8>>, kind: i64, mode: i64, mtime: i64, path: &str, target: &str, data: &[u8]) -> Result<(), String> {
    let mut header = Header::new_gnu();
    header.set_mode(mode as u32);
    header.set_mtime(mtime.max(0) as u64);
    header.set_uid(0);
    header.set_gid(0);
    match kind {
        FILE => {
            header.set_entry_type(EntryType::Regular);
            header.set_size(data.len() as u64);
            builder.append_data(&mut header, path, data)
        }
        DIRECTORY => {
            header.set_entry_type(EntryType::Directory);
            header.set_size(0);
            builder.append_data(&mut header, path, std::io::empty())
        }
        SYMLINK | HARDLINK => {
            header.set_entry_type(if kind == SYMLINK { EntryType::Symlink } else { EntryType::Link });
            header.set_size(0);
            builder.append_link(&mut header, path, target)
        }
        _ => return Err("unknown entry kind".to_string()),
    }
    .map_err(|e| e.to_string())
}

fn add_zip(writer: &mut ZipWriter<Cursor<Vec<u8>>>, kind: i64, mode: i64, mtime: i64, path: &str, target: &str, data: &[u8]) -> Result<(), String> {
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(mode as u32)
        .last_modified_time(zip_time(mtime));
    match kind {
        FILE => writer.start_file(path, options).map_err(|e| e.to_string()).and_then(|()| writer.write_all(data).map_err(|e| e.to_string())),
        DIRECTORY => writer.add_directory(path, options).map_err(|e| e.to_string()),
        SYMLINK => writer.add_symlink(path, target, options).map_err(|e| e.to_string()),
        _ => Err("zip has no such entry kind".to_string()),
    }
}

/// Adds an entry. Returns 0, or `-length - 1` with the message in `error`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn archive_writer_add(
    handle: i64,
    kind: i64,
    mode: i64,
    mtime: i64,
    path: *const u8,
    path_length: i64,
    target: *const u8,
    target_length: i64,
    data: *const u8,
    data_length: i64,
    error: *mut u8,
    capacity: i64,
) -> i64 {
    let (path, target, data) = unsafe { (bytes(path, path_length), bytes(target, target_length), bytes(data, data_length)) };
    let (Ok(path), Ok(target)) = (std::str::from_utf8(path), std::str::from_utf8(target)) else {
        return unsafe { fail("a path is not UTF-8", error, capacity) };
    };
    let result = match unsafe { &mut *(handle as *mut Writer) } {
        Writer::Tar(builder) => add_tar(builder, kind, mode, mtime, path, target, data),
        Writer::Zip(writer) => add_zip(writer, kind, mode, mtime, path, target, data),
        Writer::Done(_) => Err("the archive is finished".to_string()),
    };
    match result {
        Ok(()) => 0,
        Err(message) => unsafe { fail(&message, error, capacity) },
    }
}

/// Ends the archive. Returns its size in bytes, or `-length - 1` with the message in `error`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn archive_writer_finish(handle: i64, error: *mut u8, capacity: i64) -> i64 {
    let slot = unsafe { &mut *(handle as *mut Writer) };
    let finished = match std::mem::replace(slot, Writer::Done(Vec::new())) {
        Writer::Tar(builder) => builder.into_inner().map_err(|e| e.to_string()),
        Writer::Zip(writer) => (*writer).finish().map(Cursor::into_inner).map_err(|e| e.to_string()),
        Writer::Done(data) => Ok(data),
    };
    match finished {
        Ok(data) => {
            let size = data.len() as i64;
            *slot = Writer::Done(data);
            size
        }
        Err(message) => unsafe { fail(&message, error, capacity) },
    }
}

/// Copies the finished archive to `out`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn archive_writer_take(handle: i64, out: *mut u8, capacity: i64) -> i64 {
    if let Writer::Done(data) = unsafe { &*(handle as *const Writer) } {
        unsafe { give(data, out, capacity) };
        return data.len() as i64;
    }
    -1
}

/// Frees a writer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn archive_writer_free(handle: i64) -> i64 {
    if handle != 0 {
        drop(unsafe { Box::from_raw(handle as *mut Writer) });
    }
    0
}
