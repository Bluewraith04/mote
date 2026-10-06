//! `.mpk` package archives: a gzip-compressed ustar archive, sorted and timestamp-free
//! so the same package always produces the same bytes.

use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use flate2::read::GzDecoder;
use flate2::{Compression, GzBuilder};

const BLOCK: usize = 512;

/// Reading and writing `.mpk` archives.
pub struct MpkArchive;

impl MpkArchive {
    /// Gzip-compressed ustar bytes of `entries` (path, contents), sorted by path.
    pub fn encode(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
        let mut sorted: Vec<&(String, Vec<u8>)> = entries.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        let mut tar = Vec::new();
        for (path, data) in sorted {
            tar.extend_from_slice(&header(path, data.len())?);
            tar.extend_from_slice(data);
            tar.resize(tar.len().next_multiple_of(BLOCK), 0);
        }
        tar.resize(tar.len() + 2 * BLOCK, 0);
        let mut gz = GzBuilder::new().mtime(0).write(Vec::new(), Compression::default());
        gz.write_all(&tar).map_err(|e| e.to_string())?;
        gz.finish().map_err(|e| e.to_string())
    }

    /// The regular-file entries of an `.mpk`, in archive order.
    pub fn decode(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
        let mut tar = Vec::new();
        GzDecoder::new(bytes).read_to_end(&mut tar).map_err(|e| format!("not an .mpk archive: {e}"))?;
        let mut out = Vec::new();
        let mut at = 0;
        while at + BLOCK <= tar.len() {
            let h = &tar[at..at + BLOCK];
            if h.iter().all(|&b| b == 0) {
                break;
            }
            let stored = octal(&h[148..156])?;
            let sum: u64 = h.iter().enumerate().map(|(i, &b)| if (148..156).contains(&i) { 32 } else { b as u64 }).sum();
            if stored != sum {
                return Err(".mpk header checksum mismatch — file is corrupt".to_string());
            }
            let size = octal(&h[124..136])? as usize;
            let name = field(&h[0..100]);
            let prefix = field(&h[345..500]);
            let path = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            let start = at + BLOCK;
            let data = tar.get(start..start + size).ok_or(".mpk entry runs past the end — file is truncated")?;
            if matches!(h[156], b'0' | 0) {
                out.push((path, data.to_vec()));
            }
            at = start + size.next_multiple_of(BLOCK);
        }
        Ok(out)
    }

    pub fn write(entries: &[(String, Vec<u8>)], path: &Path) -> Result<(), String> {
        let bytes = Self::encode(entries)?;
        fs::write(path, bytes).map_err(|e| format!("Failed to write '{}': {}", path.display(), e))
    }

    pub fn read(path: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
        let bytes = fs::read(path).map_err(|e| format!("Failed to read '{}': {}", path.display(), e))?;
        Self::decode(&bytes)
    }
}

fn header(path: &str, size: usize) -> Result<[u8; BLOCK], String> {
    let (prefix, name) = split_path(path)?;
    let mut h = [0u8; BLOCK];
    h[..name.len()].copy_from_slice(name.as_bytes());
    put_octal(&mut h[100..108], 0o644);
    put_octal(&mut h[108..116], 0);
    put_octal(&mut h[116..124], 0);
    put_octal(&mut h[124..136], size as u64);
    put_octal(&mut h[136..148], 0);
    h[156] = b'0';
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
    h[148..156].fill(b' ');
    let sum: u64 = h.iter().map(|&b| b as u64).sum();
    h[148..154].copy_from_slice(format!("{sum:06o}").as_bytes());
    h[154] = 0;
    Ok(h)
}

fn split_path(path: &str) -> Result<(&str, &str), String> {
    if path.len() <= 100 {
        return Ok(("", path));
    }
    path.match_indices('/')
        .map(|(i, _)| (&path[..i], &path[i + 1..]))
        .find(|(p, n)| p.len() <= 155 && n.len() <= 100 && !n.is_empty())
        .ok_or_else(|| format!("path too long for an .mpk entry: {path}"))
}

fn put_octal(dst: &mut [u8], v: u64) {
    let digits = dst.len() - 1;
    dst[..digits].copy_from_slice(format!("{v:0digits$o}").as_bytes());
    dst[digits] = 0;
}

fn octal(src: &[u8]) -> Result<u64, String> {
    let s: String = src.iter().take_while(|&&b| b != 0).map(|&b| b as char).collect();
    let s = s.trim();
    if s.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(s, 8).map_err(|_| ".mpk header has a bad number field — file is corrupt".to_string())
}

fn field(src: &[u8]) -> String {
    String::from_utf8_lossy(&src[..src.iter().position(|&b| b == 0).unwrap_or(src.len())]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_is_reproducible() {
        let long = format!("src/{}/deep.mote", "d".repeat(120));
        let entries = vec![
            ("b.txt".to_string(), b"second".to_vec()),
            ("a.txt".to_string(), vec![7u8; 1000]),
            (long.clone(), b"x".to_vec()),
        ];
        let bytes = MpkArchive::encode(&entries).unwrap();
        assert_eq!(bytes, MpkArchive::encode(&entries).unwrap());
        let back = MpkArchive::decode(&bytes).unwrap();
        let names: Vec<&str> = back.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(names, ["a.txt", "b.txt", long.as_str()]);
        assert_eq!(back[0].1, vec![7u8; 1000]);
    }

    #[test]
    fn rejects_garbage_and_long_names() {
        assert!(MpkArchive::decode(b"not gzip").unwrap_err().contains("not an .mpk"));
        assert!(MpkArchive::encode(&[("x".repeat(300), vec![])]).unwrap_err().contains("too long"));
    }
}
