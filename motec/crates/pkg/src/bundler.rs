use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use compiler::CompiledProgram;
use modules::MultiFileCompiler;

/// Appends a compiled program to a copy of the `mote` binary.
pub struct StandaloneBundler;

const TRAILER_LEN: u64 = 8 + 15;

impl StandaloneBundler {
    pub const MAGIC_TRAILER: &'static [u8; 15] = b"MOTE_PAYLOAD_V1";

    /// Compiles `entry_mote_path` and bundles the result into a standalone binary.
    pub fn bundle(entry_mote_path: &Path, output_exe_path: &Path) -> Result<(), String> {
        let root_dir = entry_mote_path
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();
        let mut compiler = MultiFileCompiler::new(root_dir);
        let compiled = compiler.compile_program(entry_mote_path)?;
        Self::bundle_program(&compiled, output_exe_path)
    }

    /// Bundles an already-compiled program into a copy of the running `mote` binary.
    pub fn bundle_program(
        compiled: &CompiledProgram,
        output_exe_path: &Path,
    ) -> Result<(), String> {
        let current_exe = std::env::current_exe()
            .map_err(|e| format!("Failed to locate current executable: {}", e))?;
        let host_bytes = fs::read(&current_exe)
            .map_err(|e| format!("Failed to read host executable '{:?}': {}", current_exe, e))?;
        let bytes = Self::assemble(host_bytes, &compiled.to_bytes());

        if let Some(parent) = output_exe_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create output directory: {}", e))?;
        }

        let tmp_path = output_exe_path.with_extension("mote-tmp");
        let mut out_file = File::create(&tmp_path)
            .map_err(|e| format!("Failed to create output executable '{:?}': {}", output_exe_path, e))?;
        out_file
            .write_all(&bytes)
            .map_err(|e| format!("Failed to write output executable '{:?}': {}", output_exe_path, e))?;
        out_file.flush().map_err(|e| e.to_string())?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o755)).ok();
        }
        drop(out_file);
        fs::rename(&tmp_path, output_exe_path).map_err(|e| {
            fs::remove_file(&tmp_path).ok();
            format!("Failed to write output executable '{:?}': {}", output_exe_path, e)
        })
    }

    /// `host` without its signature and old payload, then zero padding, `payload` and the trailer,
    /// ending on an 8-byte boundary so a PE certificate table can follow.
    pub fn assemble(mut host: Vec<u8>, payload: &[u8]) -> Vec<u8> {
        if let Some((entry, offset, _)) = pe_certificate_table(&host) {
            host.truncate(offset as usize);
            host[entry..entry + 8].fill(0);
            let opt = u32::from_le_bytes(host[0x3C..0x40].try_into().unwrap()) as usize + 24;
            host[opt + 64..opt + 68].fill(0);
        }
        if let Some((start, _)) = find_payload(&host, host.len() as u64) {
            host.truncate(start as usize);
        }
        let unpadded = host.len() as u64 + payload.len() as u64 + TRAILER_LEN;
        host.resize(host.len() + (unpadded.next_multiple_of(8) - unpadded) as usize, 0);
        host.extend_from_slice(payload);
        host.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        host.extend_from_slice(Self::MAGIC_TRAILER);
        host
    }

    /// Inspects the running binary to check for an appended bytecode payload.
    pub fn detect_and_read_payload() -> Option<Result<CompiledProgram, String>> {
        Self::read_payload(&std::env::current_exe().ok()?)
    }

    /// The program embedded in the executable at `exe_path`, if any.
    pub fn read_payload(exe_path: &Path) -> Option<Result<CompiledProgram, String>> {
        let mut file = File::open(exe_path).ok()?;
        let total_len = file.metadata().ok()?.len();

        let mut header = vec![0u8; 4096.min(total_len as usize)];
        file.read_exact(&mut header).ok()?;
        let end = match pe_certificate_table(&header) {
            Some((_, offset, _)) if u64::from(offset) <= total_len => u64::from(offset),
            _ => total_len,
        };
        if end < TRAILER_LEN {
            return None;
        }
        let mut trailer = [0u8; TRAILER_LEN as usize];
        file.seek(SeekFrom::Start(end - TRAILER_LEN)).ok()?;
        file.read_exact(&mut trailer).ok()?;
        let (start, len) = find_payload(&trailer, end)?;
        if start > end {
            return Some(Err("Corrupted payload length in standalone executable".to_string()));
        }

        if file.seek(SeekFrom::Start(start)).is_err() {
            return Some(Err("Failed to seek to embedded payload offset".to_string()));
        }
        let mut payload_buf = vec![0u8; len as usize];
        if let Err(e) = file.read_exact(&mut payload_buf) {
            return Some(Err(format!("Failed to read embedded payload: {}", e)));
        }
        Some(CompiledProgram::from_bytes(&payload_buf))
    }
}

fn find_payload(tail: &[u8], end: u64) -> Option<(u64, u64)> {
    let n = tail.len();
    if (n as u64) < TRAILER_LEN || &tail[n - 15..] != StandaloneBundler::MAGIC_TRAILER {
        return None;
    }
    let len = u64::from_le_bytes(tail[n - 23..n - 15].try_into().ok()?);
    Some((end.checked_sub(TRAILER_LEN + len).unwrap_or(u64::MAX), len))
}

/// For a PE image: the file offset of its certificate-table directory entry, and the table's offset and size.
pub fn pe_certificate_table(bytes: &[u8]) -> Option<(usize, u32, u32)> {
    let u16_at = |at: usize| bytes.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if bytes.get(..2)? != b"MZ" {
        return None;
    }
    let pe = u32_at(0x3C)? as usize;
    if bytes.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let opt = pe + 24;
    let (count_at, dirs_at) = match u16_at(opt)? {
        0x10b => (opt + 92, opt + 96),
        0x20b => (opt + 108, opt + 112),
        _ => return None,
    };
    if u32_at(count_at)? <= 4 {
        return None;
    }
    let entry = dirs_at + 4 * 8;
    let (offset, size) = (u32_at(entry)?, u32_at(entry + 4)?);
    (offset != 0 && size != 0).then_some((entry, offset, size))
}
