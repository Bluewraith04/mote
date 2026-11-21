//! The Rust backend of `compress`: gzip, zlib and raw deflate coders by handle, with the output drained by `compress_take`.
#![allow(clippy::missing_safety_doc)]
use flate2::write::{DeflateEncoder, GzEncoder, MultiGzDecoder, ZlibEncoder};
use flate2::{Compression, Decompress, FlushDecompress, Status};
use std::io::Write;

const GZIP: i64 = 0;
const ZLIB: i64 = 1;
const DEFLATE: i64 = 2;

unsafe fn bytes<'a>(pointer: *const u8, length: i64) -> &'a [u8] {
    if length <= 0 { &[] } else { unsafe { std::slice::from_raw_parts(pointer, length as usize) } }
}

unsafe fn give(value: &[u8], out: *mut u8, capacity: i64) {
    if capacity > 0 {
        unsafe { std::ptr::copy_nonoverlapping(value.as_ptr(), out, value.len().min(capacity as usize)) };
    }
}

/// The output so far; a write that takes it past the limit fails.
struct Sink {
    buf: Vec<u8>,
    limit: usize,
}

impl Write for Sink {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.buf.len() + data.len() > self.limit {
            return Err(std::io::Error::other("output is over the limit"));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

enum Coder {
    GzipEncoder(GzEncoder<Sink>),
    ZlibEncoder(ZlibEncoder<Sink>),
    DeflateEncoder(DeflateEncoder<Sink>),
    GzipDecoder(MultiGzDecoder<Sink>),
    Inflater { state: Decompress, sink: Sink, ended: bool },
}

impl Coder {
    fn sink(&mut self) -> &mut Sink {
        match self {
            Coder::GzipEncoder(coder) => coder.get_mut(),
            Coder::ZlibEncoder(coder) => coder.get_mut(),
            Coder::DeflateEncoder(coder) => coder.get_mut(),
            Coder::GzipDecoder(coder) => coder.get_mut(),
            Coder::Inflater { sink, .. } => sink,
        }
    }

    fn push(&mut self, data: &[u8]) -> Result<(), String> {
        match self {
            Coder::GzipEncoder(coder) => coder.write_all(data).map_err(|e| e.to_string()),
            Coder::ZlibEncoder(coder) => coder.write_all(data).map_err(|e| e.to_string()),
            Coder::DeflateEncoder(coder) => coder.write_all(data).map_err(|e| e.to_string()),
            Coder::GzipDecoder(coder) => coder.write_all(data).map_err(|e| e.to_string()),
            Coder::Inflater { state, sink, ended } => inflate(state, sink, ended, data),
        }
    }

    fn finish(&mut self) -> Result<(), String> {
        match self {
            Coder::GzipEncoder(coder) => coder.try_finish().map_err(|e| e.to_string()),
            Coder::ZlibEncoder(coder) => coder.try_finish().map_err(|e| e.to_string()),
            Coder::DeflateEncoder(coder) => coder.try_finish().map_err(|e| e.to_string()),
            Coder::GzipDecoder(coder) => coder.try_finish().map_err(|e| e.to_string()),
            Coder::Inflater { ended, .. } => if *ended { Ok(()) } else { Err("the stream ends early".to_string()) },
        }
    }
}

fn inflate(state: &mut Decompress, sink: &mut Sink, ended: &mut bool, mut input: &[u8]) -> Result<(), String> {
    while !input.is_empty() {
        if *ended {
            return Err("data after the end of the stream".to_string());
        }
        if sink.buf.len() > sink.limit {
            return Err("output is over the limit".to_string());
        }
        sink.buf.reserve(32 * 1024);
        let before = state.total_in();
        let status = state.decompress_vec(input, &mut sink.buf, FlushDecompress::None).map_err(|e| e.to_string())?;
        input = &input[(state.total_in() - before) as usize..];
        if status == Status::StreamEnd {
            *ended = true;
        }
    }
    if sink.buf.len() > sink.limit {
        return Err("output is over the limit".to_string());
    }
    Ok(())
}

unsafe fn coder<'a>(handle: i64) -> &'a mut Coder {
    unsafe { &mut *(handle as *mut Coder) }
}

unsafe fn fail(message: &str, error: *mut u8, capacity: i64) -> i64 {
    unsafe { give(message.as_bytes(), error, capacity) };
    -(message.len() as i64) - 1
}

/// Starts an encoder of format 0 (gzip), 1 (zlib) or 2 (raw deflate) at level 0 to 9. Returns a handle, or 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compress_encoder_new(format: i64, level: i64) -> i64 {
    if !(0..=9).contains(&level) {
        return 0;
    }
    let level = Compression::new(level as u32);
    let sink = Sink { buf: Vec::new(), limit: usize::MAX };
    let coder = match format {
        GZIP => Coder::GzipEncoder(GzEncoder::new(sink, level)),
        ZLIB => Coder::ZlibEncoder(ZlibEncoder::new(sink, level)),
        DEFLATE => Coder::DeflateEncoder(DeflateEncoder::new(sink, level)),
        _ => return 0,
    };
    Box::into_raw(Box::new(coder)) as i64
}

/// Starts a decoder whose output may not pass `limit` bytes (a negative limit has none). Returns a handle, or 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compress_decoder_new(format: i64, limit: i64) -> i64 {
    let sink = Sink { buf: Vec::new(), limit: if limit < 0 { usize::MAX } else { limit as usize } };
    let coder = match format {
        GZIP => Coder::GzipDecoder(MultiGzDecoder::new(sink)),
        ZLIB => Coder::Inflater { state: Decompress::new(true), sink, ended: false },
        DEFLATE => Coder::Inflater { state: Decompress::new(false), sink, ended: false },
        _ => return 0,
    };
    Box::into_raw(Box::new(coder)) as i64
}

/// Feeds bytes to a coder. Returns 0, or `-length - 1` with the message in `error`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compress_push(handle: i64, data: *const u8, length: i64, error: *mut u8, capacity: i64) -> i64 {
    match unsafe { coder(handle) }.push(unsafe { bytes(data, length) }) {
        Ok(()) => 0,
        Err(message) => unsafe { fail(&message, error, capacity) },
    }
}

/// Ends the input: an encoder writes its trailer, a decoder checks that the stream is whole. Returns 0, or `-length - 1`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compress_finish(handle: i64, error: *mut u8, capacity: i64) -> i64 {
    match unsafe { coder(handle) }.finish() {
        Ok(()) => 0,
        Err(message) => unsafe { fail(&message, error, capacity) },
    }
}

/// How many output bytes wait to be taken.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compress_pending(handle: i64) -> i64 {
    unsafe { coder(handle) }.sink().buf.len() as i64
}

/// Moves up to `capacity` output bytes to `out` and returns how many.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compress_take(handle: i64, out: *mut u8, capacity: i64) -> i64 {
    let sink = unsafe { coder(handle) }.sink();
    let count = sink.buf.len().min(capacity.max(0) as usize);
    unsafe { give(&sink.buf[..count], out, count as i64) };
    sink.buf.drain(..count);
    count as i64
}

/// Frees a coder.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compress_free(handle: i64) -> i64 {
    if handle != 0 {
        drop(unsafe { Box::from_raw(handle as *mut Coder) });
    }
    0
}
