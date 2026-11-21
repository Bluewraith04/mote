//! The Rust backend of `crypto`: SHA-2 hashers by handle, HMAC, HKDF and a constant-time compare.
#![allow(clippy::missing_safety_doc)]
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256, Sha384, Sha512};
use subtle::ConstantTimeEq;

unsafe fn bytes<'a>(pointer: *const u8, length: i64) -> &'a [u8] {
    if length <= 0 { &[] } else { unsafe { std::slice::from_raw_parts(pointer, length as usize) } }
}

unsafe fn give(value: &[u8], out: *mut u8, capacity: i64) {
    if capacity > 0 {
        unsafe { std::ptr::copy_nonoverlapping(value.as_ptr(), out, value.len().min(capacity as usize)) };
    }
}

enum Hasher {
    S256(Sha256),
    S384(Sha384),
    S512(Sha512),
}

fn hmac_with<M: Mac + KeyInit>(key: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    let mut mac = <M as KeyInit>::new_from_slice(key).expect("HMAC takes any key length");
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().to_vec()
}

fn mac(algorithm: i64, key: &[u8], parts: &[&[u8]]) -> Option<Vec<u8>> {
    match algorithm {
        0 => Some(hmac_with::<Hmac<Sha256>>(key, parts)),
        1 => Some(hmac_with::<Hmac<Sha384>>(key, parts)),
        2 => Some(hmac_with::<Hmac<Sha512>>(key, parts)),
        _ => None,
    }
}

/// Starts a hasher of algorithm 0 (SHA-256), 1 (SHA-384) or 2 (SHA-512). Returns a handle, or 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn crypto_hasher_new(algorithm: i64) -> i64 {
    let hasher = match algorithm {
        0 => Hasher::S256(Sha256::new()),
        1 => Hasher::S384(Sha384::new()),
        2 => Hasher::S512(Sha512::new()),
        _ => return 0,
    };
    Box::into_raw(Box::new(hasher)) as i64
}

/// Feeds bytes to a hasher.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn crypto_hasher_update(handle: i64, data: *const u8, length: i64) -> i64 {
    let data = unsafe { bytes(data, length) };
    match unsafe { &mut *(handle as *mut Hasher) } {
        Hasher::S256(state) => state.update(data),
        Hasher::S384(state) => state.update(data),
        Hasher::S512(state) => state.update(data),
    }
    0
}

/// Writes the digest to `out`, frees the hasher and returns the digest length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn crypto_hasher_finish(handle: i64, out: *mut u8, capacity: i64) -> i64 {
    let digest = match *unsafe { Box::from_raw(handle as *mut Hasher) } {
        Hasher::S256(state) => state.finalize().to_vec(),
        Hasher::S384(state) => state.finalize().to_vec(),
        Hasher::S512(state) => state.finalize().to_vec(),
    };
    unsafe { give(&digest, out, capacity) };
    digest.len() as i64
}

/// Frees a hasher that was not finished.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn crypto_hasher_free(handle: i64) -> i64 {
    if handle != 0 {
        drop(unsafe { Box::from_raw(handle as *mut Hasher) });
    }
    0
}

/// Writes the HMAC of `data` under `key` to `out` and returns its length, or -1 for an unknown algorithm.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn crypto_hmac(algorithm: i64, key: *const u8, key_length: i64, data: *const u8, data_length: i64, out: *mut u8, capacity: i64) -> i64 {
    let (key, data) = unsafe { (bytes(key, key_length), bytes(data, data_length)) };
    match mac(algorithm, key, &[data]) {
        Some(tag) => {
            unsafe { give(&tag, out, capacity) };
            tag.len() as i64
        }
        None => -1,
    }
}

/// HKDF (RFC 5869) of `length` bytes into `out`; returns `length`, or -1 when it is more than 255 hashes long.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn crypto_hkdf(
    algorithm: i64,
    salt: *const u8,
    salt_length: i64,
    secret: *const u8,
    secret_length: i64,
    info: *const u8,
    info_length: i64,
    length: i64,
    out: *mut u8,
    capacity: i64,
) -> i64 {
    let (salt, secret, info) = unsafe { (bytes(salt, salt_length), bytes(secret, secret_length), bytes(info, info_length)) };
    let hash_length = match algorithm {
        0 => 32,
        1 => 48,
        2 => 64,
        _ => return -1,
    };
    if length < 0 || length > 255 * hash_length as i64 {
        return -1;
    }
    let salt = if salt.is_empty() { vec![0u8; hash_length] } else { salt.to_vec() };
    let key = mac(algorithm, &salt, &[secret]).expect("algorithm checked");
    let mut okm: Vec<u8> = Vec::with_capacity(length as usize);
    let mut block: Vec<u8> = Vec::new();
    let mut counter = 1u8;
    while (okm.len() as i64) < length {
        block = mac(algorithm, &key, &[&block, info, &[counter]]).expect("algorithm checked");
        okm.extend_from_slice(&block);
        counter = counter.wrapping_add(1);
    }
    okm.truncate(length as usize);
    unsafe { give(&okm, out, capacity) };
    length
}

/// 1 when the two byte strings are equal, else 0; the time depends only on the lengths.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn crypto_equal(a: *const u8, a_length: i64, b: *const u8, b_length: i64) -> i64 {
    let (a, b) = unsafe { (bytes(a, a_length), bytes(b, b_length)) };
    i64::from(bool::from(a.ct_eq(b)))
}
