use std::fs;
use std::path::Path;

use compiler::CompiledProgram;

/// On-disk container for a compiled Mote program — a `.mbc` file.
///
/// [`CompiledProgram::to_bytes`] is the encoding of the program *structure*; this
/// wrapper adds file-level framing: a magic number, a format version distinct
/// from the structure's own version byte, an explicit payload length, and a
/// CRC-32 integrity check.
///
/// Layout (all integers little-endian):
///
/// ```text
/// offset  size  field
/// 0       5     magic            b"MOTEC"
/// 5       2     format version   u16, currently 2
/// 7       4     payload length   u32
/// 11      4     payload CRC-32    u32 (IEEE, reflected)
/// 15      N     payload          CompiledProgram::to_bytes()
/// ```
pub struct MbcFile;

impl MbcFile {
    pub const MAGIC: &'static [u8; 5] = b"MOTEC";
    pub(crate) const FORMAT_VERSION: u16 = 3;
    const HEADER_LEN: usize = 15;

    /// Serialize `program` into the `.mbc` container format.
    pub fn encode(program: &CompiledProgram) -> Vec<u8> {
        let payload = program.to_bytes();
        let mut buf = Vec::with_capacity(Self::HEADER_LEN + payload.len());
        buf.extend_from_slice(Self::MAGIC);
        buf.extend_from_slice(&Self::FORMAT_VERSION.to_le_bytes());
        buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        buf.extend_from_slice(&crc32(&payload).to_le_bytes());
        buf.extend_from_slice(&payload);
        buf
    }

    /// Parse a `.mbc` container, verifying magic, version, length and checksum
    /// before decoding the program.
    pub fn decode(bytes: &[u8]) -> Result<CompiledProgram, String> {
        if bytes.len() < Self::HEADER_LEN {
            return Err(format!(
                "not a .mbc file: {} bytes, shorter than the {}-byte header",
                bytes.len(),
                Self::HEADER_LEN
            ));
        }
        if &bytes[0..5] != Self::MAGIC {
            return Err("not a .mbc file: bad magic number".into());
        }
        let version = u16::from_le_bytes([bytes[5], bytes[6]]);
        if version != Self::FORMAT_VERSION {
            return Err(format!(
                "unsupported .mbc format version {} (this build reads version {})",
                version,
                Self::FORMAT_VERSION
            ));
        }
        let payload_len = u32::from_le_bytes([bytes[7], bytes[8], bytes[9], bytes[10]]) as usize;
        let expected_crc = u32::from_le_bytes([bytes[11], bytes[12], bytes[13], bytes[14]]);
        let payload = &bytes[Self::HEADER_LEN..];
        if payload.len() != payload_len {
            return Err(format!(
                ".mbc payload length mismatch: header says {payload_len}, file has {}",
                payload.len()
            ));
        }
        let actual_crc = crc32(payload);
        if actual_crc != expected_crc {
            return Err(format!(
                ".mbc checksum mismatch: expected {expected_crc:08x}, computed {actual_crc:08x} — file is corrupt"
            ));
        }
        CompiledProgram::from_bytes(payload)
    }

    /// Encode `program` and write it to `path`.
    pub fn write(program: &CompiledProgram, path: &Path) -> Result<(), String> {
        fs::write(path, Self::encode(program))
            .map_err(|e| format!("failed to write '{}': {}", path.display(), e))
    }

    /// Read and decode a `.mbc` file at `path`.
    pub fn read(path: &Path) -> Result<CompiledProgram, String> {
        let bytes = fs::read(path)
            .map_err(|e| format!("failed to read '{}': {}", path.display(), e))?;
        Self::decode(&bytes)
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::{crc32, MbcFile};

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn round_trips_a_compiled_program() {
        let program =
            compiler::Compiler::compile("fn main() -> Int { return 7 + 8 }", "t.mote").unwrap();
        let bytes = MbcFile::encode(&program);
        assert_eq!(&bytes[0..5], MbcFile::MAGIC);

        let back = MbcFile::decode(&bytes).unwrap();
        assert_eq!(back.code_objects.len(), program.code_objects.len());
        assert_eq!(back.type_descriptors.len(), program.type_descriptors.len());
    }

    #[test]
    fn rejects_a_corrupted_payload() {
        let program = compiler::Compiler::compile("fn main() -> Int { return 1 }", "t.mote").unwrap();
        let mut bytes = MbcFile::encode(&program);
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        let err = MbcFile::decode(&bytes).unwrap_err();
        assert!(err.contains("checksum mismatch"), "got: {err}");
    }

    #[test]
    fn rejects_a_foreign_file() {
        let err = MbcFile::decode(b"this is not a mote bytecode file at all").unwrap_err();
        assert!(err.contains("bad magic"), "got: {err}");
    }
}
