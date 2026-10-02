//! Compression and archive natives over `flate2`, `tar` and `zip`.

use std::io::{Cursor, Read, Write};

use super::formats::{bytes_arg, invalid};
use super::*;

const MAX_OUTPUT: u64 = 1 << 30;

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

fn list_items(ctx: &NativeCallContext<'_>, idx: usize) -> Result<Vec<Value>, String> {
    let list = ctx.arg(idx).ok_or("missing list argument")?;
    let len = slot_count(ctx.heap, list, HEADER_LEN_SLOT)?;
    let backing = ctx.heap.get_slot(list, HEADER_BACKING_SLOT)?;
    (0..len).map(|i| ctx.heap.get_slot(backing, BACKING_DATA_BASE + i)).collect()
}

fn entries_arg(ctx: &NativeCallContext<'_>) -> Result<Vec<(String, Vec<u8>)>, NativeError> {
    let names = string_list_arg(ctx, 0, "pack")?;
    let datas = list_items(ctx, 1)?;
    if names.len() != datas.len() {
        return Err(invalid("pack needs one data value per name"));
    }
    let mut out = Vec::with_capacity(names.len());
    for (name, data) in names.into_iter().zip(datas) {
        if !safe_name(&name) {
            return Err(invalid(format!("unsafe entry name {name}")));
        }
        let (backing, len) = bytes_state(ctx, data)?;
        out.push((name, ctx.heap.read_bytes(backing, 0, len)?.to_vec()));
    }
    Ok(out)
}

fn alloc_entries(ctx: &mut NativeCallContext<'_>, entries: Vec<(String, Vec<u8>)>) -> Result<Value, NativeError> {
    let mut flat = Vec::with_capacity(entries.len() * 2);
    for (name, data) in entries {
        flat.push(ctx.heap.alloc_string(name.as_bytes())?);
        flat.push(alloc_bytes(ctx, &data)?);
    }
    Ok(alloc_list(ctx, &flat)?)
}

pub(super) fn compress(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
    let data = bytes_arg(ctx, 0)?;
    let level = flate2::Compression::new(arg_int(ctx, 2).clamp(0, 9) as u32);
    let out = match arg_int(ctx, 1) {
        0 => GzEncoder::new(Vec::new(), level).write_all_finish(&data),
        1 => ZlibEncoder::new(Vec::new(), level).write_all_finish(&data),
        _ => DeflateEncoder::new(Vec::new(), level).write_all_finish(&data),
    }
    .map_err(|e| e.to_string())?;
    alloc_bytes(ctx, &out)
}

trait WriteAllFinish {
    fn write_all_finish(self, data: &[u8]) -> std::io::Result<Vec<u8>>;
}

macro_rules! finish_with {
    ($($encoder:ty),*) => {$(
        impl WriteAllFinish for $encoder {
            fn write_all_finish(mut self, data: &[u8]) -> std::io::Result<Vec<u8>> {
                self.write_all(data)?;
                self.finish()
            }
        }
    )*};
}
finish_with!(
    flate2::write::GzEncoder<Vec<u8>>,
    flate2::write::ZlibEncoder<Vec<u8>>,
    flate2::write::DeflateEncoder<Vec<u8>>
);

pub(super) fn decompress(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
    let data = bytes_arg(ctx, 0)?;
    let mut budget = MAX_OUTPUT;
    let out = match arg_int(ctx, 1) {
        0 => read_capped(GzDecoder::new(&data[..]), &mut budget),
        1 => read_capped(ZlibDecoder::new(&data[..]), &mut budget),
        _ => read_capped(DeflateDecoder::new(&data[..]), &mut budget),
    }
    .map_err(invalid)?;
    Ok(alloc_bytes(ctx, &out)?)
}

pub(super) fn tar_pack(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let entries = entries_arg(ctx)?;
    let mut builder = tar::Builder::new(Vec::new());
    for (name, data) in entries {
        let mut header = tar::Header::new_gnu();
        let dir = name.ends_with('/');
        header.set_entry_type(if dir { tar::EntryType::Directory } else { tar::EntryType::Regular });
        header.set_mode(if dir { 0o755 } else { 0o644 });
        header.set_mtime(0);
        header.set_size(if dir { 0 } else { data.len() as u64 });
        let body: &[u8] = if dir { &[] } else { &data };
        builder.append_data(&mut header, &name, body).map_err(|e| invalid(e.to_string()))?;
    }
    let out = builder.into_inner().map_err(|e| invalid(e.to_string()))?;
    Ok(alloc_bytes(ctx, &out)?)
}

pub(super) fn tar_unpack(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let data = bytes_arg(ctx, 0)?;
    let mut archive = tar::Archive::new(Cursor::new(data));
    let mut budget = MAX_OUTPUT;
    let mut out = Vec::new();
    for entry in archive.entries().map_err(|e| invalid(e.to_string()))? {
        let mut entry = entry.map_err(|e| invalid(e.to_string()))?;
        let kind = entry.header().entry_type();
        let mut name = entry.path().map_err(|e| invalid(e.to_string()))?.to_string_lossy().into_owned();
        if kind.is_dir() {
            if !name.ends_with('/') {
                name.push('/');
            }
        } else if !kind.is_file() {
            continue;
        }
        if !safe_name(&name) {
            return Err(invalid(format!("unsafe entry name {name}")));
        }
        let body = if kind.is_dir() { Vec::new() } else { read_capped(&mut entry, &mut budget).map_err(invalid)? };
        out.push((name, body));
    }
    alloc_entries(ctx, out)
}

pub(super) fn zip_pack(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let entries = entries_arg(ctx)?;
    let level = arg_int(ctx, 2).clamp(0, 9);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(if level == 0 { zip::CompressionMethod::Stored } else { zip::CompressionMethod::Deflated })
        .compression_level(Some(level))
        .unix_permissions(0o644);
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in entries {
        if name.ends_with('/') {
            writer.add_directory(&name, options.unix_permissions(0o755)).map_err(|e| invalid(e.to_string()))?;
        } else {
            writer.start_file(&name, options).map_err(|e| invalid(e.to_string()))?;
            writer.write_all(&data).map_err(|e| invalid(e.to_string()))?;
        }
    }
    let out = writer.finish().map_err(|e| invalid(e.to_string()))?.into_inner();
    Ok(alloc_bytes(ctx, &out)?)
}

pub(super) fn zip_unpack(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let data = bytes_arg(ctx, 0)?;
    let mut archive = zip::ZipArchive::new(Cursor::new(data)).map_err(|e| invalid(e.to_string()))?;
    let mut budget = MAX_OUTPUT;
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| invalid(e.to_string()))?;
        let mut name = file.name().to_string();
        let dir = file.is_dir();
        if dir && !name.ends_with('/') {
            name.push('/');
        }
        if !safe_name(&name) {
            return Err(invalid(format!("unsafe entry name {name}")));
        }
        let body = if dir { Vec::new() } else { read_capped(&mut file, &mut budget).map_err(invalid)? };
        out.push((name, body));
    }
    alloc_entries(ctx, out)
}
