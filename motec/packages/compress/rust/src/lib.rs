//! gzip, zlib and raw deflate, behind the bytes-in, bytes-out C convention of `std.dev.libtools`.

use std::io::{Read, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::slice;

const MAX_OUTPUT: u64 = 1 << 30;

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

fn read_capped(r: impl Read) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    r.take(MAX_OUTPUT + 1).read_to_end(&mut out).map_err(|e| e.to_string())?;
    if out.len() as u64 > MAX_OUTPUT {
        return Err("decompressed data is larger than 1 GiB".to_string());
    }
    Ok(out)
}

fn compress(data: &[u8], mode: i64, level: i64) -> Result<Vec<u8>, String> {
    use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
    let level = flate2::Compression::new(level.clamp(0, 9) as u32);
    let done = match mode {
        0 => {
            let mut e = GzEncoder::new(Vec::new(), level);
            e.write_all(data).and_then(|()| e.finish())
        }
        1 => {
            let mut e = ZlibEncoder::new(Vec::new(), level);
            e.write_all(data).and_then(|()| e.finish())
        }
        _ => {
            let mut e = DeflateEncoder::new(Vec::new(), level);
            e.write_all(data).and_then(|()| e.finish())
        }
    };
    done.map_err(|e| e.to_string())
}

fn decompress(data: &[u8], mode: i64) -> Result<Vec<u8>, String> {
    use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
    match mode {
        0 => read_capped(GzDecoder::new(data)),
        1 => read_capped(ZlibDecoder::new(data)),
        _ => read_capped(DeflateDecoder::new(data)),
    }
}

/// Compresses the input: `mode` 0 gzip, 1 zlib, 2 raw deflate; `level` 0 (store) to 9 (smallest).
///
/// # Safety
/// `input` must be readable for `len` bytes and `out` writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_compress(input: *const u8, len: i64, out: *mut u8, cap: i64, mode: i64, level: i64) -> i64 {
    call(input, len, out, cap, |bytes| compress(bytes, mode, level))
}

/// Decompresses the input: `mode` 0 gzip, 1 zlib, 2 raw deflate; stops with an error past 1 GiB.
///
/// # Safety
/// `input` must be readable for `len` bytes and `out` writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_decompress(input: *const u8, len: i64, out: *mut u8, cap: i64, mode: i64) -> i64 {
    call(input, len, out, cap, |bytes| decompress(bytes, mode))
}
