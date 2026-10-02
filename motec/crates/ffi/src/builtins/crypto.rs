//! Hashing, randomness and password natives over the RustCrypto, `getrandom` and `rcgen` crates.

use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{Error as PasswordError, PasswordHasher, PasswordVerifier};
use argon2::Argon2;
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256, Sha384, Sha512};
use subtle::ConstantTimeEq;

use super::formats::{bytes_arg, invalid, text_arg};
use super::*;

const MAX_RANDOM: i64 = 64 << 20;

pub(super) fn crypto_hash(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let data = bytes_arg(ctx, 1)?;
    let digest = match arg_int(ctx, 0) {
        0 => Sha256::digest(&data).to_vec(),
        1 => Sha384::digest(&data).to_vec(),
        _ => Sha512::digest(&data).to_vec(),
    };
    alloc_bytes(ctx, &digest)
}

fn mac<M: Mac + KeyInit>(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut m = <M as KeyInit>::new_from_slice(key).expect("HMAC takes a key of any length");
    m.update(data);
    m.finalize().into_bytes().to_vec()
}

pub(super) fn crypto_hmac(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let key = bytes_arg(ctx, 1)?;
    let data = bytes_arg(ctx, 2)?;
    let tag = match arg_int(ctx, 0) {
        0 => mac::<Hmac<Sha256>>(&key, &data),
        1 => mac::<Hmac<Sha384>>(&key, &data),
        _ => mac::<Hmac<Sha512>>(&key, &data),
    };
    alloc_bytes(ctx, &tag)
}

pub(super) fn crypto_equal(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let a = bytes_arg(ctx, 0)?;
    let b = bytes_arg(ctx, 1)?;
    Ok(Value::Bool(bool::from(a.as_slice().ct_eq(b.as_slice()))))
}

pub(super) fn crypto_random(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let n = arg_int(ctx, 0);
    if !(0..=MAX_RANDOM).contains(&n) {
        return Err(invalid(format!("random_bytes takes 0 to {MAX_RANDOM} bytes, not {n}")));
    }
    let mut buf = vec![0u8; n as usize];
    getrandom::fill(&mut buf).map_err(|e| invalid(format!("no secure random source: {e}")))?;
    Ok(alloc_bytes(ctx, &buf)?)
}

pub(super) fn hex_encode(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let data = bytes_arg(ctx, 0)?;
    ctx.heap.alloc_string(hex::encode(data).as_bytes())
}

pub(super) fn hex_decode(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let text = text_arg(ctx, 0);
    let data = hex::decode(text.as_bytes()).map_err(|e| invalid(format!("not hexadecimal: {e}")))?;
    Ok(alloc_bytes(ctx, &data)?)
}

pub(super) fn password_hash(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let password = text_arg(ctx, 0);
    let hash = Argon2::default().hash_password(password.as_bytes()).map_err(|e| invalid(format!("cannot hash the password: {e}")))?;
    Ok(ctx.heap.alloc_string(hash.to_string().as_bytes())?)
}

pub(super) fn password_verify(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let password = text_arg(ctx, 0);
    let text = text_arg(ctx, 1);
    let parsed = PasswordHash::new(&text).map_err(|e| invalid(format!("not a password hash: {e}")))?;
    match Argon2::default().verify_password(password.as_bytes(), &parsed) {
        Ok(()) => Ok(Value::Bool(true)),
        Err(PasswordError::PasswordInvalid) => Ok(Value::Bool(false)),
        Err(e) => Err(invalid(format!("cannot check the password: {e}"))),
    }
}

pub(super) fn tls_self_signed(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let names = string_list_arg(ctx, 0, "tls_self_signed")?;
    if names.is_empty() {
        return Err(invalid("a certificate needs at least one name"));
    }
    let made = rcgen::generate_simple_self_signed(names).map_err(|e| invalid(format!("cannot make a certificate: {e}")))?;
    let parts = [
        ctx.heap.alloc_string(made.cert.pem().as_bytes())?,
        ctx.heap.alloc_string(made.signing_key.serialize_pem().as_bytes())?,
    ];
    Ok(alloc_list(ctx, &parts)?)
}

pub(super) fn tls_check_identity(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let cert = text_arg(ctx, 0);
    let key = text_arg(ctx, 1);
    platform::tls::server_config(&cert, &key).map_err(|e| invalid(e.message))?;
    Ok(Value::null())
}
